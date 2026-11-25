pub mod pipeline;
pub mod projectors;
pub mod scc;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::adapters::{ImportResolution, LanguageAdapter};
use crate::cache::{
    check_text_guards, path_to_uri, read_file_guarded, uri_to_path, DirtyBufferOverlay, DiskStamp,
    FileIrCache, GuardError,
};
use crate::ir::{
    AbsenceValue, CallTarget, FileSkeleton, FunctionIr, GuardEnv, GuardFact, SsiInst, SsiOp,
    ValueId, STATE_NULL, STATE_PRESENT, STATE_UNDEFINED,
};
use crate::protocol::schema::{
    CanonicalPos, CanonicalRange, EdgeSide, ExpandParams, ExpandResult, FileEntry, FlowEdge,
    FlowNode, HoverItem, HoverRisk, LspPosition, LspResolveBatchParams, LspResolveItem,
    LspResolveKind, NodeKind, OffsetEncoding, Phase2Status, RpcError, SliceDirection,
    SliceEnvelope, SliceGraph, SliceParams, SliceProjections, SliceStats, TreeRow,
    TruncationReason, UnknownBoundary, UnknownReason, SCHEMA_VERSION,
};
use crate::resolver::discover_callers;
use crate::resolver::lsp_provider::{
    DEFAULT_LSP_TIMEOUT_MS, MAX_IMPLEMENTATION_TARGETS, PHASE2_CEILING_MS,
};
use crate::resolver::{ResolveContext, SymbolResolver};
use crate::slicer::pipeline::{run_reduction_pipeline, RawSliceEdge, RawSliceNode};
use crate::slicer::projectors::{project_focus_spans, project_hover_card, project_tree_rows};
use crate::text::PositionMapper;

pub struct SlicerEngine {
    pub cache: Arc<FileIrCache>,
    pub dirty_buffers: Arc<DirtyBufferOverlay>,
    resolver: RwLock<Arc<dyn SymbolResolver>>,
    pub adapters: Vec<Arc<dyn LanguageAdapter>>,
}

impl SlicerEngine {
    pub fn new(
        cache: Arc<FileIrCache>,
        dirty_buffers: Arc<DirtyBufferOverlay>,
        resolver: Arc<dyn SymbolResolver>,
        adapters: Vec<Arc<dyn LanguageAdapter>>,
    ) -> Self {
        Self {
            cache,
            dirty_buffers,
            resolver: RwLock::new(resolver),
            adapters,
        }
    }

    pub fn resolver(&self) -> Arc<dyn SymbolResolver> {
        self.resolver.read().expect("resolver read lock").clone()
    }

    pub fn set_resolver(&self, resolver: Arc<dyn SymbolResolver>) {
        *self.resolver.write().expect("resolver write lock") = resolver;
    }

    pub fn adapter_for_uri(&self, uri: &str) -> Option<&Arc<dyn LanguageAdapter>> {
        let path = uri_to_path(uri);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        self.adapters.iter().find(|a| a.supports_extension(ext))
    }

    pub fn load_source(&self, uri: &str) -> Result<(Arc<String>, Option<DiskStamp>), GuardError> {
        if let Some(dirty) = self.dirty_buffers.get(uri) {
            check_text_guards(&dirty.text)?;
            return Ok((dirty.text, None));
        }
        let path = uri_to_path(uri);
        let (text, stamp) = read_file_guarded(&path)?;
        Ok((Arc::new(text), Some(stamp)))
    }

    pub fn get_or_parse_skeleton(
        &self,
        uri: &str,
        adapter: &Arc<dyn LanguageAdapter>,
    ) -> Result<(Arc<FileSkeleton>, Arc<String>), GuardError> {
        let (text, stamp) = self.load_source(uri)?;
        if let Some(cached) = self.cache.get_skeleton(uri, stamp.as_ref()) {
            let content_hash = xxhash_rust::xxh3::xxh3_64(text.as_bytes());
            if cached.content_hash == content_hash {
                return Ok((cached, text));
            }
        }
        let skel = Arc::new(adapter.parse_skeleton(uri, &text));
        self.cache
            .insert_skeleton(uri.to_string(), skel.clone(), stamp);
        Ok((skel, text))
    }

    pub fn get_or_lower_function(
        &self,
        uri: &str,
        text: &str,
        file_skeleton: &FileSkeleton,
        skel: &crate::ir::FunctionSkeleton,
        adapter: &Arc<dyn LanguageAdapter>,
    ) -> Arc<FunctionIr> {
        if let Some(cached) = self
            .cache
            .get_function(uri, &skel.symbol_id, skel.content_hash)
        {
            return cached;
        }
        let ir = Arc::new(adapter.lower_function(uri, text, file_skeleton, skel));
        self.cache.insert_function(
            uri.to_string(),
            skel.symbol_id.clone(),
            skel.content_hash,
            ir.clone(),
        );
        ir
    }

    pub fn slice_query<F>(
        &self,
        params: &SliceParams,
        trace_id: &str,
        cancel: &AtomicBool,
        mut on_delta: Option<F>,
    ) -> Result<SliceEnvelope, RpcError>
    where
        F: FnMut(SliceEnvelope),
    {
        let (phase1_env, phase2_opt) = self.slice_phase1(params, trace_id, cancel)?;
        if let (Some(ctx), Some(ref mut emit_delta)) = (phase2_opt, &mut on_delta) {
            if let Some(delta) = self.execute_phase2(ctx, cancel) {
                emit_delta(delta);
            }
        }
        Ok(phase1_env)
    }

    pub fn slice_phase1(
        &self,
        params: &SliceParams,
        trace_id: &str,
        cancel: &AtomicBool,
    ) -> Result<(SliceEnvelope, Option<Phase2Context>), RpcError> {
        let (env, _graph, p2) = self.slice_phase1_with_snapshot(params, trace_id, cancel)?;
        Ok((env, p2))
    }

    pub fn slice_phase1_with_snapshot(
        &self,
        params: &SliceParams,
        trace_id: &str,
        cancel: &AtomicBool,
    ) -> Result<(SliceEnvelope, SliceGraph, Option<Phase2Context>), RpcError> {
        let start = Instant::now();
        let query_id = params.query_id.unwrap_or(1);

        let adapter = self.adapter_for_uri(&params.uri).ok_or_else(|| {
            RpcError::invalid_params(format!("Unsupported file type: {}", params.uri))
        })?;

        let (skeleton, text) = match self.get_or_parse_skeleton(&params.uri, adapter) {
            Ok(v) => v,
            Err(guard_err) => {
                return Ok((
                    self.guard_error_envelope(params, query_id, trace_id, guard_err, start),
                    SliceGraph {
                        nodes: Vec::new(),
                        edges: Vec::new(),
                    },
                    None,
                ));
            }
        };

        let mapper = PositionMapper::new(&text);
        let canonical_pos = mapper.lsp_to_canonical(params.position, params.offset_encoding);

        // Find enclosing function or <module>
        let fn_skel = skeleton
            .functions
            .iter()
            .filter(|f| {
                f.name != "<module>"
                    && f.range.start <= canonical_pos
                    && canonical_pos <= f.range.end
            })
            .min_by_key(|f| f.end_byte.saturating_sub(f.start_byte))
            .or_else(|| skeleton.functions.iter().find(|f| f.name == "<module>"))
            .or_else(|| skeleton.functions.first())
            .ok_or_else(|| RpcError::internal("No function skeleton found"))?;

        let fn_ir = self.get_or_lower_function(&params.uri, &text, &skeleton, fn_skel, adapter);

        let target_vid = fn_ir
            .lookup_at_pos(canonical_pos)
            .or_else(|| fn_ir.returns.first().copied())
            .unwrap_or(ValueId(0));

        let mut files = vec![FileEntry {
            file_id: 0,
            uri: params.uri.clone(),
        }];
        let mut file_ids: HashMap<String, u32> = HashMap::new();
        file_ids.insert(params.uri.clone(), 0);

        let mut eval = self.evaluate_function_ir(
            &params.uri,
            &skeleton,
            &text,
            &fn_ir,
            target_vid,
            adapter,
            &mut files,
            &mut file_ids,
            params.workspace_root.as_deref().map(Path::new),
            start,
            params.include_snippets,
        );

        let mut extra_truncation: Vec<TruncationReason> = Vec::new();
        if matches!(
            params.direction,
            SliceDirection::From | SliceDirection::Both
        ) && fn_ir.name != "<module>"
        {
            if params.lsp_attached {
                eval.pending_lsp_items.push(LspResolveItem {
                    key: fn_ir.name.clone(),
                    uri: params.uri.clone(),
                    position: params.position,
                    offset_encoding: params.offset_encoding,
                    kind: LspResolveKind::CallHierarchyIncoming,
                });
            } else {
                let target_path = uri_to_path(&params.uri);
                let ws_root = params
                    .workspace_root
                    .as_deref()
                    .map(Path::new)
                    .or_else(|| target_path.parent())
                    .unwrap_or_else(|| Path::new("."));
                let scan =
                    discover_callers(ws_root, &fn_ir.name, &target_path, params.exclude_tests);
                if let Some(reason) = scan.truncated {
                    extra_truncation.push(reason);
                }
                let root_id = eval
                    .root_ids
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "n0".to_string());
                for (idx, caller) in scan.callers.into_iter().enumerate() {
                    let fid = if let Some(&existing) = file_ids.get(&caller.uri) {
                        existing
                    } else {
                        let id = files.len() as u32;
                        files.push(FileEntry {
                            file_id: id,
                            uri: caller.uri.clone(),
                        });
                        file_ids.insert(caller.uri.clone(), id);
                        id
                    };
                    let caller_id = format!("caller:{fid}:{idx}");
                    eval.raw_nodes.push(RawSliceNode {
                        id: caller_id.clone(),
                        file_id: fid,
                        is_test_file: caller.is_test_file,
                        is_copy_passthrough: false,
                        range: caller.range,
                        label: format!("{} -> {}", caller.caller_name, fn_ir.name),
                        snippet: if params.include_snippets {
                            caller.snippet
                        } else {
                            None
                        },
                        absence: eval.target_absence.to_summary(),
                        provenance: "ripgrep_caller".to_string(),
                        is_guard: false,
                        guard_text: None,
                        is_danger_sink: false,
                        danger_kind: None,
                    });
                    eval.raw_edges.push(RawSliceEdge {
                        from: root_id.clone(),
                        to: caller_id,
                        label: Some("called_by".to_string()),
                        is_back_edge: false,
                        cycle_group_id: None,
                        provenance: "ripgrep_caller".to_string(),
                        edge_side: Some(EdgeSide::Downstream),
                    });
                }
            }
        }

        let phase1_budget = if params.include_tree
            || matches!(
                params.direction,
                SliceDirection::From | SliceDirection::Both
            ) {
            Duration::from_millis(300)
        } else {
            Duration::from_millis(50)
        };

        let pipeline_out = run_reduction_pipeline(
            &eval.root_ids,
            &eval.raw_nodes,
            &eval.raw_edges,
            params.exclude_tests,
            phase1_budget,
            start,
        );

        let mut truncation = pipeline_out.truncation;
        for reason in extra_truncation {
            if !truncation.contains(&reason) {
                truncation.push(reason);
            }
        }
        if cancel.load(Ordering::Relaxed) {
            truncation.push(TruncationReason::Cancelled);
        }

        let symbol_name = fn_ir
            .inst(target_vid)
            .and_then(|i| i.name.clone())
            .unwrap_or_else(|| fn_ir.name.clone());

        let hover_card = project_hover_card(
            &symbol_name,
            eval.target_absence,
            &files,
            eval.hover_items.clone(),
            Vec::new(),
            eval.guards.clone(),
            eval.local_risks.clone(),
            eval.unknown_boundaries.clone(),
            &truncation,
        );
        let focus_spans = if params.include_focus {
            Some(project_focus_spans(&pipeline_out.graph, &files))
        } else {
            None
        };
        let tree_rows = if params.include_tree {
            Some(project_tree_rows(
                &eval.root_ids,
                &pipeline_out.graph,
                &files,
            ))
        } else {
            None
        };

        let phase1_ms = start.elapsed().as_millis() as u64;

        let phase1_envelope = SliceEnvelope {
            schema_version: SCHEMA_VERSION,
            query_id,
            trace_id: trace_id.to_string(),
            root_ids: eval.root_ids.clone(),
            direction: params.direction,
            files: files.clone(),
            graph: if params.include_graph {
                Some(pipeline_out.graph.clone())
            } else {
                None
            },
            projections: SliceProjections {
                hover_card: Some(hover_card),
                focus_spans,
                tree_rows,
            },
            truncation: truncation.clone(),
            stats: SliceStats {
                phase1_ms,
                phase2_ms: 0,
                nodes_visited: pipeline_out.nodes_visited,
                nodes_emitted: pipeline_out.graph.nodes.len(),
                contracted_chains: pipeline_out.contracted_chains,
                unknown_external_generic_calls: eval.unknown_external_generic_calls,
                total_external_generic_calls: eval.total_external_generic_calls,
                uninstalled_external_calls: eval.uninstalled_external_calls,
            },
        };

        tracing::info!(
            trace_id = %trace_id,
            query_id = query_id,
            method = "lineage/slice",
            uri = %params.uri,
            phase1_ms = phase1_ms,
            phase2_ms = 0u64,
            truncation = ?phase1_envelope.truncation,
            unknown_count = eval.unknown_boundaries.len(),
            "phase1 slice completed"
        );

        let snapshot_graph = pipeline_out.graph.clone();

        let phase2_ctx = if params.lsp_attached
            && !eval.pending_lsp_items.is_empty()
            && !cancel.load(Ordering::Relaxed)
        {
            Some(Phase2Context {
                params: params.clone(),
                trace_id: trace_id.to_string(),
                query_id,
                start,
                phase2_ceiling_ms: PHASE2_CEILING_MS,
                files,
                file_ids,
                fn_name: fn_ir.name.clone(),
                fn_range: fn_ir.range,
                symbol_name,
                eval,
                pipeline_graph: pipeline_out.graph,
                pipeline_nodes_visited: pipeline_out.nodes_visited,
                pipeline_contracted_chains: pipeline_out.contracted_chains,
                truncation,
                phase1_ms,
                phase1_focus_spans: phase1_envelope.projections.focus_spans.clone(),
                phase1_tree_rows: phase1_envelope.projections.tree_rows.clone(),
                phase1_nodes_emitted: phase1_envelope.stats.nodes_emitted,
            })
        } else {
            None
        };

        Ok((phase1_envelope, snapshot_graph, phase2_ctx))
    }

    pub fn execute_phase2(&self, ctx: Phase2Context, cancel: &AtomicBool) -> Option<SliceEnvelope> {
        self.execute_phase2_with_status(ctx, cancel)
            .map(|(env, _graph, _status)| env)
    }

    pub fn execute_phase2_with_status(
        &self,
        mut ctx: Phase2Context,
        cancel: &AtomicBool,
    ) -> Option<(SliceEnvelope, SliceGraph, Phase2Status)> {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let max_timeout_ms = ctx
            .params
            .lsp_timeout_ms
            .unwrap_or(DEFAULT_LSP_TIMEOUT_MS)
            .min(ctx.phase2_ceiling_ms);
        let batch = LspResolveBatchParams {
            query_id: ctx.query_id,
            trace_id: ctx.trace_id.clone(),
            depth: 1,
            timeout_ms: max_timeout_ms,
            items: ctx.eval.pending_lsp_items.clone(),
            circuit_open_clients: Vec::new(),
            max_timeout_ms: Some(max_timeout_ms),
        };
        let resolve_ctx = ResolveContext {
            workspace_root: ctx.params.workspace_root.clone(),
            lsp_client_name: ctx.params.lsp_client_name.clone(),
        };
        let batch_res = self
            .resolver()
            .resolve_batch(batch, &resolve_ctx, Instant::now());
        if cancel.load(Ordering::Relaxed) {
            return None;
        }

        let adapter = self.adapter_for_uri(&ctx.params.uri);
        let mut phase2_items = Vec::new();
        let mut phase2_tree_rows: Vec<TreeRow> = Vec::new();
        let mut updated_boundaries = ctx.eval.unknown_boundaries.clone();
        let mut resolved_absence = ctx.eval.target_absence;
        let mut any_timed_out = false;
        let root_id = ctx
            .eval
            .root_ids
            .first()
            .cloned()
            .unwrap_or_else(|| "n0".to_string());

        for (idx, item_res) in batch_res.results.into_iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let matched_item = ctx
                .eval
                .pending_lsp_items
                .get(idx)
                .filter(|it| it.key == item_res.key)
                .or_else(|| {
                    ctx.eval
                        .pending_lsp_items
                        .iter()
                        .find(|it| it.key == item_res.key)
                });
            let item_kind = matched_item
                .map(|it| it.kind)
                .unwrap_or(LspResolveKind::Definition);
            let item_pos = matched_item.map(|it| CanonicalPos {
                line: it.position.line,
                col: it.position.character,
            });

            if item_res.timed_out {
                any_timed_out = true;
                updated_boundaries.push(UnknownBoundary {
                    file_id: 0,
                    uri: ctx.params.uri.clone(),
                    range: ctx.fn_range,
                    symbol: item_res.key.clone(),
                    reason: UnknownReason::LspTimeout,
                });
                continue;
            }
            if item_res.circuit_open {
                updated_boundaries.push(UnknownBoundary {
                    file_id: 0,
                    uri: ctx.params.uri.clone(),
                    range: ctx.fn_range,
                    symbol: item_res.key.clone(),
                    reason: UnknownReason::CircuitOpen,
                });
                continue;
            }
            if item_res.unattached {
                continue;
            }
            if item_res.locations.len() > MAX_IMPLEMENTATION_TARGETS
                && item_kind != LspResolveKind::CallHierarchyIncoming
            {
                updated_boundaries.push(UnknownBoundary {
                    file_id: 0,
                    uri: ctx.params.uri.clone(),
                    range: ctx.fn_range,
                    symbol: item_res.key.clone(),
                    reason: UnknownReason::PolymorphicCap,
                });
                continue;
            }

            if item_kind == LspResolveKind::CallHierarchyIncoming {
                if item_res.locations.is_empty() {
                    let target_path = uri_to_path(&ctx.params.uri);
                    let ws_root = ctx
                        .params
                        .workspace_root
                        .as_deref()
                        .map(Path::new)
                        .or_else(|| target_path.parent())
                        .unwrap_or_else(|| Path::new("."));
                    let scan = discover_callers(
                        ws_root,
                        &ctx.fn_name,
                        &target_path,
                        ctx.params.exclude_tests,
                    );
                    if let Some(reason) = scan.truncated {
                        if !ctx.truncation.contains(&reason) {
                            ctx.truncation.push(reason);
                        }
                    }
                    for (cidx, caller) in scan.callers.into_iter().enumerate() {
                        let fid = if let Some(&existing) = ctx.file_ids.get(&caller.uri) {
                            existing
                        } else {
                            let id = ctx.files.len() as u32;
                            ctx.files.push(FileEntry {
                                file_id: id,
                                uri: caller.uri.clone(),
                            });
                            ctx.file_ids.insert(caller.uri.clone(), id);
                            id
                        };
                        let is_absent = resolved_absence.is_maybe_absent();
                        let badge = if is_absent {
                            "[NULL]".to_string()
                        } else if resolved_absence.has_unknown()
                            || resolved_absence == AbsenceValue::bottom()
                        {
                            "[?]".to_string()
                        } else {
                            "[OK]".to_string()
                        };
                        let caller_id = format!("caller:{fid}:{cidx}");
                        let caller_label = format!("{} -> {}", caller.caller_name, ctx.fn_name);
                        let caller_snippet = if ctx.params.include_snippets {
                            caller.snippet
                        } else {
                            None
                        };
                        ctx.pipeline_graph.nodes.push(FlowNode {
                            id: caller_id.clone(),
                            kind: NodeKind::SourceNode,
                            file_id: fid,
                            range: caller.range,
                            label: caller_label.clone(),
                            snippet: caller_snippet.clone(),
                            absence: resolved_absence.to_summary(),
                            provenance: "ripgrep_caller".to_string(),
                            is_guard: false,
                            guard_text: None,
                            is_danger_sink: false,
                            danger_kind: None,
                        });
                        ctx.pipeline_graph.edges.push(FlowEdge {
                            from: root_id.clone(),
                            to: caller_id.clone(),
                            label: Some("called_by".to_string()),
                            is_back_edge: false,
                            cycle_group_id: None,
                            provenance: "ripgrep_caller".to_string(),
                            edge_side: Some(EdgeSide::Downstream),
                        });
                        phase2_tree_rows.push(TreeRow {
                            node_id: caller_id,
                            depth: 1,
                            file_id: fid,
                            uri: caller.uri.clone(),
                            range: caller.range,
                            label: caller_label,
                            badge,
                            is_cycle: false,
                            is_frontier: false,
                            expansion_token: None,
                            is_absent,
                            group_key: ctx.symbol_name.clone(),
                            provenance: "ripgrep_caller".to_string(),
                            has_children: false,
                            snippet: caller_snippet,
                        });
                    }
                } else {
                    for (cidx, loc) in item_res.locations.into_iter().enumerate() {
                        if ctx.start.elapsed() > Duration::from_millis(ctx.phase2_ceiling_ms) {
                            any_timed_out = true;
                            break;
                        }
                        let fid = if let Some(&existing) = ctx.file_ids.get(&loc.uri) {
                            existing
                        } else {
                            let id = ctx.files.len() as u32;
                            ctx.files.push(FileEntry {
                                file_id: id,
                                uri: loc.uri.clone(),
                            });
                            ctx.file_ids.insert(loc.uri.clone(), id);
                            id
                        };
                        let mut loc_range = CanonicalRange {
                            start: CanonicalPos {
                                line: loc.start.line,
                                col: loc.start.character,
                            },
                            end: CanonicalPos {
                                line: loc.end.line,
                                col: loc.end.character,
                            },
                        };
                        if let Ok((caller_text, _)) = self.load_source(&loc.uri) {
                            let mapper = PositionMapper::new(&caller_text);
                            let enc = item_res.offset_encoding.unwrap_or(OffsetEncoding::Utf16);
                            loc_range = CanonicalRange {
                                start: mapper.lsp_to_canonical(loc.start, enc),
                                end: mapper.lsp_to_canonical(loc.end, enc),
                            };
                        }
                        let is_absent = resolved_absence.is_maybe_absent();
                        let badge = if is_absent {
                            "[NULL]".to_string()
                        } else if resolved_absence.has_unknown()
                            || resolved_absence == AbsenceValue::bottom()
                        {
                            "[?]".to_string()
                        } else {
                            "[OK]".to_string()
                        };
                        let caller_id = format!("lsp_caller:{fid}:{cidx}");
                        let caller_label = format!("caller -> {}", ctx.fn_name);
                        ctx.pipeline_graph.nodes.push(FlowNode {
                            id: caller_id.clone(),
                            kind: NodeKind::SourceNode,
                            file_id: fid,
                            range: loc_range,
                            label: caller_label.clone(),
                            snippet: None,
                            absence: resolved_absence.to_summary(),
                            provenance: "lsp_call_hierarchy".to_string(),
                            is_guard: false,
                            guard_text: None,
                            is_danger_sink: false,
                            danger_kind: None,
                        });
                        ctx.pipeline_graph.edges.push(FlowEdge {
                            from: root_id.clone(),
                            to: caller_id.clone(),
                            label: Some("called_by".to_string()),
                            is_back_edge: false,
                            cycle_group_id: None,
                            provenance: "lsp_call_hierarchy".to_string(),
                            edge_side: Some(EdgeSide::Downstream),
                        });
                        phase2_tree_rows.push(TreeRow {
                            node_id: caller_id,
                            depth: 1,
                            file_id: fid,
                            uri: loc.uri.clone(),
                            range: loc_range,
                            label: caller_label,
                            badge,
                            is_cycle: false,
                            is_frontier: false,
                            expansion_token: None,
                            is_absent,
                            group_key: ctx.symbol_name.clone(),
                            provenance: "lsp_call_hierarchy".to_string(),
                            has_children: false,
                            snippet: None,
                        });
                    }
                }
                continue;
            }

            if !item_res.locations.is_empty() {
                updated_boundaries.retain(|b| {
                    !(b.symbol == item_res.key
                        && matches!(
                            b.reason,
                            UnknownReason::ExternalUntyped
                                | UnknownReason::ExternalUninstalled
                                | UnknownReason::BarrelFanoutCap
                        ))
                });
            }

            let attach_node_id = item_pos
                .and_then(|pos| {
                    ctx.pipeline_graph
                        .nodes
                        .iter()
                        .find(|n| n.file_id == 0 && n.range.start == pos)
                        .map(|n| n.id.clone())
                })
                .unwrap_or_else(|| root_id.clone());

            for (lidx, loc) in item_res.locations.into_iter().enumerate() {
                if ctx.start.elapsed() > Duration::from_millis(ctx.phase2_ceiling_ms) {
                    any_timed_out = true;
                    break;
                }
                let fid = if let Some(&existing) = ctx.file_ids.get(&loc.uri) {
                    existing
                } else {
                    let id = ctx.files.len() as u32;
                    ctx.files.push(FileEntry {
                        file_id: id,
                        uri: loc.uri.clone(),
                    });
                    ctx.file_ids.insert(loc.uri.clone(), id);
                    id
                };

                let mut loc_range = CanonicalRange {
                    start: CanonicalPos {
                        line: loc.start.line,
                        col: loc.start.character,
                    },
                    end: CanonicalPos {
                        line: loc.end.line,
                        col: loc.end.character,
                    },
                };
                let mut target_absence = resolved_absence;

                // If target is a .d.ts or .pyi stub, extract outer-wrapper signature
                if loc.uri.ends_with(".d.ts") || loc.uri.ends_with(".pyi") {
                    if let Ok((stub_text, _)) = self.load_source(&loc.uri) {
                        let stub_mapper = PositionMapper::new(&stub_text);
                        let enc = item_res.offset_encoding.unwrap_or(OffsetEncoding::Utf16);
                        loc_range = CanonicalRange {
                            start: stub_mapper.lsp_to_canonical(loc.start, enc),
                            end: stub_mapper.lsp_to_canonical(loc.end, enc),
                        };
                        if let Some(ad) = adapter {
                            if let Some(sig_abs) = ad.extract_declaration_signature(
                                &loc.uri,
                                &stub_text,
                                &item_res.key,
                            ) {
                                target_absence = sig_abs;
                                resolved_absence = resolved_absence.join(sig_abs);
                            }
                        }
                    }
                } else if let Some(target_adapter) = self.adapter_for_uri(&loc.uri) {
                    if let Ok((target_skel, target_text)) =
                        self.get_or_parse_skeleton(&loc.uri, target_adapter)
                    {
                        let target_mapper = PositionMapper::new(&target_text);
                        let enc = item_res.offset_encoding.unwrap_or(OffsetEncoding::Utf16);
                        let cpos = target_mapper.lsp_to_canonical(loc.start, enc);
                        loc_range = CanonicalRange {
                            start: cpos,
                            end: target_mapper.lsp_to_canonical(loc.end, enc),
                        };
                        if let Some(fskel) = target_skel
                            .functions
                            .iter()
                            .find(|f| f.range.start <= cpos && cpos <= f.range.end)
                        {
                            let target_ir = self.get_or_lower_function(
                                &loc.uri,
                                &target_text,
                                &target_skel,
                                fskel,
                                target_adapter,
                            );
                            target_absence = target_ir.local_summary.return_absence;
                            resolved_absence = resolved_absence.join(target_absence);
                        }
                    }
                }

                let lsp_node_id = format!("lsp_def:{fid}:{idx}:{lidx}");
                let lsp_label = format!("LSP target {}", item_res.key);
                ctx.pipeline_graph.nodes.push(FlowNode {
                    id: lsp_node_id.clone(),
                    kind: NodeKind::SourceNode,
                    file_id: fid,
                    range: loc_range,
                    label: lsp_label.clone(),
                    snippet: None,
                    absence: target_absence.to_summary(),
                    provenance: "lsp_definition".to_string(),
                    is_guard: false,
                    guard_text: None,
                    is_danger_sink: false,
                    danger_kind: None,
                });
                ctx.pipeline_graph.edges.push(FlowEdge {
                    from: attach_node_id.clone(),
                    to: lsp_node_id,
                    label: Some("lsp_def".to_string()),
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "lsp_definition".to_string(),
                    edge_side: Some(EdgeSide::Upstream),
                });

                phase2_items.push(HoverItem {
                    index: 0,
                    file_id: fid,
                    uri: loc.uri.clone(),
                    range: loc_range,
                    label: lsp_label,
                    snippet: None,
                    provenance: "lsp_definition".to_string(),
                    is_absent_origin: resolved_absence.is_maybe_absent(),
                });
            }
        }

        if !updated_boundaries
            .iter()
            .any(|b| b.reason == UnknownReason::ExternalUntyped)
        {
            resolved_absence = resolved_absence.without_unknown(UnknownReason::ExternalUntyped);
        }
        if !updated_boundaries
            .iter()
            .any(|b| b.reason == UnknownReason::ExternalUninstalled)
        {
            resolved_absence = resolved_absence.without_unknown(UnknownReason::ExternalUninstalled);
        }

        // Propagate resolved absence into the root node(s) of pipeline_graph.
        for node in &mut ctx.pipeline_graph.nodes {
            if ctx.eval.root_ids.contains(&node.id) {
                node.absence = resolved_absence.to_summary();
            }
        }

        let phase2_ms = ctx.start.elapsed().as_millis() as u64;
        let updated_card = project_hover_card(
            &ctx.symbol_name,
            resolved_absence,
            &ctx.files,
            ctx.eval.hover_items,
            phase2_items,
            ctx.eval.guards,
            ctx.eval.local_risks,
            updated_boundaries.clone(),
            &ctx.truncation,
        );

        let merged_tree_rows = if ctx.params.include_tree {
            let mut rows = ctx.phase1_tree_rows.unwrap_or_default();
            rows.extend(phase2_tree_rows);
            Some(rows)
        } else {
            ctx.phase1_tree_rows
        };

        let snapshot_graph = ctx.pipeline_graph.clone();
        let nodes_emitted = ctx.pipeline_graph.nodes.len().max(ctx.phase1_nodes_emitted);

        let delta_env = SliceEnvelope {
            schema_version: SCHEMA_VERSION,
            query_id: ctx.query_id,
            trace_id: ctx.trace_id.clone(),
            root_ids: ctx.eval.root_ids,
            direction: ctx.params.direction,
            files: ctx.files,
            graph: if ctx.params.include_graph {
                Some(ctx.pipeline_graph)
            } else {
                None
            },
            projections: SliceProjections {
                hover_card: Some(updated_card),
                focus_spans: ctx.phase1_focus_spans,
                tree_rows: merged_tree_rows,
            },
            truncation: ctx.truncation.clone(),
            stats: SliceStats {
                phase1_ms: ctx.phase1_ms,
                phase2_ms,
                nodes_visited: ctx.pipeline_nodes_visited,
                nodes_emitted,
                contracted_chains: ctx.pipeline_contracted_chains,
                unknown_external_generic_calls: ctx.eval.unknown_external_generic_calls,
                total_external_generic_calls: ctx.eval.total_external_generic_calls,
                uninstalled_external_calls: ctx.eval.uninstalled_external_calls,
            },
        };

        tracing::info!(
            trace_id = %ctx.trace_id,
            query_id = ctx.query_id,
            method = "lineage/sliceDelta",
            uri = %ctx.params.uri,
            phase1_ms = ctx.phase1_ms,
            phase2_ms = phase2_ms,
            truncation = ?ctx.truncation,
            unknown_count = updated_boundaries.len(),
            "phase2 slice delta completed"
        );

        let status = if any_timed_out {
            Phase2Status::TimedOut
        } else {
            Phase2Status::Complete
        };

        Some((delta_env, snapshot_graph, status))
    }

    fn backward_reachable(fn_ir: &FunctionIr, target: ValueId) -> HashSet<ValueId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(target);
        queue.push_back(target);

        while let Some(vid) = queue.pop_front() {
            let Some(inst) = fn_ir.inst(vid) else {
                continue;
            };
            match &inst.op {
                SsiOp::Assign { src } | SsiOp::Pi { src, .. } => {
                    if visited.insert(*src) {
                        queue.push_back(*src);
                    }
                }
                SsiOp::Phi { incoming } => {
                    for &(_b, src) in incoming {
                        if visited.insert(src) {
                            queue.push_back(src);
                        }
                    }
                }
                SsiOp::ProjectField { base, .. }
                | SsiOp::ProjectIndex { base, .. }
                | SsiOp::UnwrapVariant { base, .. } => {
                    if visited.insert(*base) {
                        queue.push_back(*base);
                    }
                }
                SsiOp::Call { inputs, .. } => {
                    for &arg in inputs {
                        if visited.insert(arg) {
                            queue.push_back(arg);
                        }
                    }
                }
                SsiOp::Const { .. } | SsiOp::Param { .. } | SsiOp::UnknownOp { .. } => {}
            }
        }
        visited
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_function_ir(
        &self,
        uri: &str,
        skeleton: &FileSkeleton,
        source_text: &str,
        fn_ir: &FunctionIr,
        target_vid: ValueId,
        adapter: &Arc<dyn LanguageAdapter>,
        files: &mut Vec<FileEntry>,
        file_ids: &mut HashMap<String, u32>,
        workspace_root: Option<&Path>,
        start: Instant,
        include_snippets: bool,
    ) -> EvalResult {
        let mut values: HashMap<ValueId, AbsenceValue> = HashMap::new();
        let mut block_envs: HashMap<crate::ir::BlockId, GuardEnv> = HashMap::new();
        let mut raw_nodes = Vec::new();
        let mut raw_edges = Vec::new();
        let mut hover_items = Vec::new();
        let mut guards_desc = Vec::new();
        let mut unknown_boundaries = Vec::new();
        let mut pending_lsp_items = Vec::new();
        let mut unknown_external_generic_calls = 0usize;
        let mut total_external_generic_calls = 0usize;
        let mut uninstalled_external_calls = 0usize;

        let reachable = Self::backward_reachable(fn_ir, target_vid);

        for block in &fn_ir.blocks {
            let mut guard_env = if block.preds.len() == 1 {
                block_envs.get(&block.preds[0]).cloned().unwrap_or_default()
            } else {
                GuardEnv::new()
            };
            let block_in_slice = block.insts.iter().any(|vid| reachable.contains(vid));
            for g in &block.guards {
                guard_env.add_fact(g.clone());
                if block_in_slice {
                    guards_desc.push(format!("{g:?}"));
                }
            }
            block_envs.insert(block.id, guard_env.clone());
            for &vid in &block.insts {
                let Some(inst) = fn_ir.inst(vid) else {
                    continue;
                };
                let in_slice = reachable.contains(&vid);
                let mut dummy_bounds = Vec::new();
                let mut dummy_lsp = Vec::new();
                let mut dummy_unk = 0usize;
                let mut dummy_tot = 0usize;
                let mut dummy_uninst = 0usize;
                let mut slice_graph = if in_slice {
                    Some((&mut raw_nodes, &mut raw_edges))
                } else {
                    None
                };
                let val = self.eval_inst(
                    inst,
                    uri,
                    skeleton,
                    source_text,
                    &values,
                    &guard_env,
                    adapter,
                    files,
                    file_ids,
                    workspace_root,
                    start,
                    if in_slice {
                        &mut unknown_boundaries
                    } else {
                        &mut dummy_bounds
                    },
                    if in_slice {
                        &mut pending_lsp_items
                    } else {
                        &mut dummy_lsp
                    },
                    if in_slice {
                        &mut unknown_external_generic_calls
                    } else {
                        &mut dummy_unk
                    },
                    if in_slice {
                        &mut total_external_generic_calls
                    } else {
                        &mut dummy_tot
                    },
                    if in_slice {
                        &mut uninstalled_external_calls
                    } else {
                        &mut dummy_uninst
                    },
                    &mut slice_graph,
                );
                values.insert(vid, val);
                if in_slice {
                    self.record_inst_node(
                        inst,
                        val,
                        uri,
                        adapter,
                        include_snippets,
                        &mut raw_nodes,
                        &mut raw_edges,
                        &mut hover_items,
                    );
                }
            }
        }

        let target_absence = match values.get(&target_vid).copied() {
            Some(v) => v,
            None => {
                unknown_boundaries.push(UnknownBoundary {
                    file_id: 0,
                    uri: uri.to_string(),
                    range: fn_ir.range,
                    symbol: fn_ir.name.clone(),
                    reason: UnknownReason::ExternalUntyped,
                });
                AbsenceValue::unknown(UnknownReason::ExternalUntyped)
            }
        };

        let local_risks: Vec<HoverRisk> = fn_ir
            .danger_sinks
            .iter()
            .filter(|sink| sink.target == target_vid || reachable.contains(&sink.target))
            .filter_map(|sink| {
                let sink_val = values.get(&sink.target).copied().unwrap_or_default();
                if sink_val.is_maybe_absent() || sink_val.is_definitely_absent() {
                    let target_node_id = format!("n{}", sink.target.0);
                    let danger_kind = format!("{:?}", sink.kind);
                    for node in &mut raw_nodes {
                        if node.id == target_node_id {
                            node.is_danger_sink = true;
                            node.danger_kind = Some(danger_kind.clone());
                        }
                    }
                    Some(HoverRisk {
                        file_id: 0,
                        uri: uri.to_string(),
                        range: sink.range,
                        kind: danger_kind,
                        message: format!("Unguarded {} on potentially absent value", sink.label),
                        node_id: Some(target_node_id),
                    })
                } else {
                    None
                }
            })
            .collect();

        EvalResult {
            root_ids: vec![format!("n{}", target_vid.0)],
            target_absence,
            raw_nodes,
            raw_edges,
            hover_items,
            guards: guards_desc,
            local_risks,
            unknown_boundaries,
            pending_lsp_items,
            unknown_external_generic_calls,
            total_external_generic_calls,
            uninstalled_external_calls,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_inst(
        &self,
        inst: &SsiInst,
        uri: &str,
        skeleton: &FileSkeleton,
        source_text: &str,
        values: &HashMap<ValueId, AbsenceValue>,
        guard_env: &GuardEnv,
        adapter: &Arc<dyn LanguageAdapter>,
        files: &mut Vec<FileEntry>,
        file_ids: &mut HashMap<String, u32>,
        workspace_root: Option<&Path>,
        start: Instant,
        unknown_boundaries: &mut Vec<UnknownBoundary>,
        pending_lsp_items: &mut Vec<LspResolveItem>,
        unknown_ext_calls: &mut usize,
        total_ext_calls: &mut usize,
        uninstalled_ext_calls: &mut usize,
        slice_graph: &mut Option<(&mut Vec<RawSliceNode>, &mut Vec<RawSliceEdge>)>,
    ) -> AbsenceValue {
        match &inst.op {
            SsiOp::Const { value, .. } => *value,
            SsiOp::Param {
                default_absence, ..
            } => *default_absence,
            SsiOp::Assign { src } => values.get(src).copied().unwrap_or_default(),
            SsiOp::Phi { incoming } => {
                let mut acc = AbsenceValue::bottom();
                for &(_blk, vid) in incoming {
                    if let Some(&v) = values.get(&vid) {
                        acc = acc.join(v);
                    }
                }
                acc
            }
            SsiOp::Pi { src, guard } => {
                let base = values.get(src).copied().unwrap_or_default();
                let mut local_env = guard_env.clone();
                local_env.add_fact(guard.clone());
                let place = inst.name.as_deref().unwrap_or("_");
                let narrowed = local_env.apply_to(place, base);
                if narrowed.known_states == 0 && !narrowed.has_unknown() {
                    base.narrow_mask(STATE_PRESENT | STATE_NULL | STATE_UNDEFINED)
                } else {
                    narrowed
                }
            }
            SsiOp::ProjectField { base, .. }
            | SsiOp::ProjectIndex { base, .. }
            | SsiOp::UnwrapVariant { base, .. } => values.get(base).copied().unwrap_or_default(),
            SsiOp::UnknownOp { reason, symbol } => {
                unknown_boundaries.push(UnknownBoundary {
                    file_id: 0,
                    uri: uri.to_string(),
                    range: inst.range,
                    symbol: symbol.clone(),
                    reason: *reason,
                });
                AbsenceValue::unknown(*reason)
            }
            SsiOp::Call { callee, inputs, .. } => match callee {
                CallTarget::Stdlib(sym) => {
                    *total_ext_calls += 1;
                    if let Some(abs) = adapter.stdlib_summary(sym) {
                        abs
                    } else {
                        *unknown_ext_calls += 1;
                        unknown_boundaries.push(UnknownBoundary {
                            file_id: 0,
                            uri: uri.to_string(),
                            range: inst.range,
                            symbol: sym.clone(),
                            reason: UnknownReason::ExternalUntyped,
                        });
                        AbsenceValue::unknown(UnknownReason::ExternalUntyped)
                    }
                }
                CallTarget::Local(sym) => {
                    if let Some(fskel) = skeleton.functions.iter().find(|f| f.name == *sym) {
                        let callee_ir =
                            self.get_or_lower_function(uri, source_text, skeleton, fskel, adapter);
                        let mut ret_abs = callee_ir.local_summary.return_absence;
                        for &param_idx in &callee_ir.local_summary.param_flow_to_return {
                            if let Some(&arg_vid) = inputs.get(param_idx) {
                                if let Some(&arg_val) = values.get(&arg_vid) {
                                    ret_abs = ret_abs.join(arg_val);
                                }
                            }
                        }
                        return ret_abs;
                    }
                    let from_path = uri_to_path(uri);
                    for sib_path in adapter.resolve_same_package_siblings(&from_path, false, start)
                    {
                        let sib_uri = path_to_uri(&sib_path);
                        if let Ok((sib_skel, sib_text)) =
                            self.get_or_parse_skeleton(&sib_uri, adapter)
                        {
                            if let Some(fskel) = sib_skel.functions.iter().find(|f| f.name == *sym)
                            {
                                let sib_fid = if let Some(&existing) = file_ids.get(&sib_uri) {
                                    existing
                                } else {
                                    let fid = files.len() as u32;
                                    files.push(FileEntry {
                                        file_id: fid,
                                        uri: sib_uri.clone(),
                                    });
                                    file_ids.insert(sib_uri.clone(), fid);
                                    fid
                                };
                                let callee_ir = self.get_or_lower_function(
                                    &sib_uri, &sib_text, &sib_skel, fskel, adapter,
                                );
                                let mut ret_abs = callee_ir.local_summary.return_absence;
                                for &param_idx in &callee_ir.local_summary.param_flow_to_return {
                                    if let Some(&arg_vid) = inputs.get(param_idx) {
                                        if let Some(&arg_val) = values.get(&arg_vid) {
                                            ret_abs = ret_abs.join(arg_val);
                                        }
                                    }
                                }
                                push_cross_file_callee(
                                    slice_graph,
                                    inst.id,
                                    sib_fid,
                                    &sib_uri,
                                    fskel.range,
                                    format!("fn {sym}"),
                                    ret_abs,
                                    adapter,
                                );
                                return ret_abs;
                            }
                        }
                    }
                    unknown_boundaries.push(UnknownBoundary {
                        file_id: 0,
                        uri: uri.to_string(),
                        range: inst.range,
                        symbol: sym.clone(),
                        reason: UnknownReason::DynamicDispatch,
                    });
                    AbsenceValue::unknown(UnknownReason::DynamicDispatch)
                }
                CallTarget::Imported {
                    module_specifier,
                    symbol,
                } => {
                    *total_ext_calls += 1;
                    let from_path = uri_to_path(uri);
                    match adapter.resolve_static_import(
                        &from_path,
                        module_specifier,
                        workspace_root,
                        start,
                    ) {
                        ImportResolution::Resolved(target_path) => {
                            let target_uri = path_to_uri(&target_path);
                            let target_fid = if let Some(&existing) = file_ids.get(&target_uri) {
                                existing
                            } else {
                                let fid = files.len() as u32;
                                files.push(FileEntry {
                                    file_id: fid,
                                    uri: target_uri.clone(),
                                });
                                file_ids.insert(target_uri.clone(), fid);
                                fid
                            };
                            if target_uri.ends_with(".d.ts") || target_uri.ends_with(".pyi") {
                                if let Ok((stub_text, _)) = self.load_source(&target_uri) {
                                    if let Some(abs) = adapter.extract_declaration_signature(
                                        &target_uri,
                                        &stub_text,
                                        symbol,
                                    ) {
                                        if abs.has_unknown() {
                                            *unknown_ext_calls += 1;
                                        }
                                        push_cross_file_callee(
                                            slice_graph,
                                            inst.id,
                                            target_fid,
                                            &target_uri,
                                            CanonicalRange {
                                                start: CanonicalPos { line: 0, col: 0 },
                                                end: CanonicalPos { line: 0, col: 0 },
                                            },
                                            format!("{symbol} (stub)"),
                                            abs,
                                            adapter,
                                        );
                                        return abs;
                                    }
                                }
                            } else if let Some(target_adapter) = self.adapter_for_uri(&target_uri) {
                                if let Ok((target_skel, target_text)) =
                                    self.get_or_parse_skeleton(&target_uri, target_adapter)
                                {
                                    if let Some(fskel) =
                                        target_skel.functions.iter().find(|f| f.name == *symbol)
                                    {
                                        let target_ir = self.get_or_lower_function(
                                            &target_uri,
                                            &target_text,
                                            &target_skel,
                                            fskel,
                                            target_adapter,
                                        );
                                        let mut ret_abs = target_ir.local_summary.return_absence;
                                        for &param_idx in
                                            &target_ir.local_summary.param_flow_to_return
                                        {
                                            if let Some(&arg_vid) = inputs.get(param_idx) {
                                                if let Some(&arg_val) = values.get(&arg_vid) {
                                                    ret_abs = ret_abs.join(arg_val);
                                                }
                                            }
                                        }
                                        push_cross_file_callee(
                                            slice_graph,
                                            inst.id,
                                            target_fid,
                                            &target_uri,
                                            fskel.range,
                                            format!("fn {symbol}"),
                                            ret_abs,
                                            target_adapter,
                                        );
                                        return ret_abs;
                                    }
                                    for sib_path in target_adapter.resolve_same_package_siblings(
                                        &target_path,
                                        false,
                                        start,
                                    ) {
                                        let sib_uri = path_to_uri(&sib_path);
                                        if let Ok((sib_skel, sib_text)) =
                                            self.get_or_parse_skeleton(&sib_uri, target_adapter)
                                        {
                                            if let Some(fskel) = sib_skel
                                                .functions
                                                .iter()
                                                .find(|f| f.name == *symbol)
                                            {
                                                let sib_fid = if let Some(&existing) =
                                                    file_ids.get(&sib_uri)
                                                {
                                                    existing
                                                } else {
                                                    let fid = files.len() as u32;
                                                    files.push(FileEntry {
                                                        file_id: fid,
                                                        uri: sib_uri.clone(),
                                                    });
                                                    file_ids.insert(sib_uri.clone(), fid);
                                                    fid
                                                };
                                                let target_ir = self.get_or_lower_function(
                                                    &sib_uri,
                                                    &sib_text,
                                                    &sib_skel,
                                                    fskel,
                                                    target_adapter,
                                                );
                                                let mut ret_abs =
                                                    target_ir.local_summary.return_absence;
                                                for &param_idx in
                                                    &target_ir.local_summary.param_flow_to_return
                                                {
                                                    if let Some(&arg_vid) = inputs.get(param_idx) {
                                                        if let Some(&arg_val) = values.get(&arg_vid)
                                                        {
                                                            ret_abs = ret_abs.join(arg_val);
                                                        }
                                                    }
                                                }
                                                push_cross_file_callee(
                                                    slice_graph,
                                                    inst.id,
                                                    sib_fid,
                                                    &sib_uri,
                                                    fskel.range,
                                                    format!("fn {symbol}"),
                                                    ret_abs,
                                                    target_adapter,
                                                );
                                                return ret_abs;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        ImportResolution::BarrelCapReached => {
                            *unknown_ext_calls += 1;
                            unknown_boundaries.push(UnknownBoundary {
                                file_id: 0,
                                uri: uri.to_string(),
                                range: inst.range,
                                symbol: symbol.clone(),
                                reason: UnknownReason::BarrelFanoutCap,
                            });
                            pending_lsp_items.push(LspResolveItem {
                                key: symbol.clone(),
                                uri: uri.to_string(),
                                position: LspPosition {
                                    line: inst.range.start.line,
                                    character: inst.range.start.col,
                                },
                                offset_encoding: OffsetEncoding::Utf8,
                                kind: LspResolveKind::Definition,
                            });
                            return AbsenceValue::unknown(UnknownReason::BarrelFanoutCap);
                        }
                        ImportResolution::Unresolved => {}
                    }

                    let is_external_pkg = is_external_package_specifier(module_specifier);
                    let reason = if is_external_pkg {
                        *uninstalled_ext_calls += 1;
                        UnknownReason::ExternalUninstalled
                    } else {
                        *unknown_ext_calls += 1;
                        UnknownReason::ExternalUntyped
                    };
                    pending_lsp_items.push(LspResolveItem {
                        key: symbol.clone(),
                        uri: uri.to_string(),
                        position: LspPosition {
                            line: inst.range.start.line,
                            character: inst.range.start.col,
                        },
                        offset_encoding: OffsetEncoding::Utf8,
                        kind: LspResolveKind::Definition,
                    });
                    unknown_boundaries.push(UnknownBoundary {
                        file_id: 0,
                        uri: uri.to_string(),
                        range: inst.range,
                        symbol: symbol.clone(),
                        reason,
                    });
                    AbsenceValue::unknown(reason)
                }
                CallTarget::Dynamic(sym) => {
                    *total_ext_calls += 1;
                    *unknown_ext_calls += 1;
                    unknown_boundaries.push(UnknownBoundary {
                        file_id: 0,
                        uri: uri.to_string(),
                        range: inst.range,
                        symbol: sym.clone(),
                        reason: UnknownReason::DynamicDispatch,
                    });
                    AbsenceValue::unknown(UnknownReason::DynamicDispatch)
                }
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_inst_node(
        &self,
        inst: &SsiInst,
        val: AbsenceValue,
        uri: &str,
        adapter: &Arc<dyn LanguageAdapter>,
        include_snippets: bool,
        raw_nodes: &mut Vec<RawSliceNode>,
        raw_edges: &mut Vec<RawSliceEdge>,
        hover_items: &mut Vec<HoverItem>,
    ) {
        let id = format!("n{}", inst.id.0);
        let label = match &inst.op {
            SsiOp::Const { label, .. } => label.clone(),
            SsiOp::Param { name, .. } => format!("param {name}"),
            SsiOp::Assign { .. } => inst.name.clone().unwrap_or_else(|| "assign".to_string()),
            SsiOp::Phi { .. } => format!("phi {}", inst.name.as_deref().unwrap_or("")),
            SsiOp::Pi { .. } => format!("narrow {}", inst.name.as_deref().unwrap_or("")),
            SsiOp::Call { callee, .. } => format_call_target(callee),
            SsiOp::UnknownOp { symbol, .. } => format!("unverified {symbol}"),
            _ => inst.name.clone().unwrap_or_else(|| "op".to_string()),
        };

        let is_copy = matches!(inst.op, SsiOp::Assign { .. });
        let (is_guard, guard_text) = match &inst.op {
            SsiOp::Pi { guard, .. } => (true, Some(format_guard_fact(guard))),
            _ => (false, None),
        };
        raw_nodes.push(RawSliceNode {
            id: id.clone(),
            file_id: 0,
            is_test_file: adapter.classify_test_file(uri),
            is_copy_passthrough: is_copy,
            range: inst.range,
            label: label.clone(),
            snippet: if include_snippets {
                inst.snippet.clone()
            } else {
                None
            },
            absence: val.to_summary(),
            provenance: "local_ssi".to_string(),
            is_guard,
            guard_text,
            is_danger_sink: false,
            danger_kind: None,
        });

        match &inst.op {
            SsiOp::Assign { src } | SsiOp::Pi { src, .. } => {
                raw_edges.push(RawSliceEdge {
                    from: id.clone(),
                    to: format!("n{}", src.0),
                    label: None,
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "def_use".to_string(),
                    edge_side: Some(EdgeSide::Upstream),
                });
            }
            SsiOp::Phi { incoming } => {
                for &(_b, src) in incoming {
                    raw_edges.push(RawSliceEdge {
                        from: id.clone(),
                        to: format!("n{}", src.0),
                        label: Some("phi".to_string()),
                        is_back_edge: false,
                        cycle_group_id: None,
                        provenance: "phi".to_string(),
                        edge_side: Some(EdgeSide::Upstream),
                    });
                }
            }
            SsiOp::ProjectField { base, field } => {
                raw_edges.push(RawSliceEdge {
                    from: id.clone(),
                    to: format!("n{}", base.0),
                    label: Some(format!(".{field}")),
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "project_field".to_string(),
                    edge_side: Some(EdgeSide::Upstream),
                });
            }
            SsiOp::ProjectIndex { base, .. } => {
                raw_edges.push(RawSliceEdge {
                    from: id.clone(),
                    to: format!("n{}", base.0),
                    label: Some("[]".to_string()),
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "project_index".to_string(),
                    edge_side: Some(EdgeSide::Upstream),
                });
            }
            SsiOp::UnwrapVariant { base, variant } => {
                raw_edges.push(RawSliceEdge {
                    from: id.clone(),
                    to: format!("n{}", base.0),
                    label: Some(format!("unwrap {variant}")),
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "unwrap_variant".to_string(),
                    edge_side: Some(EdgeSide::Upstream),
                });
            }
            SsiOp::Call { inputs, .. } => {
                for &arg in inputs {
                    raw_edges.push(RawSliceEdge {
                        from: id.clone(),
                        to: format!("n{}", arg.0),
                        label: Some("arg".to_string()),
                        is_back_edge: false,
                        cycle_group_id: None,
                        provenance: "call_arg".to_string(),
                        edge_side: Some(EdgeSide::Upstream),
                    });
                }
            }
            SsiOp::Const { .. } | SsiOp::Param { .. } | SsiOp::UnknownOp { .. } => {}
        }

        if val.is_maybe_absent()
            || val.is_definitely_absent()
            || matches!(inst.op, SsiOp::Const { .. })
        {
            hover_items.push(HoverItem {
                index: 0,
                file_id: 0,
                uri: uri.to_string(),
                range: inst.range,
                label,
                snippet: if include_snippets {
                    inst.snippet.clone()
                } else {
                    None
                },
                provenance: "local_ssi".to_string(),
                is_absent_origin: val.is_maybe_absent() || val.is_definitely_absent(),
            });
        }
    }

    fn guard_error_envelope(
        &self,
        params: &SliceParams,
        query_id: u64,
        trace_id: &str,
        err: GuardError,
        start: Instant,
    ) -> SliceEnvelope {
        let reason = err.to_unknown_reason();
        let boundary = UnknownBoundary {
            file_id: 0,
            uri: params.uri.clone(),
            range: CanonicalRange {
                start: CanonicalPos { line: 0, col: 0 },
                end: CanonicalPos { line: 0, col: 0 },
            },
            symbol: params.uri.clone(),
            reason,
        };
        let card = project_hover_card(
            &params.uri,
            AbsenceValue::unknown(reason),
            &[FileEntry {
                file_id: 0,
                uri: params.uri.clone(),
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![boundary],
            &[],
        );
        SliceEnvelope {
            schema_version: SCHEMA_VERSION,
            query_id,
            trace_id: trace_id.to_string(),
            root_ids: Vec::new(),
            direction: params.direction,
            files: vec![FileEntry {
                file_id: 0,
                uri: params.uri.clone(),
            }],
            graph: None,
            projections: SliceProjections {
                hover_card: Some(card),
                focus_spans: None,
                tree_rows: None,
            },
            truncation: Vec::new(),
            stats: SliceStats {
                phase1_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            },
        }
    }

    pub fn expand_node(
        &self,
        params: &ExpandParams,
        trace_id: &str,
        cancel: &AtomicBool,
    ) -> Result<ExpandResult, RpcError> {
        let (res, _graph, _roots) = self.expand_node_with_graph(params, trace_id, cancel)?;
        Ok(res)
    }

    pub fn expand_node_with_graph(
        &self,
        params: &ExpandParams,
        trace_id: &str,
        cancel: &AtomicBool,
    ) -> Result<(ExpandResult, SliceGraph, Vec<String>), RpcError> {
        let slice_params = SliceParams {
            uri: params.uri.clone(),
            position: params.position,
            offset_encoding: params.offset_encoding,
            direction: params.direction,
            include_graph: false,
            include_focus: false,
            include_tree: true,
            include_snippets: true,
            exclude_tests: params.exclude_tests,
            query_id: None,
            lsp_attached: params.lsp_attached,
            lsp_client_name: None,
            workspace_root: params.workspace_root.clone(),
            lsp_timeout_ms: None,
        };
        let (env, graph, _phase2_opt) =
            self.slice_phase1_with_snapshot(&slice_params, trace_id, cancel)?;
        let tree_rows = env.projections.tree_rows.unwrap_or_default();
        Ok((
            ExpandResult {
                schema_version: SCHEMA_VERSION,
                tree_rows,
                files: env.files,
                truncation: env.truncation,
            },
            graph,
            env.root_ids,
        ))
    }

    pub fn dump_ir_text(&self, uri: &str) -> Result<String, String> {
        let adapter = self
            .adapter_for_uri(uri)
            .ok_or_else(|| format!("Unsupported file type: {uri}"))?;
        let (skel, text) = self
            .get_or_parse_skeleton(uri, adapter)
            .map_err(|e| format!("Guard error: {e:?}"))?;
        let mut out = String::new();
        out.push_str(&format!(
            "File: {} (hash={:#x})\n",
            skel.uri, skel.content_hash
        ));
        for f in &skel.functions {
            let ir = self.get_or_lower_function(uri, &text, &skel, f, adapter);
            out.push_str(&format!("fn {} ({}):\n", ir.name, ir.symbol_id));
            for b in &ir.blocks {
                out.push_str(&format!("  bb{} (terminated={}):\n", b.id.0, b.terminated));
                for &vid in &b.insts {
                    if let Some(inst) = ir.inst(vid) {
                        out.push_str(&format!("    v{}: {:?}\n", inst.id.0, inst.op));
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn dump_cfg_dot(&self, uri: &str) -> Result<String, String> {
        let adapter = self
            .adapter_for_uri(uri)
            .ok_or_else(|| format!("Unsupported file type: {uri}"))?;
        let (skel, text) = self
            .get_or_parse_skeleton(uri, adapter)
            .map_err(|e| format!("Guard error: {e:?}"))?;
        let mut out = String::from("digraph cfg {\n");
        let mut seen_nodes = HashSet::new();
        for f in &skel.functions {
            let ir = self.get_or_lower_function(uri, &text, &skel, f, adapter);
            for b in &ir.blocks {
                let node_name = format!("\"{}::bb{}\"", ir.name, b.id.0);
                if seen_nodes.insert(node_name.clone()) {
                    out.push_str(&format!("  {node_name};\n"));
                }
                for succ in &b.succs {
                    out.push_str(&format!(
                        "  {node_name} -> \"{}::bb{}\";\n",
                        ir.name, succ.0
                    ));
                }
            }
        }
        out.push_str("}\n");
        Ok(out)
    }
}

fn format_guard_fact(guard: &GuardFact) -> String {
    match guard {
        GuardFact::StateIn { place, mask } => format!("{place} (mask={mask:#x})"),
        GuardFact::Implies {
            antecedent_place,
            consequent_place,
            ..
        } => format!("{antecedent_place} => {consequent_place}"),
        GuardFact::Unreachable => "unreachable".to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn push_cross_file_callee(
    graph: &mut Option<(&mut Vec<RawSliceNode>, &mut Vec<RawSliceEdge>)>,
    caller_inst_id: ValueId,
    callee_fid: u32,
    callee_uri: &str,
    callee_range: CanonicalRange,
    label: String,
    absence: AbsenceValue,
    adapter: &Arc<dyn LanguageAdapter>,
) {
    if let Some((raw_nodes, raw_edges)) = graph.as_mut() {
        let callee_id = format!(
            "xcall:{}:{}:{}",
            caller_inst_id.0, callee_fid, callee_range.start.line
        );
        raw_nodes.push(RawSliceNode {
            id: callee_id.clone(),
            file_id: callee_fid,
            is_test_file: adapter.classify_test_file(callee_uri),
            is_copy_passthrough: false,
            range: callee_range,
            label,
            snippet: None,
            absence: absence.to_summary(),
            provenance: "cross_file_call".to_string(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        });
        raw_edges.push(RawSliceEdge {
            from: format!("n{}", caller_inst_id.0),
            to: callee_id,
            label: Some("returns".to_string()),
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "cross_file_call".to_string(),
            edge_side: Some(EdgeSide::Upstream),
        });
    }
}

fn format_call_target(callee: &CallTarget) -> String {
    match callee {
        CallTarget::Local(sym) | CallTarget::Stdlib(sym) => {
            if sym.ends_with("()") {
                sym.clone()
            } else {
                format!("{sym}()")
            }
        }
        CallTarget::Imported { symbol, .. } => {
            if symbol.ends_with("()") {
                symbol.clone()
            } else {
                format!("{symbol}()")
            }
        }
        CallTarget::Dynamic(sym) => {
            if sym.ends_with("()") {
                sym.clone()
            } else {
                format!("{sym}()")
            }
        }
    }
}

fn is_external_package_specifier(spec: &str) -> bool {
    !spec.starts_with('.')
        && !spec.starts_with('/')
        && !spec.starts_with("crate::")
        && !spec.starts_with("super::")
        && !spec.starts_with("self::")
}

pub struct Phase2Context {
    params: SliceParams,
    trace_id: String,
    query_id: u64,
    start: Instant,
    pub phase2_ceiling_ms: u64,
    files: Vec<FileEntry>,
    file_ids: HashMap<String, u32>,
    fn_name: String,
    fn_range: CanonicalRange,
    symbol_name: String,
    eval: EvalResult,
    pipeline_graph: SliceGraph,
    pipeline_nodes_visited: usize,
    pipeline_contracted_chains: usize,
    truncation: Vec<TruncationReason>,
    phase1_ms: u64,
    phase1_focus_spans: Option<Vec<crate::protocol::schema::FocusSpan>>,
    phase1_tree_rows: Option<Vec<TreeRow>>,
    phase1_nodes_emitted: usize,
}

struct EvalResult {
    root_ids: Vec<String>,
    target_absence: AbsenceValue,
    raw_nodes: Vec<RawSliceNode>,
    raw_edges: Vec<RawSliceEdge>,
    hover_items: Vec<HoverItem>,
    guards: Vec<String>,
    local_risks: Vec<HoverRisk>,
    unknown_boundaries: Vec<UnknownBoundary>,
    pending_lsp_items: Vec<LspResolveItem>,
    unknown_external_generic_calls: usize,
    total_external_generic_calls: usize,
    uninstalled_external_calls: usize,
}
