use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Sender};

use crate::cache::{current_process_rss_bytes, parse_worker_cap};
use crate::export::{render_export, ExportOptions};
use crate::protocol::schema::{
    CancelRequestParams, CloseBufferParams, DebugDumpParams, EdgeSide, EngineCapabilities,
    ExpandParams, ExpandResult, ExportParams, ExportSourceMode, FileEntry, FlowEdge, FlowNode,
    InitializeParams, InitializeResult, LspResolveBatchParams, LspResolveBatchResult, NodeKind,
    Phase2Status, RpcError, RpcId, RpcNotification, RpcRequest, RpcResponse, SliceDirection,
    SliceEnvelope, SliceGraph, SliceParams, StatusResult, SyncBufferParams, PROTOCOL_VERSION,
    SCHEMA_VERSION,
};
use crate::resolver::ewma::EwmaTracker;
use crate::resolver::lsp_provider::{
    NullLspProvider, ReverseRpcError, ReverseRpcSender, StdioLspProvider,
};
use crate::slicer::{Phase2Context, SlicerEngine};

pub const SNAPSHOT_LRU_CAPACITY: usize = 4;
pub const DEFAULT_EXPORT_LSP_TIMEOUT_MS: u64 = 500;

pub struct QueryCancelGuard {
    pub query_id: u64,
    pub state: Arc<TransportState>,
}

impl Drop for QueryCancelGuard {
    fn drop(&mut self) {
        self.state.finish_query(self.query_id);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotPhase2State {
    InFlight,
    Settled(Phase2Status),
}

#[derive(Debug, Clone)]
pub struct MaterializedSliceSnapshot {
    pub envelope: SliceEnvelope,
    pub workspace_root: Option<String>,
    pub lsp_attached: bool,
    pub phase2_state: SnapshotPhase2State,
    pub expand_seq: u32,
}

#[derive(Default)]
pub struct QuerySnapshotCache {
    entries: HashMap<u64, MaterializedSliceSnapshot>,
    lru_order: VecDeque<u64>,
}

impl QuerySnapshotCache {
    pub fn insert(&mut self, query_id: u64, snapshot: MaterializedSliceSnapshot) {
        if self.entries.contains_key(&query_id) {
            self.lru_order.retain(|&id| id != query_id);
        }
        while self.entries.len() >= SNAPSHOT_LRU_CAPACITY {
            if let Some(oldest) = self.lru_order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }
        self.entries.insert(query_id, snapshot);
        self.lru_order.push_back(query_id);
    }

    pub fn get_cloned(&mut self, query_id: u64) -> Option<MaterializedSliceSnapshot> {
        let snap = self.entries.get(&query_id)?.clone();
        self.lru_order.retain(|&id| id != query_id);
        self.lru_order.push_back(query_id);
        Some(snap)
    }

    pub fn update_phase2(
        &mut self,
        query_id: u64,
        delta_env: &SliceEnvelope,
        full_graph: SliceGraph,
        status: Phase2Status,
    ) {
        let Some(snap) = self.entries.get_mut(&query_id) else {
            return;
        };
        if snap.expand_seq == 0 {
            let mut updated = delta_env.clone();
            updated.graph = Some(full_graph);
            snap.envelope = updated;
        } else {
            // Preserve any lazily expanded exp{k}_ subtrees & files merged prior to Phase 2 finish.
            let prev_graph = snap.envelope.graph.take().unwrap_or(SliceGraph {
                nodes: Vec::new(),
                edges: Vec::new(),
            });
            let mut merged_files = delta_env.files.clone();
            let mut uri_to_fid: HashMap<String, u32> = merged_files
                .iter()
                .map(|f| (f.uri.clone(), f.file_id))
                .collect();
            let mut old_fid_to_new: HashMap<u32, u32> = HashMap::new();
            for old_f in &snap.envelope.files {
                if let Some(&fid) = uri_to_fid.get(&old_f.uri) {
                    old_fid_to_new.insert(old_f.file_id, fid);
                } else {
                    let fid = merged_files.len() as u32;
                    merged_files.push(FileEntry {
                        file_id: fid,
                        uri: old_f.uri.clone(),
                    });
                    uri_to_fid.insert(old_f.uri.clone(), fid);
                    old_fid_to_new.insert(old_f.file_id, fid);
                }
            }
            let mut merged_graph = full_graph;
            for mut n in prev_graph.nodes {
                if n.id.starts_with("exp") {
                    if let Some(&nfid) = old_fid_to_new.get(&n.file_id) {
                        n.file_id = nfid;
                    }
                    merged_graph.nodes.push(n);
                }
            }
            for e in prev_graph.edges {
                if e.from.starts_with("exp") || e.to.starts_with("exp") {
                    merged_graph.edges.push(e);
                }
            }
            let mut updated = delta_env.clone();
            updated.files = merged_files;
            updated.graph = Some(merged_graph);
            snap.envelope = updated;
        }
        snap.phase2_state = SnapshotPhase2State::Settled(status);
    }

    pub fn merge_expansion(
        &mut self,
        parent_query_id: u64,
        parent_node_id: Option<&str>,
        exp_res: &ExpandResult,
        exp_graph: &SliceGraph,
        exp_roots: &[String],
    ) {
        let Some(snap) = self.entries.get_mut(&parent_query_id) else {
            return;
        };
        snap.expand_seq = snap.expand_seq.saturating_add(1);
        let k = snap.expand_seq;

        let mut uri_to_fid: HashMap<String, u32> = snap
            .envelope
            .files
            .iter()
            .map(|f| (f.uri.clone(), f.file_id))
            .collect();
        let mut exp_fid_map: HashMap<u32, u32> = HashMap::new();
        for f in &exp_res.files {
            if let Some(&existing) = uri_to_fid.get(&f.uri) {
                exp_fid_map.insert(f.file_id, existing);
            } else {
                let fid = snap.envelope.files.len() as u32;
                snap.envelope.files.push(FileEntry {
                    file_id: fid,
                    uri: f.uri.clone(),
                });
                uri_to_fid.insert(f.uri.clone(), fid);
                exp_fid_map.insert(f.file_id, fid);
            }
        }

        let graph = snap.envelope.graph.get_or_insert_with(|| SliceGraph {
            nodes: Vec::new(),
            edges: Vec::new(),
        });

        let mut parent_is_downstream = false;
        if let Some(pid) = parent_node_id {
            for node in &mut graph.nodes {
                if node.id == pid
                    && matches!(node.kind, NodeKind::FrontierNode | NodeKind::ClusterNode)
                {
                    node.kind = NodeKind::SourceNode;
                }
            }
            parent_is_downstream = graph.edges.iter().any(|e| {
                (e.from == pid || e.to == pid) && e.edge_side == Some(EdgeSide::Downstream)
            });
        }

        let prefix = format!("exp{k}_");
        for n in &exp_graph.nodes {
            let mapped_fid = exp_fid_map.get(&n.file_id).copied().unwrap_or(n.file_id);
            graph.nodes.push(FlowNode {
                id: format!("{prefix}{}", n.id),
                kind: n.kind,
                file_id: mapped_fid,
                range: n.range,
                label: n.label.clone(),
                snippet: n.snippet.clone(),
                absence: n.absence.clone(),
                provenance: n.provenance.clone(),
                is_guard: n.is_guard,
                guard_text: n.guard_text.clone(),
                is_danger_sink: n.is_danger_sink,
                danger_kind: n.danger_kind.clone(),
            });
        }

        for e in &exp_graph.edges {
            graph.edges.push(FlowEdge {
                from: format!("{prefix}{}", e.from),
                to: format!("{prefix}{}", e.to),
                label: e.label.clone(),
                is_back_edge: e.is_back_edge,
                cycle_group_id: e.cycle_group_id,
                provenance: e.provenance.clone(),
                edge_side: e.edge_side,
            });
        }

        if let Some(pid) = parent_node_id {
            let side = match snap.envelope.direction {
                SliceDirection::To => EdgeSide::Upstream,
                SliceDirection::From => EdgeSide::Downstream,
                SliceDirection::Both => {
                    if parent_is_downstream {
                        EdgeSide::Downstream
                    } else {
                        EdgeSide::Upstream
                    }
                }
            };
            for r in exp_roots {
                graph.edges.push(FlowEdge {
                    from: pid.to_string(),
                    to: format!("{prefix}{r}"),
                    label: Some("expanded".to_string()),
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "expand_node".to_string(),
                    edge_side: Some(side),
                });
            }
        }

        for reason in &exp_res.truncation {
            if !snap.envelope.truncation.contains(reason) {
                snap.envelope.truncation.push(*reason);
            }
        }
    }
}

pub struct ExportPhase2Job {
    pub id: RpcId,
    pub params: ExportParams,
    pub ctx: Phase2Context,
    pub fallback_envelope: SliceEnvelope,
    pub fallback_graph: SliceGraph,
    pub cancel: Arc<AtomicBool>,
    pub query_id: u64,
    pub trace_id: String,
    pub state: Arc<TransportState>,
    pub _guard: QueryCancelGuard,
}

pub struct SliceDeltaJob {
    pub ctx: Phase2Context,
    pub cancel: Arc<AtomicBool>,
    pub query_id: u64,
    pub trace_id: String,
    pub state: Arc<TransportState>,
    pub _guard: QueryCancelGuard,
}

pub enum Phase2Job {
    SliceDelta(Box<SliceDeltaJob>),
    Export(Box<ExportPhase2Job>),
}

pub struct TransportState {
    pub engine: Arc<SlicerEngine>,
    pub log_path: Option<String>,
    pub out_tx: Sender<String>,
    pub next_engine_id: AtomicU64,
    pub next_trace_id: AtomicU64,
    pub next_export_qid: AtomicU64,
    pub pending_reverse: Mutex<HashMap<RpcId, (u64, oneshot::Sender<RpcResponse>)>>,
    pub active_cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    pub phase2_tx: Mutex<Option<Sender<Phase2Job>>>,
    pub snapshot_cache: Mutex<QuerySnapshotCache>,
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
            next_export_qid: AtomicU64::new(100_000),
            pending_reverse: Mutex::new(HashMap::new()),
            active_cancels: Mutex::new(HashMap::new()),
            phase2_tx: Mutex::new(phase2_tx),
            snapshot_cache: Mutex::new(QuerySnapshotCache::default()),
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
                match job {
                    Phase2Job::SliceDelta(delta_job) => {
                        let SliceDeltaJob {
                            ctx,
                            cancel,
                            query_id,
                            trace_id,
                            state,
                            _guard,
                        } = *delta_job;
                        if !cancel.load(Ordering::Relaxed)
                            && !state.shutdown_flag.load(Ordering::Relaxed)
                        {
                            if let Some((delta_env, full_graph, status)) =
                                state.engine.execute_phase2_with_status(ctx, &cancel)
                            {
                                {
                                    let mut cache =
                                        state.snapshot_cache.lock().expect("snapshot_cache lock");
                                    cache.update_phase2(query_id, &delta_env, full_graph, status);
                                }
                                if !cancel.load(Ordering::Relaxed) {
                                    let notif = RpcNotification::new(
                                        "lineage/sliceDelta",
                                        serde_json::json!({
                                            "query_id": query_id,
                                            "trace_id": trace_id,
                                            "envelope": delta_env,
                                        }),
                                    );
                                    state.send_json(&notif);
                                }
                            }
                        }
                    }
                    Phase2Job::Export(exp_job) => {
                        let ExportPhase2Job {
                            id,
                            params,
                            ctx,
                            mut fallback_envelope,
                            fallback_graph,
                            cancel,
                            query_id,
                            trace_id,
                            state,
                            _guard,
                        } = *exp_job;
                        if cancel.load(Ordering::Relaxed)
                            || state.shutdown_flag.load(Ordering::Relaxed)
                        {
                            state.send_json(&RpcResponse::err(id, RpcError::cancelled()));
                            continue;
                        }
                        let wait_start = Instant::now();
                        let timeout_ms = params
                            .lsp_timeout_ms
                            .unwrap_or(DEFAULT_EXPORT_LSP_TIMEOUT_MS);
                        if let Some((mut delta_env, full_graph, status)) =
                            state.engine.execute_phase2_with_status(ctx, &cancel)
                        {
                            let wait_ms = wait_start.elapsed().as_millis() as u64;
                            {
                                let mut cache =
                                    state.snapshot_cache.lock().expect("snapshot_cache lock");
                                cache.update_phase2(
                                    query_id,
                                    &delta_env,
                                    full_graph.clone(),
                                    status,
                                );
                            }
                            delta_env.graph = Some(full_graph);
                            let opts = ExportOptions::for_format(
                                params.format,
                                params.workspace_root.as_deref().map(PathBuf::from),
                                params.null_only,
                                params.include_snippets,
                                false, // RPC never exposes absolute paths
                                params.lsp_attached,
                                timeout_ms,
                                status,
                                ExportSourceMode::FreshSlice,
                                wait_ms,
                                None,
                            );
                            let out = render_export(&delta_env, &opts, &trace_id);
                            state.send_json(&RpcResponse::ok(
                                id,
                                serde_json::to_value(&out.result).unwrap(),
                            ));
                        } else if cancel.load(Ordering::Relaxed) {
                            state.send_json(&RpcResponse::err(id, RpcError::cancelled()));
                        } else {
                            let wait_ms = wait_start.elapsed().as_millis() as u64;
                            fallback_envelope.graph = Some(fallback_graph);
                            let opts = ExportOptions::for_format(
                                params.format,
                                params.workspace_root.as_deref().map(PathBuf::from),
                                params.null_only,
                                params.include_snippets,
                                false,
                                params.lsp_attached,
                                timeout_ms,
                                Phase2Status::TimedOut,
                                ExportSourceMode::FreshSlice,
                                wait_ms,
                                None,
                            );
                            let out = render_export(&fallback_envelope, &opts, &trace_id);
                            state.send_json(&RpcResponse::ok(
                                id,
                                serde_json::to_value(&out.result).unwrap(),
                            ));
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
                match state
                    .engine
                    .slice_phase1_with_snapshot(&p, &trace_id, &cancel)
                {
                    Ok((env, snapshot_graph, phase2_opt)) => {
                        let mut snap_env = env.clone();
                        snap_env.graph = Some(snapshot_graph);
                        let p2_state = if phase2_opt.is_some() {
                            SnapshotPhase2State::InFlight
                        } else {
                            SnapshotPhase2State::Settled(Phase2Status::NotNeeded)
                        };
                        {
                            let mut cache =
                                state.snapshot_cache.lock().expect("snapshot_cache lock");
                            cache.insert(
                                qid,
                                MaterializedSliceSnapshot {
                                    envelope: snap_env,
                                    workspace_root: p.workspace_root.clone(),
                                    lsp_attached: p.lsp_attached,
                                    phase2_state: p2_state,
                                    expand_seq: 0,
                                },
                            );
                        }

                        state.send_json(&RpcResponse::ok(id, serde_json::to_value(&env).unwrap()));
                        if let Some(ctx) = phase2_opt {
                            let job = Phase2Job::SliceDelta(Box::new(SliceDeltaJob {
                                ctx,
                                cancel,
                                query_id: qid,
                                trace_id: trace_id.clone(),
                                state: Arc::clone(state),
                                _guard: QueryCancelGuard {
                                    query_id: qid,
                                    state: Arc::clone(state),
                                },
                            }));
                            let guard = state.phase2_tx.lock().expect("phase2_tx lock");
                            if let Some(ref tx) = *guard {
                                if tx.try_send(job).is_err() {
                                    tracing::warn!(
                                        reason = "phase2_queue_full",
                                        query_id = qid,
                                        trace_id = %trace_id,
                                        "phase2 queue full for lineage/slice"
                                    );
                                }
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
        "lineage/export" => match serde_json::from_value::<ExportParams>(req.params) {
            Ok(p) => {
                handle_export_request(state, id.clone(), p, &trace_id);
                return;
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
                    .expand_node_with_graph(&p, &trace_id, &cancel)
                    .map(|(res, exp_graph, exp_roots)| {
                        if let Some(parent_qid) = p.parent_query_id {
                            let mut cache =
                                state.snapshot_cache.lock().expect("snapshot_cache lock");
                            cache.merge_expansion(
                                parent_qid,
                                p.parent_node_id.as_deref(),
                                &res,
                                &exp_graph,
                                &exp_roots,
                            );
                        }
                        serde_json::to_value(res).unwrap()
                    })
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

fn handle_export_request(
    state: &Arc<TransportState>,
    id: RpcId,
    params: ExportParams,
    trace_id: &str,
) {
    let timeout_ms = params
        .lsp_timeout_ms
        .unwrap_or(DEFAULT_EXPORT_LSP_TIMEOUT_MS);

    if let Some(qid) = params.query_id {
        let cached = {
            let mut cache = state.snapshot_cache.lock().expect("snapshot_cache lock");
            cache.get_cloned(qid)
        };
        if let Some(snap) = cached {
            let ws_root = params
                .workspace_root
                .as_deref()
                .or(snap.workspace_root.as_deref())
                .map(PathBuf::from);
            let (p2_status, static_reason) = match snap.phase2_state {
                SnapshotPhase2State::Settled(status) => {
                    if params.no_lsp && status == Phase2Status::NotNeeded {
                        (Phase2Status::Skipped, None)
                    } else {
                        (status, None)
                    }
                }
                SnapshotPhase2State::InFlight => {
                    if params.no_lsp {
                        (Phase2Status::Skipped, None)
                    } else {
                        (
                            Phase2Status::TimedOut,
                            Some("Static only (LSP still settling)".to_string()),
                        )
                    }
                }
            };
            let opts = ExportOptions::for_format(
                params.format,
                ws_root,
                params.null_only,
                params.include_snippets,
                false, // RPC never allows absolute_paths
                snap.lsp_attached,
                timeout_ms,
                p2_status,
                ExportSourceMode::LiveSnapshot,
                0,
                static_reason,
            );
            let out = render_export(&snap.envelope, &opts, trace_id);
            state.send_json(&RpcResponse::ok(
                id,
                serde_json::to_value(&out.result).unwrap(),
            ));
            return;
        }
    }

    let Some(uri) = params.uri.clone() else {
        state.send_json(&RpcResponse::err(
            id,
            RpcError::invalid_params("lineage/export requires cached query_id or (uri, position)"),
        ));
        return;
    };
    let Some(position) = params.position else {
        state.send_json(&RpcResponse::err(
            id,
            RpcError::invalid_params("lineage/export requires position when query_id is absent"),
        ));
        return;
    };

    let qid = params
        .query_id
        .unwrap_or_else(|| state.next_export_qid.fetch_add(1, Ordering::Relaxed));
    let cancel = state.register_cancel(qid);
    let slice_params = SliceParams {
        uri,
        position,
        offset_encoding: params.offset_encoding,
        direction: params.direction,
        include_graph: false,
        include_focus: false,
        include_tree: false,
        include_snippets: params.include_snippets.unwrap_or(false),
        exclude_tests: params.exclude_tests,
        query_id: Some(qid),
        lsp_attached: params.lsp_attached && !params.no_lsp,
        lsp_client_name: params.lsp_client_name.clone(),
        workspace_root: params.workspace_root.clone(),
        lsp_timeout_ms: Some(timeout_ms),
    };

    match state
        .engine
        .slice_phase1_with_snapshot(&slice_params, trace_id, &cancel)
    {
        Ok((mut env, snapshot_graph, phase2_opt)) => {
            let mut snap_env = env.clone();
            snap_env.graph = Some(snapshot_graph.clone());
            let p2_state = if phase2_opt.is_some() && !params.no_lsp {
                SnapshotPhase2State::InFlight
            } else {
                SnapshotPhase2State::Settled(if params.no_lsp {
                    Phase2Status::Skipped
                } else {
                    Phase2Status::NotNeeded
                })
            };
            {
                let mut cache = state.snapshot_cache.lock().expect("snapshot_cache lock");
                cache.insert(
                    qid,
                    MaterializedSliceSnapshot {
                        envelope: snap_env,
                        workspace_root: params.workspace_root.clone(),
                        lsp_attached: slice_params.lsp_attached,
                        phase2_state: p2_state,
                        expand_seq: 0,
                    },
                );
            }

            let Some(mut ctx) = phase2_opt.filter(|_| !params.no_lsp) else {
                state.finish_query(qid);
                env.graph = Some(snapshot_graph);
                let p2_status = if params.no_lsp {
                    Phase2Status::Skipped
                } else {
                    Phase2Status::NotNeeded
                };
                let opts = ExportOptions::for_format(
                    params.format,
                    params.workspace_root.as_deref().map(PathBuf::from),
                    params.null_only,
                    params.include_snippets,
                    false,
                    slice_params.lsp_attached,
                    timeout_ms,
                    p2_status,
                    ExportSourceMode::FreshSlice,
                    0,
                    None,
                );
                let out = render_export(&env, &opts, trace_id);
                state.send_json(&RpcResponse::ok(
                    id,
                    serde_json::to_value(&out.result).unwrap(),
                ));
                return;
            };

            ctx.phase2_ceiling_ms = timeout_ms;
            let job = Phase2Job::Export(Box::new(ExportPhase2Job {
                id: id.clone(),
                params: params.clone(),
                ctx,
                fallback_envelope: env.clone(),
                fallback_graph: snapshot_graph.clone(),
                cancel,
                query_id: qid,
                trace_id: trace_id.to_string(),
                state: Arc::clone(state),
                _guard: QueryCancelGuard {
                    query_id: qid,
                    state: Arc::clone(state),
                },
            }));
            let send_res = {
                let guard = state.phase2_tx.lock().expect("phase2_tx lock");
                match *guard {
                    Some(ref tx) => tx.try_send(job).map_err(|_| ()),
                    None => Err(()),
                }
            };
            if send_res.is_err() {
                tracing::warn!(
                    reason = "phase2_queue_full",
                    query_id = qid,
                    trace_id = %trace_id,
                    "phase2 queue full for lineage/export; falling back to Phase 1"
                );
                state.finish_query(qid);
                env.graph = Some(snapshot_graph);
                let opts = ExportOptions::for_format(
                    params.format,
                    params.workspace_root.as_deref().map(PathBuf::from),
                    params.null_only,
                    params.include_snippets,
                    false,
                    slice_params.lsp_attached,
                    timeout_ms,
                    Phase2Status::Skipped,
                    ExportSourceMode::FreshSlice,
                    0,
                    Some("Static only (LSP queue full)".to_string()),
                );
                let out = render_export(&env, &opts, trace_id);
                state.send_json(&RpcResponse::ok(
                    id,
                    serde_json::to_value(&out.result).unwrap(),
                ));
            }
        }
        Err(err) => {
            state.finish_query(qid);
            state.send_json(&RpcResponse::err(id, err));
        }
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
            export_formats: vec!["mermaid".to_string(), "dot".to_string(), "html".to_string()],
        },
    }
}
