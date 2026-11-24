use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{bounded, unbounded, Sender};

use crate::cache::{current_process_rss_bytes, parse_worker_cap};
use crate::protocol::schema::{
    CancelRequestParams, CloseBufferParams, DebugDumpParams, EngineCapabilities, ExpandParams,
    InitializeParams, InitializeResult, LspResolveBatchParams, LspResolveBatchResult, RpcError,
    RpcId, RpcNotification, RpcRequest, RpcResponse, SliceParams, StatusResult, SyncBufferParams,
    PROTOCOL_VERSION, SCHEMA_VERSION,
};
use crate::resolver::ewma::EwmaTracker;
use crate::resolver::lsp_provider::{
    NullLspProvider, ReverseRpcError, ReverseRpcSender, StdioLspProvider,
};
use crate::slicer::{Phase2Context, SlicerEngine};

pub struct QueryCancelGuard {
    pub query_id: u64,
    pub state: Arc<TransportState>,
}

impl Drop for QueryCancelGuard {
    fn drop(&mut self) {
        self.state.finish_query(self.query_id);
    }
}

pub struct Phase2Job {
    pub ctx: Phase2Context,
    pub cancel: Arc<AtomicBool>,
    pub query_id: u64,
    pub trace_id: String,
    pub state: Arc<TransportState>,
    pub _guard: QueryCancelGuard,
}

pub struct TransportState {
    pub engine: Arc<SlicerEngine>,
    pub log_path: Option<String>,
    pub out_tx: Sender<String>,
    pub next_engine_id: AtomicU64,
    pub next_trace_id: AtomicU64,
    pub pending_reverse: Mutex<HashMap<RpcId, (u64, oneshot::Sender<RpcResponse>)>>,
    pub active_cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    pub phase2_tx: Mutex<Option<Sender<Phase2Job>>>,
    pub shutdown_flag: AtomicBool,
}

impl TransportState {
    pub fn new(
        engine: Arc<SlicerEngine>,
        log_path: Option<String>,
        out_tx: Sender<String>,
    ) -> Arc<Self> {
        Self::with_phase2_tx(engine, log_path, out_tx, None)
    }

    pub fn with_phase2_tx(
        engine: Arc<SlicerEngine>,
        log_path: Option<String>,
        out_tx: Sender<String>,
        phase2_tx: Option<Sender<Phase2Job>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            engine,
            log_path,
            out_tx,
            next_engine_id: AtomicU64::new(1),
            next_trace_id: AtomicU64::new(1),
            pending_reverse: Mutex::new(HashMap::new()),
            active_cancels: Mutex::new(HashMap::new()),
            phase2_tx: Mutex::new(phase2_tx),
            shutdown_flag: AtomicBool::new(false),
        })
    }

    pub fn close_phase2_pool(&self) {
        let mut guard = self.phase2_tx.lock().expect("phase2_tx lock");
        *guard = None;
    }

    pub fn send_json<T: serde::Serialize>(&self, msg: &T) {
        if let Ok(line) = serde_json::to_string(msg) {
            let _ = self.out_tx.send(line);
        }
    }

    pub fn make_reverse_rpc_sender(self: &Arc<Self>) -> ReverseRpcSender {
        let state = Arc::clone(self);
        Arc::new(move |params: LspResolveBatchParams| {
            if state.shutdown_flag.load(Ordering::Relaxed) {
                return Err(ReverseRpcError::Cancelled);
            }
            let n = state.next_engine_id.fetch_add(1, Ordering::Relaxed);
            let id = RpcId::Str(format!("e-{n}"));
            let (tx, rx) = oneshot::channel();
            {
                let mut map = state.pending_reverse.lock().expect("pending_reverse lock");
                map.insert(id.clone(), (params.query_id, tx));
            }
            let payload =
                serde_json::to_value(&params).map_err(|e| ReverseRpcError::Other(e.to_string()))?;
            let req = RpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(id.clone()),
                method: "lineage/lspResolveBatch".to_string(),
                params: payload,
            };
            state.send_json(&req);

            let wait_ms = params.timeout_ms.saturating_add(50);
            match rx.recv_timeout(Duration::from_millis(wait_ms)) {
                Ok(resp) => {
                    if let Some(err) = resp.error {
                        Err(ReverseRpcError::Other(err.message))
                    } else if let Some(res) = resp.result {
                        serde_json::from_value::<LspResolveBatchResult>(res)
                            .map_err(|e| ReverseRpcError::Other(e.to_string()))
                    } else {
                        Err(ReverseRpcError::Other(
                            "Empty reverse-RPC response".to_string(),
                        ))
                    }
                }
                Err(oneshot::RecvTimeoutError::Disconnected) => Err(ReverseRpcError::Cancelled),
                Err(oneshot::RecvTimeoutError::Timeout) => {
                    let mut map = state.pending_reverse.lock().expect("pending_reverse lock");
                    map.remove(&id);
                    Err(ReverseRpcError::Timeout)
                }
            }
        })
    }

    pub fn register_cancel(&self, query_id: u64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let mut map = self.active_cancels.lock().expect("cancels lock");
        map.insert(query_id, flag.clone());
        flag
    }

    pub fn cancel_query(&self, query_id: u64) {
        {
            let map = self.active_cancels.lock().expect("cancels lock");
            if let Some(flag) = map.get(&query_id) {
                flag.store(true, Ordering::Relaxed);
            }
        }
        let mut pending = self.pending_reverse.lock().expect("pending_reverse lock");
        pending.retain(|_, (qid, _)| *qid != query_id);
    }

    pub fn cancel_all_and_drain(&self) {
        {
            let map = self.active_cancels.lock().expect("cancels lock");
            for flag in map.values() {
                flag.store(true, Ordering::Relaxed);
            }
        }
        let mut pending = self.pending_reverse.lock().expect("pending_reverse lock");
        pending.clear();
    }

    pub fn finish_query(&self, query_id: u64) {
        let mut map = self.active_cancels.lock().expect("cancels lock");
        map.remove(&query_id);
    }

    pub fn next_trace_id(&self) -> String {
        let n = self.next_trace_id.fetch_add(1, Ordering::Relaxed);
        format!("tr-{n}")
    }
}

pub fn serve_stdio(engine: Arc<SlicerEngine>, log_path: Option<String>) {
    let (out_tx, out_rx) = unbounded::<String>();
    let (phase2_tx, phase2_rx) = bounded::<Phase2Job>(64);
    let state = TransportState::with_phase2_tx(engine.clone(), log_path, out_tx, Some(phase2_tx));
    let reverse_sender = state.make_reverse_rpc_sender();
    let ewma = Arc::new(EwmaTracker::new());
    let workspace_root = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    engine.set_resolver(Arc::new(StdioLspProvider::new(
        reverse_sender,
        ewma,
        workspace_root,
        "neovim-lsp".to_string(),
    )));

    // Dedicated stdout writer thread (stdout isolation: NDJSON frames only).
    let writer_handle = thread::spawn(move || {
        let stdout = io::stdout();
        let mut handle = io::BufWriter::new(stdout.lock());
        while let Ok(line) = out_rx.recv() {
            if writeln!(handle, "{line}").is_err() {
                break;
            }
            if handle.flush().is_err() {
                break;
            }
        }
    });

    // Dedicated bounded Phase 2 worker pool (2-4 threads)
    let phase2_count = parse_worker_cap().clamp(2, 4);
    let mut phase2_workers = Vec::with_capacity(phase2_count);
    for _ in 0..phase2_count {
        let rx = phase2_rx.clone();
        phase2_workers.push(thread::spawn(move || {
            while let Ok(job) = rx.recv() {
                if !job.cancel.load(Ordering::Relaxed)
                    && !job.state.shutdown_flag.load(Ordering::Relaxed)
                {
                    if let Some(delta_env) = job.state.engine.execute_phase2(job.ctx, &job.cancel) {
                        if !job.cancel.load(Ordering::Relaxed) {
                            let notif = RpcNotification::new(
                                "lineage/sliceDelta",
                                serde_json::json!({
                                    "query_id": job.query_id,
                                    "trace_id": job.trace_id,
                                    "envelope": delta_env,
                                }),
                            );
                            job.state.send_json(&notif);
                        }
                    }
                }
            }
        }));
    }

    // Phase 1 worker pool
    let worker_count = parse_worker_cap();
    let (job_tx, job_rx) = bounded::<RpcRequest>(64);
    let mut workers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let rx = job_rx.clone();
        let st = Arc::clone(&state);
        workers.push(thread::spawn(move || {
            while let Ok(req) = rx.recv() {
                handle_request(&st, req);
            }
        }));
    }

    // Dedicated stdin reader thread (runs on current thread)
    let stdin = io::stdin();
    let reader = io::BufReader::new(stdin.lock());
    for line_res in reader.lines() {
        let Ok(line) = line_res else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let val = match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "ignoring malformed JSON-RPC frame");
                continue;
            }
        };

        // Check if this is a response to an engine reverse-RPC ("e-<n>")
        if val.get("method").is_none() && val.get("id").is_some() {
            if let Ok(resp) = serde_json::from_value::<RpcResponse>(val) {
                let mut pending = state.pending_reverse.lock().expect("pending_reverse lock");
                if let Some((_qid, tx)) = pending.remove(&resp.id) {
                    let _ = tx.send(resp);
                }
            }
            continue;
        }

        if let Ok(req) = serde_json::from_value::<RpcRequest>(val) {
            if req.method == "$/cancelRequest" {
                if let Ok(params) = serde_json::from_value::<CancelRequestParams>(req.params) {
                    if let Some(qid) = params.query_id {
                        state.cancel_query(qid);
                    }
                }
                continue;
            }
            if req.method == "shutdown" {
                state.shutdown_flag.store(true, Ordering::Relaxed);
                state.cancel_all_and_drain();
                if let Some(id) = req.id {
                    state.send_json(&RpcResponse::ok(id, serde_json::json!(null)));
                }
                break;
            }
            let _ = job_tx.send(req);
        }
    }

    drop(job_tx);
    for w in workers {
        let _ = w.join();
    }
    state.close_phase2_pool();
    for w in phase2_workers {
        let _ = w.join();
    }
    state.engine.set_resolver(Arc::new(NullLspProvider));
    drop(state);
    let _ = writer_handle.join();
}

pub fn handle_request(state: &Arc<TransportState>, req: RpcRequest) {
    let trace_id = state.next_trace_id();
    let Some(id) = req.id.clone() else {
        // Notification
        match req.method.as_str() {
            "lineage/syncBuffer" => {
                if let Ok(p) = serde_json::from_value::<SyncBufferParams>(req.params) {
                    let _ = state
                        .engine
                        .dirty_buffers
                        .upsert(p.uri, p.changedtick, p.text);
                }
            }
            "lineage/closeBuffer" => {
                if let Ok(p) = serde_json::from_value::<CloseBufferParams>(req.params) {
                    state.engine.dirty_buffers.remove(&p.uri);
                }
            }
            _ => {}
        }
        return;
    };

    let result: Result<serde_json::Value, RpcError> = match req.method.as_str() {
        "initialize" => {
            let params: InitializeParams =
                serde_json::from_value(req.params).unwrap_or(InitializeParams {
                    client_name: None,
                    workspace_root: None,
                    protocol_version: None,
                });
            if let Some(v) = params.protocol_version {
                if v != PROTOCOL_VERSION {
                    Err(RpcError::invalid_params(format!(
                        "Protocol version mismatch: expected {PROTOCOL_VERSION}, got {v}"
                    )))
                } else {
                    Ok(serde_json::to_value(build_init_result(state)).unwrap())
                }
            } else {
                Ok(serde_json::to_value(build_init_result(state)).unwrap())
            }
        }
        "lineage/syncBuffer" => match serde_json::from_value::<SyncBufferParams>(req.params) {
            Ok(p) => match state
                .engine
                .dirty_buffers
                .upsert(p.uri, p.changedtick, p.text)
            {
                Ok(()) => Ok(serde_json::json!({ "ok": true })),
                Err(e) => Err(RpcError::invalid_params(format!("{e:?}"))),
            },
            Err(e) => Err(RpcError::invalid_params(e.to_string())),
        },
        "lineage/closeBuffer" => match serde_json::from_value::<CloseBufferParams>(req.params) {
            Ok(p) => {
                state.engine.dirty_buffers.remove(&p.uri);
                Ok(serde_json::json!({ "ok": true }))
            }
            Err(e) => Err(RpcError::invalid_params(e.to_string())),
        },
        "lineage/slice" => match serde_json::from_value::<SliceParams>(req.params) {
            Ok(p) => {
                let qid = p.query_id.unwrap_or(1);
                let cancel = state.register_cancel(qid);
                match state.engine.slice_phase1(&p, &trace_id, &cancel) {
                    Ok((env, phase2_opt)) => {
                        state.send_json(&RpcResponse::ok(id, serde_json::to_value(&env).unwrap()));
                        if let Some(ctx) = phase2_opt {
                            let job = Phase2Job {
                                ctx,
                                cancel,
                                query_id: qid,
                                trace_id: trace_id.clone(),
                                state: Arc::clone(state),
                                _guard: QueryCancelGuard {
                                    query_id: qid,
                                    state: Arc::clone(state),
                                },
                            };
                            let guard = state.phase2_tx.lock().expect("phase2_tx lock");
                            if let Some(ref tx) = *guard {
                                let _ = tx.try_send(job);
                            }
                        } else {
                            state.finish_query(qid);
                        }
                        return;
                    }
                    Err(err) => {
                        state.finish_query(qid);
                        Err(err)
                    }
                }
            }
            Err(e) => Err(RpcError::invalid_params(e.to_string())),
        },
        "lineage/status" => {
            let (dirty_buffers, dirty_bytes) = state.engine.dirty_buffers.stats();
            let resolver = state.engine.resolver();
            let ewma_clients = resolver
                .ewma_tracker()
                .map(|t| t.snapshots())
                .unwrap_or_default();
            let loaded_grammars: Vec<String> = state
                .engine
                .adapters
                .iter()
                .map(|a| a.language_id().to_string())
                .collect();
            let status = StatusResult {
                protocol_version: PROTOCOL_VERSION,
                schema_version: SCHEMA_VERSION,
                engine_version: env!("CARGO_PKG_VERSION").to_string(),
                cache_bytes: state.engine.cache.approx_weighted_bytes(),
                cache_cap_bytes: state.engine.cache.cap_bytes,
                dirty_buffers,
                dirty_bytes,
                loaded_grammars,
                ewma_clients,
                log_path: state.log_path.clone(),
                rss_bytes: current_process_rss_bytes(),
            };
            Ok(serde_json::to_value(status).unwrap())
        }
        "lineage/debugDump" => match serde_json::from_value::<DebugDumpParams>(req.params) {
            Ok(p) => match state.engine.dump_ir_text(&p.uri) {
                Ok(ir_text) => Ok(serde_json::json!({ "uri": p.uri, "ir": ir_text })),
                Err(e) => Err(RpcError::internal(e)),
            },
            Err(e) => Err(RpcError::invalid_params(e.to_string())),
        },
        "lineage/expand" => match serde_json::from_value::<ExpandParams>(req.params) {
            Ok(p) => {
                let cancel = Arc::new(AtomicBool::new(false));
                state
                    .engine
                    .expand_node(&p, &trace_id, &cancel)
                    .map(|res| serde_json::to_value(res).unwrap())
            }
            Err(e) => Err(RpcError::invalid_params(e.to_string())),
        },
        other => Err(RpcError::method_not_found(other)),
    };

    match result {
        Ok(val) => state.send_json(&RpcResponse::ok(id, val)),
        Err(err) => state.send_json(&RpcResponse::err(id, err)),
    }
}

fn build_init_result(state: &TransportState) -> InitializeResult {
    InitializeResult {
        protocol_version: PROTOCOL_VERSION,
        schema_version: SCHEMA_VERSION,
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: EngineCapabilities {
            languages: state
                .engine
                .adapters
                .iter()
                .map(|a| a.language_id().to_string())
                .collect(),
            reverse_rpc: true,
            delta_notifications: true,
        },
    }
}
