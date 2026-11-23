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
    check_text_guards, read_file_guarded, uri_to_path, DirtyBufferOverlay, DiskStamp, FileIrCache,
    GuardError, SliceArena,
};
use crate::ir::{
    AbsenceValue, CallTarget, FileSkeleton, FunctionIr, GuardEnv, SsiInst, SsiOp, ValueId,
    STATE_NULL, STATE_PRESENT, STATE_UNDEFINED,
};
use crate::protocol::schema::{
    CanonicalPos, CanonicalRange, FileEntry, HoverItem, HoverRisk, LspPosition,
    LspResolveBatchParams, LspResolveItem, LspResolveKind, OffsetEncoding, RpcError, SliceEnvelope,
    SliceParams, SliceProjections, SliceStats, TruncationReason, UnknownBoundary, UnknownReason,
    SCHEMA_VERSION,
};
use crate::resolver::lsp_provider::{
    DEFAULT_LSP_TIMEOUT_MS, MAX_IMPLEMENTATION_TARGETS, PHASE2_CEILING_MS,
};
use crate::resolver::SymbolResolver;
use crate::slicer::pipeline::{run_reduction_pipeline, RawSliceEdge, RawSliceNode};
use crate::slicer::projectors::project_hover_card;
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
        skel: &crate::ir::FunctionSkeleton,
        adapter: &Arc<dyn LanguageAdapter>,
    ) -> Arc<FunctionIr> {
        if let Some(cached) = self
            .cache
            .get_function(uri, &skel.symbol_id, skel.content_hash)
        {
            return cached;
        }
        let ir = Arc::new(adapter.lower_function(uri, text, skel));
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
        let start = Instant::now();
        let query_id = params.query_id.unwrap_or(1);
        let _arena = SliceArena::new(); // Query-scoped arena dropped on return.

        let adapter = self.adapter_for_uri(&params.uri).ok_or_else(|| {
            RpcError::invalid_params(format!("Unsupported file type: {}", params.uri))
        })?;

        let (skeleton, text) = match self.get_or_parse_skeleton(&params.uri, adapter) {
            Ok(v) => v,
            Err(guard_err) => {
                return Ok(self.guard_error_envelope(params, query_id, trace_id, guard_err, start));
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

        let fn_ir = self.get_or_lower_function(&params.uri, &text, fn_skel, adapter);

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

        let eval = self.evaluate_function_ir(
            &params.uri,
            &skeleton,
            &fn_ir,
            target_vid,
            adapter,
            &mut files,
            &mut file_ids,
            params.workspace_root.as_deref().map(Path::new),
            start,
            params.include_snippets,
        );

        let phase1_ms = start.elapsed().as_millis() as u64;
        let phase1_budget = Duration::from_millis(50);

        let pipeline_out = run_reduction_pipeline(
            &eval.root_ids,
            &eval.raw_nodes,
            &eval.raw_edges,
            params.exclude_tests,
            phase1_budget,
            start,
        );

        let mut truncation = pipeline_out.truncation;
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
            },
        };

        // Phase 2: if LSP is attached and open LSP-resolvable boundaries exist and not cancelled
        if params.lsp_attached && !eval.pending_lsp_items.is_empty() && !cancel.load(Ordering::Relaxed) {
            if let Some(ref mut emit_delta) = on_delta {
                let batch = LspResolveBatchParams {
                    query_id,
                    trace_id: trace_id.to_string(),
                    depth: 1,
                    timeout_ms: DEFAULT_LSP_TIMEOUT_MS,
                    items: eval.pending_lsp_items.clone(),
                };
                let batch_res = self.resolver().resolve_batch(batch, Instant::now());
                let mut phase2_items = Vec::new();
                let mut updated_boundaries = eval.unknown_boundaries.clone();
                let mut resolved_absence = eval.target_absence;

                for item_res in batch_res.results {
                    if item_res.timed_out {
                        updated_boundaries.push(UnknownBoundary {
                            file_id: 0,
                            uri: params.uri.clone(),
                            range: fn_ir.range,
                            symbol: item_res.key.clone(),
                            reason: UnknownReason::LspTimeout,
                        });
                        continue;
                    }
                    if item_res.circuit_open {
                        updated_boundaries.push(UnknownBoundary {
                            file_id: 0,
                            uri: params.uri.clone(),
                            range: fn_ir.range,
                            symbol: item_res.key.clone(),
                            reason: UnknownReason::CircuitOpen,
                        });
                        continue;
                    }
                    if item_res.unattached {
                        continue;
                    }
                    if item_res.locations.len() > MAX_IMPLEMENTATION_TARGETS {
                        updated_boundaries.push(UnknownBoundary {
                            file_id: 0,
                            uri: params.uri.clone(),
                            range: fn_ir.range,
                            symbol: item_res.key.clone(),
                            reason: UnknownReason::PolymorphicCap,
                        });
                        continue;
                    }
                    for loc in item_res.locations {
                        if start.elapsed() > Duration::from_millis(PHASE2_CEILING_MS) {
                            break;
                        }
                        let fid = if let Some(&existing) = file_ids.get(&loc.uri) {
                            existing
                        } else {
                            let id = files.len() as u32;
                            files.push(FileEntry {
                                file_id: id,
                                uri: loc.uri.clone(),
                            });
                            file_ids.insert(loc.uri.clone(), id);
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

                        // If target is a .d.ts or .pyi stub, extract outer-wrapper signature
                        if loc.uri.ends_with(".d.ts") || loc.uri.ends_with(".pyi") {
                            if let Ok((stub_text, _)) = self.load_source(&loc.uri) {
                                let stub_mapper = PositionMapper::new(&stub_text);
                                let enc = item_res.offset_encoding.unwrap_or(OffsetEncoding::Utf16);
                                loc_range = CanonicalRange {
                                    start: stub_mapper.lsp_to_canonical(loc.start, enc),
                                    end: stub_mapper.lsp_to_canonical(loc.end, enc),
                                };
                                if let Some(sig_abs) = adapter.extract_declaration_signature(
                                    &loc.uri,
                                    &stub_text,
                                    &item_res.key,
                                ) {
                                    resolved_absence = resolved_absence.join(sig_abs);
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
                                        fskel,
                                        target_adapter,
                                    );
                                    resolved_absence = resolved_absence
                                        .join(target_ir.local_summary.return_absence);
                                }
                            }
                        }

                        phase2_items.push(HoverItem {
                            index: 0,
                            file_id: fid,
                            uri: loc.uri.clone(),
                            range: loc_range,
                            label: format!("LSP target {}", item_res.key),
                            snippet: None,
                            provenance: "lsp_definition".to_string(),
                            is_absent_origin: resolved_absence.is_maybe_absent(),
                        });
                    }
                }

                let phase2_ms = start.elapsed().as_millis() as u64;
                let updated_card = project_hover_card(
                    &symbol_name,
                    resolved_absence,
                    &files,
                    eval.hover_items,
                    phase2_items,
                    eval.guards,
                    eval.local_risks,
                    updated_boundaries,
                    &truncation,
                );

                let delta_env = SliceEnvelope {
                    schema_version: SCHEMA_VERSION,
                    query_id,
                    trace_id: trace_id.to_string(),
                    root_ids: eval.root_ids,
                    direction: params.direction,
                    files,
                    graph: if params.include_graph {
                        Some(pipeline_out.graph)
                    } else {
                        None
                    },
                    projections: SliceProjections {
                        hover_card: Some(updated_card),
                    },
                    truncation,
                    stats: SliceStats {
                        phase1_ms,
                        phase2_ms,
                        nodes_visited: pipeline_out.nodes_visited,
                        nodes_emitted: phase1_envelope.stats.nodes_emitted,
                        contracted_chains: pipeline_out.contracted_chains,
                        unknown_external_generic_calls: eval.unknown_external_generic_calls,
                        total_external_generic_calls: eval.total_external_generic_calls,
                    },
                };
                emit_delta(delta_env);
            }
        }

        Ok(phase1_envelope)
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
                SsiOp::Const { .. }
                | SsiOp::Param { .. }
                | SsiOp::Call { .. }
                | SsiOp::UnknownOp { .. } => {}
            }
        }
        visited
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_function_ir(
        &self,
        uri: &str,
        skeleton: &FileSkeleton,
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
                let val = self.eval_inst(
                    inst,
                    uri,
                    skeleton,
                    &values,
                    &guard_env,
                    adapter,
                    files,
                    file_ids,
                    workspace_root,
                    start,
                    if in_slice { &mut unknown_boundaries } else { &mut dummy_bounds },
                    if in_slice { &mut pending_lsp_items } else { &mut dummy_lsp },
                    if in_slice { &mut unknown_external_generic_calls } else { &mut dummy_unk },
                    if in_slice { &mut total_external_generic_calls } else { &mut dummy_tot },
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

        let target_absence = values
            .get(&target_vid)
            .copied()
            .unwrap_or(AbsenceValue::present());

        let local_risks: Vec<HoverRisk> = fn_ir
            .danger_sinks
            .iter()
            .filter(|sink| sink.target == target_vid || reachable.contains(&sink.target))
            .filter_map(|sink| {
                let sink_val = values.get(&sink.target).copied().unwrap_or_default();
                if sink_val.is_maybe_absent() || sink_val.is_definitely_absent() {
                    Some(HoverRisk {
                        file_id: 0,
                        uri: uri.to_string(),
                        range: sink.range,
                        kind: format!("{:?}", sink.kind),
                        message: format!("Unguarded {} on potentially absent value", sink.label),
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
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_inst(
        &self,
        inst: &SsiInst,
        uri: &str,
        skeleton: &FileSkeleton,
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
            SsiOp::Call { callee, .. } => match callee {
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
                        if let Ok((_skel, text)) = self.get_or_parse_skeleton(uri, adapter) {
                            let callee_ir = self.get_or_lower_function(uri, &text, fskel, adapter);
                            return callee_ir.local_summary.return_absence;
                        }
                    }
                    AbsenceValue::present()
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
                            let target_uri = format!("file://{}", target_path.to_string_lossy());
                            if !file_ids.contains_key(&target_uri) {
                                let fid = files.len() as u32;
                                files.push(FileEntry {
                                    file_id: fid,
                                    uri: target_uri.clone(),
                                });
                                file_ids.insert(target_uri.clone(), fid);
                            }
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
                                            fskel,
                                            target_adapter,
                                        );
                                        return target_ir.local_summary.return_absence;
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

                    *unknown_ext_calls += 1;
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
                        reason: UnknownReason::ExternalUntyped,
                    });
                    AbsenceValue::unknown(UnknownReason::ExternalUntyped)
                }
                CallTarget::Dynamic(sym) => {
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
            SsiOp::Call { callee, .. } => format!("call {callee:?}"),
            SsiOp::UnknownOp { symbol, .. } => format!("unverified {symbol}"),
            _ => inst.name.clone().unwrap_or_else(|| "op".to_string()),
        };

        let is_copy = matches!(inst.op, SsiOp::Assign { .. });
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
                    });
                }
            }
            _ => {}
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
            },
            truncation: Vec::new(),
            stats: SliceStats {
                phase1_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            },
        }
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
            let ir = self.get_or_lower_function(uri, &text, f, adapter);
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
            let ir = self.get_or_lower_function(uri, &text, f, adapter);
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
}
