use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use crate::export::sanitize::PathSanitizer;
use crate::export::ExportOptions;
use crate::protocol::schema::{
    CanonicalRange, EdgeSide, ExportSourceMode, NodeKind, Phase2Status, SliceDirection,
    SliceEnvelope, TruncationReason, UnknownReason,
};
use crate::slicer::scc::tarjan_scc;

pub const DEFAULT_MAX_NODES: usize = 200;
pub const DEFAULT_MAX_EDGES: usize = 800;
pub const MAX_LABEL_CHARS: usize = 256;
pub const MAX_SNIPPET_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodePartition {
    Upstream,
    Focal,
    Downstream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AbsenceClass {
    NonNull,
    Nullable,
    Absent,
}

#[derive(Debug, Clone)]
pub struct ExportFile {
    pub file_id: u32,
    pub path: String,
    pub is_external: bool,
}

#[derive(Debug, Clone)]
pub struct ExportNode {
    pub id: String,
    pub raw_id: String,
    pub kind: NodeKind,
    pub partition: NodePartition,
    pub file_index: usize,
    pub norm_path: String,
    pub range: CanonicalRange,
    pub label: String,
    pub snippet: Option<String>,
    pub absence_class: AbsenceClass,
    pub maybe_null: bool,
    pub maybe_undefined: bool,
    pub is_unverified: bool,
    pub unknown_reasons: Vec<UnknownReason>,
    pub is_guard: bool,
    pub guard_text: Option<String>,
    pub is_merge: bool,
    pub is_danger_sink: bool,
    pub danger_kind: Option<String>,
    pub risk_messages: Vec<String>,
    pub is_frontier: bool,
    pub is_root: bool,
    pub in_cycle: bool,
    pub provenance: String,
}

#[derive(Debug, Clone)]
pub struct ExportEdge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub is_back_edge: bool,
    pub cycle_group_id: Option<u32>,
    pub provenance: String,
}

#[derive(Debug, Clone)]
pub struct ExportUnknownBoundary {
    pub file_index: usize,
    pub norm_path: String,
    pub range: CanonicalRange,
    pub symbol: String,
    pub reason: UnknownReason,
}

#[derive(Debug, Clone)]
pub struct ExportCompleteness {
    pub source_mode: ExportSourceMode,
    pub phase2_status: Phase2Status,
    pub phase2_wait_ms: u64,
    pub badges: Vec<String>,
    pub truncation: Vec<TruncationReason>,
}

#[derive(Debug, Clone)]
pub struct ExportGraph {
    pub direction: SliceDirection,
    pub root_ids: Vec<String>,
    pub files: Vec<ExportFile>,
    pub nodes: Vec<ExportNode>,
    pub edges: Vec<ExportEdge>,
    pub unknown_boundaries: Vec<ExportUnknownBoundary>,
    pub completeness: ExportCompleteness,
    pub null_only: bool,
    pub exclude_tests: bool,
    pub truncated_nodes: usize,
    pub truncated_edges: usize,
}

#[derive(Debug, Clone)]
struct RawCausalEdge {
    from: String,
    to: String,
    labels: BTreeSet<String>,
    provenances: BTreeSet<String>,
    side: EdgeSide,
}

impl ExportGraph {
    pub fn build(envelope: &SliceEnvelope, opts: &ExportOptions) -> Self {
        let sanitizer = PathSanitizer::new(opts.workspace_root.as_deref(), opts.absolute_paths);

        // Build deterministic file table keyed by file_id.
        let mut raw_files: BTreeMap<u32, ExportFile> = BTreeMap::new();
        for f in &envelope.files {
            let san = sanitizer.sanitize_uri(&f.uri);
            raw_files.insert(
                f.file_id,
                ExportFile {
                    file_id: f.file_id,
                    path: san.display,
                    is_external: san.is_external,
                },
            );
        }

        let mut file_id_to_index: BTreeMap<u32, usize> = BTreeMap::new();
        let mut files: Vec<ExportFile> = Vec::with_capacity(raw_files.len());
        for (fid, ef) in raw_files {
            file_id_to_index.insert(fid, files.len());
            files.push(ef);
        }

        let ensure_file_index = |fid: u32,
                                 fallback_uri: Option<&str>,
                                 files: &mut Vec<ExportFile>,
                                 map: &mut BTreeMap<u32, usize>|
         -> usize {
            if let Some(&idx) = map.get(&fid) {
                return idx;
            }
            let san = fallback_uri
                .map(|u| sanitizer.sanitize_uri(u))
                .unwrap_or_else(|| sanitizer.sanitize_uri("<unknown>"));
            let idx = files.len();
            files.push(ExportFile {
                file_id: fid,
                path: san.display,
                is_external: san.is_external,
            });
            map.insert(fid, idx);
            idx
        };

        let raw_graph = envelope.graph.as_ref();
        let mut raw_nodes_by_id: BTreeMap<String, &crate::protocol::schema::FlowNode> =
            BTreeMap::new();
        if let Some(g) = raw_graph {
            for n in &g.nodes {
                raw_nodes_by_id.insert(n.id.clone(), n);
                ensure_file_index(n.file_id, None, &mut files, &mut file_id_to_index);
            }
        }

        // Normalize causal edge orientation (origin -> consumer) and merge parallel (from, to) edges.
        let mut merged_edges: BTreeMap<(String, String), RawCausalEdge> = BTreeMap::new();
        if let Some(g) = raw_graph {
            for e in &g.edges {
                if !raw_nodes_by_id.contains_key(&e.from) || !raw_nodes_by_id.contains_key(&e.to) {
                    continue;
                }
                let side = resolve_edge_side(envelope.direction, e);
                let (from, to) = match side {
                    EdgeSide::Upstream => (e.to.clone(), e.from.clone()),
                    EdgeSide::Downstream => (e.from.clone(), e.to.clone()),
                };
                let entry = merged_edges
                    .entry((from.clone(), to.clone()))
                    .or_insert_with(|| RawCausalEdge {
                        from,
                        to,
                        labels: BTreeSet::new(),
                        provenances: BTreeSet::new(),
                        side,
                    });
                if let Some(lbl) = e.label.as_deref() {
                    let trimmed = lbl.trim();
                    if !trimmed.is_empty() {
                        entry.labels.insert(sanitizer.scrub_text(trimmed));
                    }
                }
                if !e.provenance.trim().is_empty() {
                    entry.provenances.insert(e.provenance.clone());
                }
            }
        }

        let root_set: BTreeSet<String> = envelope.root_ids.iter().cloned().collect();

        // Determine NodePartition (Upstream | Focal | Downstream).
        let mut partitions: BTreeMap<String, NodePartition> = BTreeMap::new();
        for id in raw_nodes_by_id.keys() {
            let part = if root_set.contains(id) {
                NodePartition::Focal
            } else {
                match envelope.direction {
                    SliceDirection::To => NodePartition::Upstream,
                    SliceDirection::From => NodePartition::Downstream,
                    SliceDirection::Both => NodePartition::Upstream,
                }
            };
            partitions.insert(id.clone(), part);
        }

        if envelope.direction == SliceDirection::Both {
            let mut downstream_adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for edge in merged_edges.values() {
                if edge.side == EdgeSide::Downstream {
                    downstream_adj
                        .entry(edge.from.as_str())
                        .or_default()
                        .push(edge.to.as_str());
                }
            }
            let mut q: VecDeque<String> = VecDeque::new();
            let mut seen_down: BTreeSet<String> = BTreeSet::new();
            for r in &root_set {
                if seen_down.insert(r.clone()) {
                    q.push_back(r.clone());
                }
            }
            while let Some(cur) = q.pop_front() {
                if let Some(nexts) = downstream_adj.get(cur.as_str()) {
                    for &nxt in nexts {
                        if seen_down.insert(nxt.to_string()) {
                            if !root_set.contains(nxt) {
                                partitions.insert(nxt.to_string(), NodePartition::Downstream);
                            }
                            q.push_back(nxt.to_string());
                        }
                    }
                }
            }
        }

        // Null-only causal path preservation.
        let mut allowed_ids: BTreeSet<String> = raw_nodes_by_id.keys().cloned().collect();
        if opts.null_only {
            let mut fwd: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            let mut rev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for edge in merged_edges.values() {
                fwd.entry(edge.from.as_str())
                    .or_default()
                    .push(edge.to.as_str());
                rev.entry(edge.to.as_str())
                    .or_default()
                    .push(edge.from.as_str());
            }

            let mut null_keep: BTreeSet<String> = BTreeSet::new();
            let mut fwd_q: VecDeque<String> = VecDeque::new();
            let mut rev_q: VecDeque<String> = VecDeque::new();
            for (id, node) in &raw_nodes_by_id {
                if (node.absence.maybe_null || node.absence.maybe_undefined)
                    && null_keep.insert(id.clone())
                {
                    fwd_q.push_back(id.clone());
                    rev_q.push_back(id.clone());
                }
            }
            let mut seen_fwd = null_keep.clone();
            while let Some(cur) = fwd_q.pop_front() {
                if let Some(nexts) = fwd.get(cur.as_str()) {
                    for &nxt in nexts {
                        if seen_fwd.insert(nxt.to_string()) {
                            null_keep.insert(nxt.to_string());
                            fwd_q.push_back(nxt.to_string());
                        }
                    }
                }
            }
            let mut seen_rev = null_keep.clone();
            while let Some(cur) = rev_q.pop_front() {
                if let Some(prevs) = rev.get(cur.as_str()) {
                    for &prv in prevs {
                        if seen_rev.insert(prv.to_string()) {
                            null_keep.insert(prv.to_string());
                            rev_q.push_back(prv.to_string());
                        }
                    }
                }
            }
            allowed_ids = null_keep;
        }

        // Collect HoverRisk messages indexed by node_id or (file_id, range).
        let mut risks_by_node: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut risks_by_loc: BTreeMap<(u32, CanonicalRange), Vec<String>> = BTreeMap::new();
        if let Some(card) = &envelope.projections.hover_card {
            for r in &card.local_risks {
                let scrubbed = sanitizer.scrub_text(&r.message);
                if let Some(nid) = &r.node_id {
                    risks_by_node.entry(nid.clone()).or_default().push(scrubbed);
                } else {
                    risks_by_loc
                        .entry((r.file_id, r.range))
                        .or_default()
                        .push(scrubbed);
                }
            }
        }

        // Build surviving ExportNode drafts.
        let mut draft_nodes: BTreeMap<String, ExportNode> = BTreeMap::new();
        for id in &allowed_ids {
            let Some(&node) = raw_nodes_by_id.get(id) else {
                continue;
            };
            let file_index = file_id_to_index[&node.file_id];
            let norm_path = files[file_index].path.clone();
            let absence_class = if node.absence.maybe_null && node.absence.maybe_undefined {
                AbsenceClass::Absent
            } else if node.absence.maybe_null || node.absence.maybe_undefined {
                if !node.absence.present {
                    AbsenceClass::Absent
                } else {
                    AbsenceClass::Nullable
                }
            } else {
                AbsenceClass::NonNull
            };
            let mut unknown_reasons = node.absence.unknown_reasons.clone();
            unknown_reasons.sort();
            unknown_reasons.dedup();
            let is_unverified = !unknown_reasons.is_empty()
                || (!node.absence.present && absence_class == AbsenceClass::NonNull);

            let raw_scrubbed = collapse_whitespace(&sanitizer.scrub_text(&node.label));
            let is_merge = raw_scrubbed == "phi"
                || raw_scrubbed.starts_with("phi ")
                || raw_scrubbed == "merge"
                || raw_scrubbed.starts_with("merge ");
            let label = cap_chars(&humanize_label(&raw_scrubbed), opts.max_label_chars);
            let snippet = node
                .snippet
                .as_deref()
                .map(|s| normalize_snippet(&sanitizer.scrub_text(s)));
            let guard_text = node
                .guard_text
                .as_deref()
                .map(|g| collapse_whitespace(&sanitizer.scrub_text(g)));

            let mut risk_messages = risks_by_node.get(id).cloned().unwrap_or_default();
            if let Some(loc_msgs) = risks_by_loc.get(&(node.file_id, node.range)) {
                risk_messages.extend(loc_msgs.iter().cloned());
            }
            risk_messages.sort();
            risk_messages.dedup();

            let is_danger_sink = node.is_danger_sink || !risk_messages.is_empty();
            let is_frontier = matches!(node.kind, NodeKind::FrontierNode | NodeKind::ClusterNode);
            let is_root = root_set.contains(id);

            draft_nodes.insert(
                id.clone(),
                ExportNode {
                    id: id.clone(),
                    raw_id: node.id.clone(),
                    kind: node.kind,
                    partition: partitions
                        .get(id)
                        .copied()
                        .unwrap_or(NodePartition::Upstream),
                    file_index,
                    norm_path,
                    range: node.range,
                    label,
                    snippet,
                    absence_class,
                    maybe_null: node.absence.maybe_null,
                    maybe_undefined: node.absence.maybe_undefined,
                    is_unverified,
                    unknown_reasons,
                    is_guard: node.is_guard,
                    guard_text,
                    is_merge,
                    is_danger_sink,
                    danger_kind: node.danger_kind.clone(),
                    risk_messages,
                    is_frontier,
                    is_root,
                    in_cycle: false,
                    provenance: node.provenance.clone(),
                },
            );
        }

        let mut draft_edges: Vec<RawCausalEdge> = merged_edges
            .into_values()
            .filter(|e| draft_nodes.contains_key(&e.from) && draft_nodes.contains_key(&e.to))
            .collect();

        // Deterministic BFS priority pruning for max_nodes.
        let max_nodes = opts.max_nodes.min(DEFAULT_MAX_NODES);
        let max_edges = opts.max_edges.min(DEFAULT_MAX_EDGES);
        let total_nodes_before = draft_nodes.len();
        let mut truncated_nodes = 0usize;
        if draft_nodes.len() > max_nodes {
            let kept = bfs_prune_nodes(&draft_nodes, &draft_edges, &envelope.root_ids, max_nodes);
            truncated_nodes = total_nodes_before.saturating_sub(kept.len());
            draft_nodes.retain(|id, _| kept.contains(id));
            draft_edges
                .retain(|e| draft_nodes.contains_key(&e.from) && draft_nodes.contains_key(&e.to));
        }

        // Canonical sort surviving nodes & assign safe N0..Nk IDs.
        let mut ordered_nodes: Vec<ExportNode> = draft_nodes.into_values().collect();
        ordered_nodes.sort_by(cmp_export_node);

        let mut raw_to_safe: BTreeMap<String, String> = BTreeMap::new();
        let mut raw_to_idx: BTreeMap<String, usize> = BTreeMap::new();
        for (idx, node) in ordered_nodes.iter_mut().enumerate() {
            let safe_id = format!("N{idx}");
            raw_to_safe.insert(node.raw_id.clone(), safe_id.clone());
            raw_to_idx.insert(node.raw_id.clone(), idx);
            node.id = safe_id;
        }

        let mut root_ids: Vec<String> = envelope
            .root_ids
            .iter()
            .filter_map(|r| raw_to_safe.get(r).cloned())
            .collect();
        root_ids.sort();
        root_ids.dedup();

        // Cap edges deterministically by priority (root-incident, absent/nullable, back-edge, canonical).
        let total_edges_before = draft_edges.len();
        let mut truncated_edges = 0usize;
        if draft_edges.len() > max_edges {
            draft_edges.sort_by(|a, b| {
                let pa = edge_priority(a, &raw_to_idx, &ordered_nodes);
                let pb = edge_priority(b, &raw_to_idx, &ordered_nodes);
                pa.cmp(&pb)
                    .then_with(|| raw_to_idx[&a.from].cmp(&raw_to_idx[&b.from]))
                    .then_with(|| raw_to_idx[&a.to].cmp(&raw_to_idx[&b.to]))
            });
            draft_edges.truncate(max_edges);
            truncated_edges = total_edges_before.saturating_sub(draft_edges.len());
        }

        // Remap edges to N0..Nk and sort canonically by (from_idx, to_idx).
        draft_edges.sort_by(|a, b| {
            raw_to_idx[&a.from]
                .cmp(&raw_to_idx[&b.from])
                .then_with(|| raw_to_idx[&a.to].cmp(&raw_to_idx[&b.to]))
        });

        let mut edges: Vec<ExportEdge> = draft_edges
            .iter()
            .map(|e| ExportEdge {
                from: raw_to_safe[&e.from].clone(),
                to: raw_to_safe[&e.to].clone(),
                label: if e.labels.is_empty() {
                    None
                } else {
                    Some(e.labels.iter().cloned().collect::<Vec<_>>().join(", "))
                },
                is_back_edge: false,
                cycle_group_id: None,
                provenance: e.provenances.iter().cloned().collect::<Vec<_>>().join(","),
            })
            .collect();

        // Compute SCC cycle_group_id and deterministic 3-color DFS feedback is_back_edge.
        let n = ordered_nodes.len();
        if n > 0 && !edges.is_empty() {
            let mut scc_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
            let mut dfs_adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
            for (eidx, e) in draft_edges.iter().enumerate() {
                let u = raw_to_idx[&e.from];
                let v = raw_to_idx[&e.to];
                scc_adj[u].push(v);
                dfs_adj[u].push((v, eidx));
            }
            let scc_res = tarjan_scc(&scc_adj);

            #[derive(Clone, Copy, PartialEq, Eq)]
            enum Color {
                White,
                Gray,
                Black,
            }

            fn dfs(
                u: usize,
                adj: &[Vec<(usize, usize)>],
                color: &mut [Color],
                back_flags: &mut [bool],
            ) {
                color[u] = Color::Gray;
                for &(v, eidx) in &adj[u] {
                    match color[v] {
                        Color::Gray => {
                            back_flags[eidx] = true;
                        }
                        Color::White => {
                            dfs(v, adj, color, back_flags);
                        }
                        Color::Black => {}
                    }
                }
                color[u] = Color::Black;
            }

            let mut color = vec![Color::White; n];
            let mut back_flags = vec![false; edges.len()];
            for r in &envelope.root_ids {
                if let Some(&ridx) = raw_to_idx.get(r) {
                    if color[ridx] == Color::White {
                        dfs(ridx, &dfs_adj, &mut color, &mut back_flags);
                    }
                }
            }
            for u in 0..n {
                if color[u] == Color::White {
                    dfs(u, &dfs_adj, &mut color, &mut back_flags);
                }
            }

            for (eidx, edge) in edges.iter_mut().enumerate() {
                let u = raw_to_idx[&draft_edges[eidx].from];
                let v = raw_to_idx[&draft_edges[eidx].to];
                let scc_u = scc_res.node_to_scc[u];
                let scc_v = scc_res.node_to_scc[v];
                if scc_u == scc_v && (scc_res.sccs[scc_u].len() > 1 || u == v) {
                    edge.cycle_group_id = Some(scc_u as u32);
                    ordered_nodes[u].in_cycle = true;
                    ordered_nodes[v].in_cycle = true;
                }
                edge.is_back_edge = back_flags[eidx];
            }
        }

        // Scrub and sort UnknownBoundaries.
        let mut unknown_boundaries: Vec<ExportUnknownBoundary> = envelope
            .projections
            .hover_card
            .as_ref()
            .map(|c| {
                c.unknown_boundaries
                    .iter()
                    .map(|b| {
                        let file_index = ensure_file_index(
                            b.file_id,
                            Some(&b.uri),
                            &mut files,
                            &mut file_id_to_index,
                        );
                        let norm_path = files[file_index].path.clone();
                        ExportUnknownBoundary {
                            file_index,
                            norm_path,
                            range: b.range,
                            symbol: sanitizer.scrub_text(&b.symbol),
                            reason: b.reason,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        unknown_boundaries.sort_by(|a, b| {
            a.norm_path
                .cmp(&b.norm_path)
                .then_with(|| a.range.start.cmp(&b.range.start))
                .then_with(|| a.symbol.cmp(&b.symbol))
                .then_with(|| a.reason.cmp(&b.reason))
        });

        let mut truncation = envelope.truncation.clone();
        if truncated_nodes > 0 && !truncation.contains(&TruncationReason::MaxNodes) {
            truncation.push(TruncationReason::MaxNodes);
        }
        if truncated_edges > 0 && !truncation.contains(&TruncationReason::MaxFanout) {
            truncation.push(TruncationReason::MaxFanout);
        }

        let mut badges = Vec::new();
        if !truncation.is_empty() || truncated_nodes > 0 || truncated_edges > 0 {
            badges.push("Budget-capped (partial)".to_string());
        }
        match opts.phase2_status {
            Phase2Status::Complete => {}
            Phase2Status::TimedOut => {
                if let Some(reason) = &opts.static_reason {
                    badges.push(reason.clone());
                } else {
                    badges.push(format!(
                        "Static only (LSP timed out after {}ms)",
                        opts.lsp_timeout_ms
                    ));
                }
            }
            Phase2Status::Skipped => {
                if let Some(reason) = &opts.static_reason {
                    badges.push(reason.clone());
                } else {
                    badges.push("Static only (--no-lsp)".to_string());
                }
            }
            Phase2Status::NotNeeded => {
                if let Some(reason) = &opts.static_reason {
                    badges.push(reason.clone());
                } else if !opts.lsp_attached && !unknown_boundaries.is_empty() {
                    badges.push("Static only (no LSP attached)".to_string());
                }
            }
        }

        ExportGraph {
            direction: envelope.direction,
            root_ids,
            files,
            nodes: ordered_nodes,
            edges,
            unknown_boundaries,
            completeness: ExportCompleteness {
                source_mode: opts.source_mode,
                phase2_status: opts.phase2_status,
                phase2_wait_ms: opts.phase2_wait_ms,
                badges,
                truncation,
            },
            null_only: opts.null_only,
            exclude_tests: opts.exclude_tests,
            truncated_nodes,
            truncated_edges,
        }
    }
}

fn resolve_edge_side(
    direction: SliceDirection,
    edge: &crate::protocol::schema::FlowEdge,
) -> EdgeSide {
    match direction {
        SliceDirection::To => EdgeSide::Upstream,
        SliceDirection::From => EdgeSide::Downstream,
        SliceDirection::Both => edge.edge_side.unwrap_or_else(|| {
            if edge.provenance == "ripgrep_caller"
                || edge.provenance == "lsp_call_hierarchy"
                || edge.label.as_deref() == Some("called_by")
            {
                EdgeSide::Downstream
            } else {
                EdgeSide::Upstream
            }
        }),
    }
}

fn node_kind_rank(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::SourceNode => 0,
        NodeKind::ContractedChain => 1,
        NodeKind::FrontierNode => 2,
        NodeKind::ClusterNode => 3,
    }
}

fn cmp_export_node(a: &ExportNode, b: &ExportNode) -> Ordering {
    a.norm_path
        .cmp(&b.norm_path)
        .then_with(|| a.range.start.line.cmp(&b.range.start.line))
        .then_with(|| a.range.start.col.cmp(&b.range.start.col))
        .then_with(|| node_kind_rank(a.kind).cmp(&node_kind_rank(b.kind)))
        .then_with(|| a.label.cmp(&b.label))
        .then_with(|| a.raw_id.cmp(&b.raw_id))
}

#[derive(Clone)]
struct BfsCandidate<'a> {
    depth: u32,
    tier: u8,
    node: &'a ExportNode,
}

impl<'a> Eq for BfsCandidate<'a> {}

impl<'a> PartialEq for BfsCandidate<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<'a> Ord for BfsCandidate<'a> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .depth
            .cmp(&self.depth)
            .then_with(|| other.tier.cmp(&self.tier))
            .then_with(|| cmp_export_node(other.node, self.node))
    }
}

impl<'a> PartialOrd for BfsCandidate<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn node_priority_tier(n: &ExportNode) -> u8 {
    if n.is_root {
        0
    } else if matches!(
        n.absence_class,
        AbsenceClass::Absent | AbsenceClass::Nullable
    ) || n.is_danger_sink
    {
        1
    } else if n.is_guard || n.is_unverified {
        2
    } else {
        3
    }
}

fn bfs_prune_nodes(
    nodes: &BTreeMap<String, ExportNode>,
    edges: &[RawCausalEdge],
    root_ids: &[String],
    max_nodes: usize,
) -> BTreeSet<String> {
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in edges {
        adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
        adj.entry(e.to.as_str()).or_default().push(e.from.as_str());
    }

    let mut heap: BinaryHeap<BfsCandidate<'_>> = BinaryHeap::new();
    let mut queued: BTreeSet<String> = BTreeSet::new();
    for r in root_ids {
        if let Some(n) = nodes.get(r) {
            if queued.insert(r.clone()) {
                heap.push(BfsCandidate {
                    depth: 0,
                    tier: node_priority_tier(n),
                    node: n,
                });
            }
        }
    }
    if heap.is_empty() {
        let mut all: Vec<&ExportNode> = nodes.values().collect();
        all.sort_by(|a, b| cmp_export_node(a, b));
        if let Some(first) = all.first() {
            queued.insert(first.raw_id.clone());
            heap.push(BfsCandidate {
                depth: 0,
                tier: node_priority_tier(first),
                node: first,
            });
        }
    }

    let mut kept: BTreeSet<String> = BTreeSet::new();
    while kept.len() < max_nodes {
        let Some(cand) = heap.pop() else {
            // Pull next remaining node in canonical order if disconnected components exist.
            let mut rem: Vec<&ExportNode> = nodes
                .values()
                .filter(|n| !queued.contains(&n.raw_id))
                .collect();
            if rem.is_empty() {
                break;
            }
            rem.sort_by(|a, b| {
                node_priority_tier(a)
                    .cmp(&node_priority_tier(b))
                    .then_with(|| cmp_export_node(a, b))
            });
            let next = rem[0];
            queued.insert(next.raw_id.clone());
            heap.push(BfsCandidate {
                depth: u32::MAX / 2,
                tier: node_priority_tier(next),
                node: next,
            });
            continue;
        };
        kept.insert(cand.node.raw_id.clone());
        if let Some(neighbors) = adj.get(cand.node.raw_id.as_str()) {
            for &nbr in neighbors {
                if let Some(nbr_node) = nodes.get(nbr) {
                    if queued.insert(nbr.to_string()) {
                        heap.push(BfsCandidate {
                            depth: cand.depth.saturating_add(1),
                            tier: node_priority_tier(nbr_node),
                            node: nbr_node,
                        });
                    }
                }
            }
        }
    }
    kept
}

fn edge_priority(
    edge: &RawCausalEdge,
    raw_to_idx: &BTreeMap<String, usize>,
    nodes: &[ExportNode],
) -> u8 {
    let from = &nodes[raw_to_idx[&edge.from]];
    let to = &nodes[raw_to_idx[&edge.to]];
    if from.is_root || to.is_root {
        0
    } else if matches!(
        from.absence_class,
        AbsenceClass::Absent | AbsenceClass::Nullable
    ) || matches!(
        to.absence_class,
        AbsenceClass::Absent | AbsenceClass::Nullable
    ) || from.is_danger_sink
        || to.is_danger_sink
    {
        1
    } else {
        2
    }
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn cap_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str("...");
    out
}

pub fn normalize_snippet(s: &str) -> String {
    let collapsed = collapse_whitespace(s);
    cap_chars(&collapsed, MAX_SNIPPET_CHARS)
}

/// Normalizes compiler-internal labels (`call Local("...")`, `phi x`, raw Tree-sitter kinds)
/// into concise, human-readable labels shared across exporters.
pub fn humanize_label(raw: &str) -> String {
    let s = raw.trim();
    if s == "phi" {
        return "merge".to_string();
    }
    if let Some(rest) = s.strip_prefix("phi ") {
        let rest = rest.trim();
        return if rest.is_empty() {
            "merge".to_string()
        } else {
            format!("merge {rest}")
        };
    }

    // Legacy Rust Debug CallTarget formatting (e.g. from --from-envelope JSONs)
    if let Some(rest) = s.strip_prefix("call ") {
        let rest = rest.trim();
        for prefix in ["Local(\"", "Stdlib(\"", "Dynamic(\""] {
            if let Some(inner) = rest.strip_prefix(prefix) {
                if let Some(sym) = inner.strip_suffix("\")") {
                    return if sym.ends_with("()") {
                        sym.to_string()
                    } else {
                        format!("{sym}()")
                    };
                }
            }
        }
        if rest.starts_with("Imported {") {
            if let Some(idx) = rest.find("symbol: \"") {
                let after = &rest[idx + "symbol: \"".len()..];
                if let Some(end_q) = after.find('"') {
                    let sym = &after[..end_q];
                    return if sym.ends_with("()") {
                        sym.to_string()
                    } else {
                        format!("{sym}()")
                    };
                }
            }
        }
    }

    match s {
        "binary_operator"
        | "binary_expression"
        | "unary_expression"
        | "unary_operator"
        | "comparison_operator"
        | "boolean_operator"
        | "parenthesized_expression"
        | "subscript_expression" => "expr".to_string(),
        "ternary_expression" | "conditional_expression" => "branch".to_string(),
        _ => s.to_string(),
    }
}
