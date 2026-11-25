use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::time::{Duration, Instant};

use crate::protocol::schema::{
    AbsenceSummary, CanonicalRange, EdgeSide, FlowEdge, FlowNode, NodeKind, SliceGraph,
    TruncationReason,
};
use crate::slicer::scc::tarjan_scc;

pub const MAX_DEPTH: u32 = 5;
pub const MAX_NODES: usize = 150;
pub const MAX_FANOUT: usize = 12;

#[derive(Debug, Clone)]
pub struct RawSliceNode {
    pub id: String,
    pub file_id: u32,
    pub is_test_file: bool,
    pub is_copy_passthrough: bool,
    pub range: CanonicalRange,
    pub label: String,
    pub snippet: Option<String>,
    pub absence: AbsenceSummary,
    pub provenance: String,
    pub is_guard: bool,
    pub guard_text: Option<String>,
    pub is_danger_sink: bool,
    pub danger_kind: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RawSliceEdge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub is_back_edge: bool,
    pub cycle_group_id: Option<u32>,
    pub provenance: String,
    pub edge_side: Option<EdgeSide>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct QueueEntry {
    depth: u32,
    file_id: u32,
    line: u32,
    col: u32,
    node_id: String,
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap by (depth, file_id, line, col, node_id)
        other
            .depth
            .cmp(&self.depth)
            .then_with(|| other.file_id.cmp(&self.file_id))
            .then_with(|| other.line.cmp(&self.line))
            .then_with(|| other.col.cmp(&self.col))
            .then_with(|| other.node_id.cmp(&self.node_id))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct PipelineOutput {
    pub graph: SliceGraph,
    pub truncation: Vec<TruncationReason>,
    pub nodes_visited: usize,
    pub contracted_chains: usize,
}

/// Executes the 5-stage deterministic reduction pipeline:
/// 1. Pre-budget test filtering
/// 2. Zero-cost copy-chain contraction (`ContractedChain`)
/// 3. Deterministic best-first scheduling
/// 4. Orthogonal caps (`max_depth=5`, `max_nodes=150`, `max_fanout=12`, wall-clock budget)
/// 5. Pure post-slice view projections (handled by `projectors.rs`)
pub fn run_reduction_pipeline(
    root_ids: &[String],
    raw_nodes: &[RawSliceNode],
    raw_edges: &[RawSliceEdge],
    exclude_tests: bool,
    budget: Duration,
    start_time: Instant,
) -> PipelineOutput {
    let mut truncation = Vec::new();

    // Stage 1: Pre-budget test filtering
    let node_map: std::collections::HashMap<&str, &RawSliceNode> = raw_nodes
        .iter()
        .filter(|n| !(exclude_tests && n.is_test_file))
        .map(|n| (n.id.as_str(), n))
        .collect();

    let mut adj: std::collections::HashMap<&str, Vec<&RawSliceEdge>> =
        std::collections::HashMap::new();
    for edge in raw_edges {
        if node_map.contains_key(edge.from.as_str()) && node_map.contains_key(edge.to.as_str()) {
            adj.entry(edge.from.as_str()).or_default().push(edge);
        }
    }
    // Sort outgoing edges deterministically by target (file_id, line, col, id)
    for edges in adj.values_mut() {
        edges.sort_by(|a, b| {
            let na = node_map[a.to.as_str()];
            let nb = node_map[b.to.as_str()];
            na.file_id
                .cmp(&nb.file_id)
                .then_with(|| na.range.start.cmp(&nb.range.start))
                .then_with(|| na.id.cmp(&nb.id))
        });
    }

    // Stage 3 & 4: Deterministic best-first scheduling + orthogonal caps
    let mut heap = BinaryHeap::new();
    for root in root_ids {
        if let Some(n) = node_map.get(root.as_str()) {
            heap.push(QueueEntry {
                depth: 0,
                file_id: n.file_id,
                line: n.range.start.line,
                col: n.range.start.col,
                node_id: n.id.clone(),
            });
        }
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut emitted_nodes: Vec<FlowNode> = Vec::new();
    let mut emitted_edges: Vec<FlowEdge> = Vec::new();
    let mut nodes_visited = 0usize;
    let mut contracted_chains = 0usize;

    while let Some(entry) = heap.pop() {
        if !visited.insert(entry.node_id.clone()) {
            continue;
        }
        nodes_visited += 1;

        if start_time.elapsed() >= budget {
            push_truncation(&mut truncation, TruncationReason::WallClockBudget);
            if let Some(raw) = node_map.get(entry.node_id.as_str()) {
                emitted_nodes.push(to_flow_node(raw, NodeKind::FrontierNode));
            }
            break;
        }

        if emitted_nodes.len() >= MAX_NODES {
            push_truncation(&mut truncation, TruncationReason::MaxNodes);
            break;
        }

        let Some(raw) = node_map.get(entry.node_id.as_str()) else {
            continue;
        };

        // Stage 2: Zero-cost copy-chain contraction
        let outgoing = adj
            .get(raw.id.as_str())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        if raw.is_copy_passthrough
            && !raw.is_guard
            && !raw.is_danger_sink
            && outgoing.len() == 1
            && entry.depth > 0
        {
            contracted_chains += 1;
            let edge = outgoing[0];
            emitted_nodes.push(to_flow_node(raw, NodeKind::ContractedChain));
            emitted_edges.push(FlowEdge {
                from: edge.from.clone(),
                to: edge.to.clone(),
                label: edge.label.clone(),
                is_back_edge: edge.is_back_edge,
                cycle_group_id: edge.cycle_group_id,
                provenance: edge.provenance.clone(),
                edge_side: edge.edge_side,
            });
            if let Some(next_node) = node_map.get(edge.to.as_str()) {
                if !visited.contains(next_node.id.as_str()) {
                    heap.push(QueueEntry {
                        depth: entry.depth, // zero-cost: does not increment depth
                        file_id: next_node.file_id,
                        line: next_node.range.start.line,
                        col: next_node.range.start.col,
                        node_id: next_node.id.clone(),
                    });
                }
            }
            continue;
        }

        if entry.depth >= MAX_DEPTH && !outgoing.is_empty() {
            push_truncation(&mut truncation, TruncationReason::MaxDepth);
            emitted_nodes.push(to_flow_node(raw, NodeKind::FrontierNode));
            continue;
        }

        emitted_nodes.push(to_flow_node(raw, NodeKind::SourceNode));

        for (fanout_count, edge) in outgoing.iter().enumerate() {
            if fanout_count >= MAX_FANOUT {
                push_truncation(&mut truncation, TruncationReason::MaxFanout);
                break;
            }
            emitted_edges.push(FlowEdge {
                from: edge.from.clone(),
                to: edge.to.clone(),
                label: edge.label.clone(),
                is_back_edge: edge.is_back_edge,
                cycle_group_id: edge.cycle_group_id,
                provenance: edge.provenance.clone(),
                edge_side: edge.edge_side,
            });
            if !edge.is_back_edge && !visited.contains(edge.to.as_str()) {
                if let Some(next_node) = node_map.get(edge.to.as_str()) {
                    heap.push(QueueEntry {
                        depth: entry.depth + 1,
                        file_id: next_node.file_id,
                        line: next_node.range.start.line,
                        col: next_node.range.start.col,
                        node_id: next_node.id.clone(),
                    });
                }
            }
        }
    }

    annotate_cycles(root_ids, &emitted_nodes, &mut emitted_edges);

    PipelineOutput {
        graph: SliceGraph {
            nodes: emitted_nodes,
            edges: emitted_edges,
        },
        truncation,
        nodes_visited,
        contracted_chains,
    }
}

fn annotate_cycles(root_ids: &[String], nodes: &[FlowNode], edges: &mut [FlowEdge]) {
    let n = nodes.len();
    if n == 0 || edges.is_empty() {
        return;
    }
    let node_idx: std::collections::HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_str(), i))
        .collect();

    let mut scc_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut dfs_adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (eidx, edge) in edges.iter().enumerate() {
        if let (Some(&u), Some(&v)) = (
            node_idx.get(edge.from.as_str()),
            node_idx.get(edge.to.as_str()),
        ) {
            scc_adj[u].push(v);
            dfs_adj[u].push((v, eidx));
        }
    }

    let scc_res = tarjan_scc(&scc_adj);

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    fn dfs_back_edges(
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
                    dfs_back_edges(v, adj, color, back_flags);
                }
                Color::Black => {}
            }
        }
        color[u] = Color::Black;
    }

    let mut color = vec![Color::White; n];
    let mut back_flags = vec![false; edges.len()];
    for root in root_ids {
        if let Some(&ridx) = node_idx.get(root.as_str()) {
            if color[ridx] == Color::White {
                dfs_back_edges(ridx, &dfs_adj, &mut color, &mut back_flags);
            }
        }
    }
    for u in 0..n {
        if color[u] == Color::White {
            dfs_back_edges(u, &dfs_adj, &mut color, &mut back_flags);
        }
    }

    for (eidx, edge) in edges.iter_mut().enumerate() {
        if let (Some(&u), Some(&v)) = (
            node_idx.get(edge.from.as_str()),
            node_idx.get(edge.to.as_str()),
        ) {
            let scc_u = scc_res.node_to_scc[u];
            let scc_v = scc_res.node_to_scc[v];
            if scc_u == scc_v && (scc_res.sccs[scc_u].len() > 1 || u == v) {
                edge.cycle_group_id = Some(scc_u as u32);
            } else {
                edge.cycle_group_id = None;
            }
            edge.is_back_edge = edge.is_back_edge || back_flags[eidx];
        }
    }
}

fn to_flow_node(raw: &RawSliceNode, kind: NodeKind) -> FlowNode {
    FlowNode {
        id: raw.id.clone(),
        kind,
        file_id: raw.file_id,
        range: raw.range,
        label: raw.label.clone(),
        snippet: raw.snippet.clone(),
        absence: raw.absence.clone(),
        provenance: raw.provenance.clone(),
        is_guard: raw.is_guard,
        guard_text: raw.guard_text.clone(),
        is_danger_sink: raw.is_danger_sink,
        danger_kind: raw.danger_kind.clone(),
    }
}

fn push_truncation(list: &mut Vec<TruncationReason>, reason: TruncationReason) {
    if !list.contains(&reason) {
        list.push(reason);
    }
}
