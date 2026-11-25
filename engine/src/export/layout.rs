use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use crate::export::ir::ExportGraph;
use crate::export::ExportOptions;
use crate::protocol::schema::SliceDirection;

pub const CARD_W: i32 = 240;
pub const CARD_H: i32 = 72;
pub const H_GAP: i32 = 40;
pub const V_GAP: i32 = 56;
pub const DUMMY_W: i32 = 24;
pub const GUTTER_GAP: i32 = 28;
pub const LANE_PITCH: i32 = 14;
pub const CORRIDOR_STEP: i32 = 10;
pub const COMPONENT_GAP_Y: i32 = 72;
pub const ARROW_PULLBACK: i32 = 8;
pub const MARGIN: i32 = 32;
pub const MAX_DUMMIES: usize = 600;
pub const CROSSING_SWEEPS: usize = 6;
pub const SELF_LOOP_OFFSET: i32 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    Forward,
    CycleGutter,
    OverflowLeft,
    SelfLoop,
}

#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub rank: i32,
    pub topo_index: usize,
}

#[derive(Debug, Clone)]
pub struct LayoutEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub route_kind: RouteKind,
    pub waypoints: Vec<(i32, i32)>,
    pub is_back_edge: bool,
    pub cycle_group_id: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutMetrics {
    pub rank_count: usize,
    pub dummy_nodes_added: usize,
    pub dummy_cap_hit: bool,
    pub feedback_back_edges: usize,
    pub overflow_edges: usize,
    pub crossing_sweeps: usize,
    pub svg_width: i32,
    pub svg_height: i32,
}

#[derive(Debug, Clone)]
pub struct SvgLayout {
    pub view_width: i32,
    pub view_height: i32,
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
    pub focal_band_y: Option<i32>,
    pub empty_state: bool,
    pub trivial_single: bool,
    pub metrics: LayoutMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerItem {
    Real(usize),
    Dummy(usize),
}

impl LayerItem {
    fn width(self) -> i32 {
        match self {
            LayerItem::Real(_) => CARD_W,
            LayerItem::Dummy(_) => DUMMY_W,
        }
    }
}

#[derive(Debug, Clone)]
struct DummyNode {
    edge_idx: usize,
    rank: usize,
    step: usize,
    x: i32,
}

struct CompEdge {
    edge_idx: usize,
    u: usize,
    v: usize,
    is_back_edge: bool,
}

struct ComponentLayout {
    node_positions: BTreeMap<usize, (i32, i32, i32)>, // node_idx -> (x, y, rank)
    edge_routes: BTreeMap<usize, (RouteKind, Vec<(i32, i32)>)>,
    focal_band_y: Option<i32>,
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
    rank_count: usize,
    sweeps_used: usize,
}

/// Deterministic `O(k log k)` greedy interval lane packer shared by right cycle
/// gutters and left overflow long-span gutters.
pub fn pack_gutter_lanes(intervals: &[(usize, i32, i32, usize, usize)]) -> Vec<(usize, usize)> {
    // Input tuple: (edge_idx, y_top, y_bottom, from_idx, to_idx)
    let mut sorted = intervals.to_vec();
    sorted.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.4.cmp(&b.4))
            .then_with(|| a.0.cmp(&b.0))
    });

    // Heap holds Reverse((lane_end_y, lane_idx))
    let mut active_lanes: BinaryHeap<Reverse<(i32, usize)>> = BinaryHeap::new();
    let mut next_lane = 0usize;
    let mut out = Vec::with_capacity(sorted.len());

    for (edge_idx, y_top, y_bottom, _, _) in sorted {
        let lane = if let Some(&Reverse((end_y, lane_idx))) = active_lanes.peek() {
            if end_y < y_top {
                active_lanes.pop();
                active_lanes.push(Reverse((y_bottom, lane_idx)));
                lane_idx
            } else {
                let l = next_lane;
                next_lane += 1;
                active_lanes.push(Reverse((y_bottom, l)));
                l
            }
        } else {
            let l = next_lane;
            next_lane += 1;
            active_lanes.push(Reverse((y_bottom, l)));
            l
        };
        out.push((edge_idx, lane));
    }

    out
}

/// Computes pure integer `i32` SVG layout coordinates from Stage C.1's `ExportGraph`.
pub fn compute_layout(graph: &ExportGraph, _opts: &ExportOptions) -> SvgLayout {
    let n = graph.nodes.len();
    if n == 0 {
        return SvgLayout {
            view_width: 640,
            view_height: 240,
            nodes: Vec::new(),
            edges: Vec::new(),
            focal_band_y: None,
            empty_state: true,
            trivial_single: false,
            metrics: LayoutMetrics {
                svg_width: 640,
                svg_height: 240,
                ..LayoutMetrics::default()
            },
        };
    }

    let mut id_to_idx: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        id_to_idx.insert(node.id.as_str(), i);
    }

    if n == 1 {
        let cx = (640 - CARD_W) / 2;
        let cy = (240 - CARD_H) / 2;
        let node = &graph.nodes[0];
        let mut edges = Vec::with_capacity(graph.edges.len());
        for (eidx, e) in graph.edges.iter().enumerate() {
            let waypoints = if e.from == e.to {
                self_loop_waypoints(cx, cy)
            } else {
                Vec::new()
            };
            edges.push(LayoutEdge {
                id: format!("E{eidx}"),
                from: e.from.clone(),
                to: e.to.clone(),
                route_kind: RouteKind::SelfLoop,
                waypoints,
                is_back_edge: e.is_back_edge,
                cycle_group_id: e.cycle_group_id,
            });
        }
        return SvgLayout {
            view_width: 640,
            view_height: 240,
            nodes: vec![LayoutNode {
                id: node.id.clone(),
                x: cx,
                y: cy,
                w: CARD_W,
                h: CARD_H,
                rank: 0,
                topo_index: 0,
            }],
            edges,
            focal_band_y: None,
            empty_state: false,
            trivial_single: true,
            metrics: LayoutMetrics {
                rank_count: 1,
                svg_width: 640,
                svg_height: 240,
                ..LayoutMetrics::default()
            },
        };
    }

    // Partition weakly connected components deterministically.
    let mut undirected_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &graph.edges {
        if let (Some(&u), Some(&v)) = (id_to_idx.get(e.from.as_str()), id_to_idx.get(e.to.as_str()))
        {
            if u != v {
                undirected_adj[u].push(v);
                undirected_adj[v].push(u);
            }
        }
    }
    for adj in &mut undirected_adj {
        adj.sort_unstable();
        adj.dedup();
    }

    let mut visited = vec![false; n];
    let mut node_to_comp = vec![0usize; n];
    let mut components: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if visited[start] {
            continue;
        }
        let comp_id = components.len();
        let mut comp = Vec::new();
        let mut q = VecDeque::new();
        visited[start] = true;
        q.push_back(start);
        while let Some(u) = q.pop_front() {
            node_to_comp[u] = comp_id;
            comp.push(u);
            for &v in &undirected_adj[u] {
                if !visited[v] {
                    visited[v] = true;
                    q.push_back(v);
                }
            }
        }
        comp.sort_unstable();
        components.push(comp);
    }

    // Order primary root's component first, then remaining components by lowest node index.
    let primary_root_idx = graph
        .root_ids
        .first()
        .and_then(|rid| id_to_idx.get(rid.as_str()).copied());
    let primary_comp = primary_root_idx.map(|r| node_to_comp[r]);

    let mut comp_order: Vec<usize> = (0..components.len()).collect();
    comp_order.sort_by(|&a, &b| {
        let a_pri = primary_comp == Some(a);
        let b_pri = primary_comp == Some(b);
        b_pri
            .cmp(&a_pri)
            .then_with(|| components[a][0].cmp(&components[b][0]))
    });

    let root_set: BTreeSet<usize> = graph
        .root_ids
        .iter()
        .filter_map(|rid| id_to_idx.get(rid.as_str()).copied())
        .collect();

    let mut comp_edges: Vec<Vec<CompEdge>> = (0..components.len()).map(|_| Vec::new()).collect();
    for (eidx, e) in graph.edges.iter().enumerate() {
        if let (Some(&u), Some(&v)) = (id_to_idx.get(e.from.as_str()), id_to_idx.get(e.to.as_str()))
        {
            let cid = node_to_comp[u];
            comp_edges[cid].push(CompEdge {
                edge_idx: eidx,
                u,
                v,
                is_back_edge: e.is_back_edge,
            });
        }
    }

    let mut global_dummies = 0usize;
    let mut dummy_cap_hit = false;
    let mut feedback_back_edges = 0usize;
    let mut overflow_edges = 0usize;
    let mut total_ranks = 0usize;
    let mut max_sweeps = 0usize;

    let mut laid_components: Vec<ComponentLayout> = Vec::with_capacity(comp_order.len());
    for cid in comp_order {
        let comp = layout_component(
            &components[cid],
            &comp_edges[cid],
            &root_set,
            graph.direction,
            &mut global_dummies,
            &mut dummy_cap_hit,
            &mut feedback_back_edges,
            &mut overflow_edges,
        );
        total_ranks += comp.rank_count;
        max_sweeps = max_sweeps.max(comp.sweeps_used);
        laid_components.push(comp);
    }

    // Stack components vertically with COMPONENT_GAP_Y and shift so min_x >= MARGIN, min_y >= MARGIN.
    let mut final_nodes: Vec<Option<LayoutNode>> = vec![None; n];
    let mut final_edges: Vec<Option<LayoutEdge>> = vec![None; graph.edges.len()];
    let mut focal_band_y: Option<i32> = None;

    let mut cursor_y = MARGIN;
    let mut global_max_x = MARGIN;
    let mut global_max_y = MARGIN;
    let mut global_rank_offset = 0i32;

    for (c_order_idx, comp) in laid_components.into_iter().enumerate() {
        let dx = MARGIN - comp.min_x;
        let dy = cursor_y - comp.min_y;
        let comp_max_x = comp.max_x + dx;
        let comp_max_y = comp.max_y + dy;
        global_max_x = global_max_x.max(comp_max_x);
        global_max_y = global_max_y.max(comp_max_y);

        if c_order_idx == 0 {
            focal_band_y = comp.focal_band_y.map(|fy| fy + dy);
        }

        for (node_idx, (lx, ly, lrank)) in comp.node_positions {
            final_nodes[node_idx] = Some(LayoutNode {
                id: graph.nodes[node_idx].id.clone(),
                x: lx + dx,
                y: ly + dy,
                w: CARD_W,
                h: CARD_H,
                rank: global_rank_offset + lrank,
                topo_index: 0,
            });
        }

        for (eidx, (route_kind, wps)) in comp.edge_routes {
            let e = &graph.edges[eidx];
            let shifted_wps = wps.into_iter().map(|(wx, wy)| (wx + dx, wy + dy)).collect();
            final_edges[eidx] = Some(LayoutEdge {
                id: format!("E{eidx}"),
                from: e.from.clone(),
                to: e.to.clone(),
                route_kind,
                waypoints: shifted_wps,
                is_back_edge: e.is_back_edge,
                cycle_group_id: e.cycle_group_id,
            });
        }

        cursor_y = comp_max_y + COMPONENT_GAP_Y;
        global_rank_offset += comp.rank_count as i32;
    }

    let mut nodes: Vec<LayoutNode> = final_nodes
        .into_iter()
        .enumerate()
        .map(|(i, opt)| {
            opt.unwrap_or_else(|| LayoutNode {
                id: graph.nodes[i].id.clone(),
                x: MARGIN,
                y: MARGIN,
                w: CARD_W,
                h: CARD_H,
                rank: 0,
                topo_index: i,
            })
        })
        .collect();

    // Deterministic topological index across (rank, y, x, id).
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        nodes[a]
            .y
            .cmp(&nodes[b].y)
            .then_with(|| nodes[a].x.cmp(&nodes[b].x))
            .then_with(|| nodes[a].id.cmp(&nodes[b].id))
    });
    for (topo_idx, &node_idx) in order.iter().enumerate() {
        nodes[node_idx].topo_index = topo_idx;
    }

    let edges: Vec<LayoutEdge> = final_edges
        .into_iter()
        .enumerate()
        .map(|(eidx, opt)| {
            opt.unwrap_or_else(|| LayoutEdge {
                id: format!("E{eidx}"),
                from: graph.edges[eidx].from.clone(),
                to: graph.edges[eidx].to.clone(),
                route_kind: RouteKind::Forward,
                waypoints: Vec::new(),
                is_back_edge: graph.edges[eidx].is_back_edge,
                cycle_group_id: graph.edges[eidx].cycle_group_id,
            })
        })
        .collect();

    let view_width = (global_max_x + MARGIN).max(640);
    let view_height = (global_max_y + MARGIN).max(240);

    SvgLayout {
        view_width,
        view_height,
        nodes,
        edges,
        focal_band_y,
        empty_state: false,
        trivial_single: false,
        metrics: LayoutMetrics {
            rank_count: total_ranks,
            dummy_nodes_added: global_dummies,
            dummy_cap_hit,
            feedback_back_edges,
            overflow_edges,
            crossing_sweeps: max_sweeps,
            svg_width: view_width,
            svg_height: view_height,
        },
    }
}

fn self_loop_waypoints(x: i32, y: i32) -> Vec<(i32, i32)> {
    let rx = x + CARD_W;
    let y1 = y + 20;
    let y2 = y + CARD_H - 20;
    vec![
        (rx, y1),
        (rx + SELF_LOOP_OFFSET, y1),
        (rx + SELF_LOOP_OFFSET, y2),
        (rx + ARROW_PULLBACK, y2),
    ]
}

#[allow(clippy::too_many_arguments)]
fn layout_component(
    comp_nodes: &[usize],
    edges: &[CompEdge],
    root_set: &BTreeSet<usize>,
    direction: SliceDirection,
    global_dummies: &mut usize,
    dummy_cap_hit: &mut bool,
    feedback_back_edges: &mut usize,
    overflow_edges: &mut usize,
) -> ComponentLayout {
    let k = comp_nodes.len();
    let mut global_to_local: BTreeMap<usize, usize> = BTreeMap::new();
    for (local, &gidx) in comp_nodes.iter().enumerate() {
        global_to_local.insert(gidx, local);
    }

    // Stage 2: Split forward DAG edges vs. feedback back-edges vs. self-loops.
    let mut forward_edges: Vec<(usize, usize, usize)> = Vec::new(); // (edge_idx, u_local, v_local)
    let mut back_edges: Vec<(usize, usize, usize)> = Vec::new();
    let mut self_loop_edges: Vec<(usize, usize)> = Vec::new(); // (edge_idx, u_local)

    for e in edges {
        let u = global_to_local[&e.u];
        let v = global_to_local[&e.v];
        if u == v {
            self_loop_edges.push((e.edge_idx, u));
        } else if e.is_back_edge {
            back_edges.push((e.edge_idx, u, v));
            *feedback_back_edges += 1;
        } else {
            forward_edges.push((e.edge_idx, u, v));
        }
    }

    // Defensive DAG guarantee on forward_edges in case of synthetic cycles without is_back_edge.
    let (safe_forward, extra_back) = ensure_acyclic_forward(k, &forward_edges);
    for be in extra_back {
        back_edges.push(be);
        *feedback_back_edges += 1;
    }

    // Stage 3: Rank assignment (longest-path or Both waist pinning with fallback).
    let comp_roots: Vec<usize> = comp_nodes
        .iter()
        .enumerate()
        .filter_map(|(local, gidx)| root_set.contains(gidx).then_some(local))
        .collect();

    let (ranks, focal_rank_opt) = assign_ranks(k, &safe_forward, &comp_roots, direction);
    let num_ranks = ranks.iter().copied().max().unwrap_or(0) + 1;

    // Stage 4: Bounded dummy-node insertion (<= MAX_DUMMIES) + OverflowLeft classification.
    let mut multi_rank: Vec<(usize, usize, usize, usize)> = Vec::new(); // (span, from_global, to_global, pos_in_safe_forward)
    let mut direct_forward: Vec<(usize, usize, usize)> = Vec::new();
    for (pos, &(eidx, u, v)) in safe_forward.iter().enumerate() {
        let span = ranks[v] - ranks[u];
        if span >= 2 {
            multi_rank.push((span, comp_nodes[u], comp_nodes[v], pos));
        } else {
            direct_forward.push((eidx, u, v));
        }
    }
    multi_rank.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });

    let mut dummy_nodes: Vec<DummyNode> = Vec::new();
    // chain_dummies[pos_in_safe_forward] = list of dummy indices
    let mut edge_dummy_chains: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut overflow_left_edges: Vec<(usize, usize, usize)> = Vec::new();

    for (span, _, _, pos) in multi_rank {
        let (eidx, u, v) = safe_forward[pos];
        let needed = span - 1;
        if *global_dummies + needed <= MAX_DUMMIES {
            *global_dummies += needed;
            let mut chain = Vec::with_capacity(needed);
            for step in 1..span {
                let d_idx = dummy_nodes.len();
                dummy_nodes.push(DummyNode {
                    edge_idx: eidx,
                    rank: ranks[u] + step,
                    step,
                    x: 0,
                });
                chain.push(d_idx);
            }
            edge_dummy_chains.insert(pos, chain);
        } else {
            *dummy_cap_hit = true;
            *overflow_edges += 1;
            overflow_left_edges.push((eidx, u, v));
        }
    }

    // Build per-rank item lists and crossing-minimization adjacency across adjacent ranks.
    let mut rank_items: Vec<Vec<LayerItem>> = vec![Vec::new(); num_ranks];
    for (local, &r) in ranks.iter().enumerate() {
        rank_items[r].push(LayerItem::Real(local));
    }
    for (d_idx, dn) in dummy_nodes.iter().enumerate() {
        rank_items[dn.rank].push(LayerItem::Dummy(d_idx));
    }
    for items in &mut rank_items {
        items.sort_by_key(|item| match item {
            LayerItem::Real(l) => (0usize, *l, 0usize),
            LayerItem::Dummy(d) => (1usize, dummy_nodes[*d].edge_idx, dummy_nodes[*d].step),
        });
    }

    let mut fwd_pairs: Vec<(LayerItem, LayerItem)> = Vec::new();
    for &(_, u, v) in &direct_forward {
        fwd_pairs.push((LayerItem::Real(u), LayerItem::Real(v)));
    }
    for (pos, chain) in &edge_dummy_chains {
        let (_, u, v) = safe_forward[*pos];
        let mut prev = LayerItem::Real(u);
        for &d_idx in chain {
            let cur = LayerItem::Dummy(d_idx);
            fwd_pairs.push((prev, cur));
            prev = cur;
        }
        fwd_pairs.push((prev, LayerItem::Real(v)));
    }

    let mut preds: BTreeMap<(bool, usize), Vec<(bool, usize)>> = BTreeMap::new();
    let mut succs: BTreeMap<(bool, usize), Vec<(bool, usize)>> = BTreeMap::new();
    for &(a, b) in &fwd_pairs {
        let ka = item_key(a);
        let kb = item_key(b);
        succs.entry(ka).or_default().push(kb);
        preds.entry(kb).or_default().push(ka);
    }

    // Stage 5: 6 alternating barycenter sweeps (integer cross-multiplication).
    let sweeps_used = if num_ranks > 1 { CROSSING_SWEEPS } else { 0 };
    for sweep in 0..sweeps_used {
        if sweep % 2 == 0 {
            for r in 1..num_ranks {
                let mut prev_pos: BTreeMap<(bool, usize), usize> = BTreeMap::new();
                for (pos, &item) in rank_items[r - 1].iter().enumerate() {
                    prev_pos.insert(item_key(item), pos);
                }
                barycenter_sort(&mut rank_items[r], &preds, &prev_pos, &dummy_nodes);
            }
        } else {
            for r in (0..num_ranks.saturating_sub(1)).rev() {
                let mut next_pos: BTreeMap<(bool, usize), usize> = BTreeMap::new();
                for (pos, &item) in rank_items[r + 1].iter().enumerate() {
                    next_pos.insert(item_key(item), pos);
                }
                barycenter_sort(&mut rank_items[r], &succs, &next_pos, &dummy_nodes);
            }
        }
    }

    // 2-pass centered median-x coordinate assignment with strict H_GAP clamping.
    let mut item_x: BTreeMap<(bool, usize), i32> = BTreeMap::new();
    let mut widest_w = 0i32;
    let mut rank_widths = vec![0i32; num_ranks];
    for (r, items) in rank_items.iter().enumerate() {
        let mut x = 0i32;
        for (i, &item) in items.iter().enumerate() {
            if i > 0 {
                x += H_GAP;
            }
            item_x.insert(item_key(item), x);
            x += item.width();
        }
        rank_widths[r] = x;
        widest_w = widest_w.max(x);
    }
    for (r, items) in rank_items.iter().enumerate() {
        let shift = (widest_w - rank_widths[r]) / 2;
        if shift > 0 {
            for &item in items {
                if let Some(vx) = item_x.get_mut(&item_key(item)) {
                    *vx += shift;
                }
            }
        }
    }

    for pass in 0..2 {
        if pass == 0 {
            for items in rank_items.iter().skip(1) {
                relax_rank_x(items, &preds, &mut item_x);
            }
        } else {
            for items in rank_items.iter().take(num_ranks.saturating_sub(1)).rev() {
                relax_rank_x(items, &succs, &mut item_x);
            }
        }
    }

    let mut local_x = vec![0i32; k];
    for (key, &vx) in &item_x {
        if !key.0 {
            local_x[key.1] = vx;
        } else {
            dummy_nodes[key.1].x = vx;
        }
    }

    // Stage 6: Allocate horizontal corridor slots in inter-rank gaps before assigning rank_y!
    let mut exit_slots_per_rank: Vec<usize> = vec![0; num_ranks];
    let mut enter_slots_per_rank: Vec<usize> = vec![0; num_ranks];

    // Sort gutter edges deterministically for slot assignment.
    let mut right_gutter_edges = back_edges.clone();
    right_gutter_edges.sort_by(|a, b| {
        ranks[a.1]
            .cmp(&ranks[b.1])
            .then_with(|| local_x[a.1].cmp(&local_x[b.1]))
            .then_with(|| a.0.cmp(&b.0))
    });

    let mut left_gutter_edges = overflow_left_edges.clone();
    left_gutter_edges.sort_by(|a, b| {
        ranks[a.1]
            .cmp(&ranks[b.1])
            .then_with(|| local_x[a.1].cmp(&local_x[b.1]))
            .then_with(|| a.0.cmp(&b.0))
    });

    let mut edge_exit_slot: BTreeMap<usize, usize> = BTreeMap::new();
    let mut edge_enter_slot: BTreeMap<usize, usize> = BTreeMap::new();

    for &(eidx, u, v) in right_gutter_edges.iter().chain(left_gutter_edges.iter()) {
        exit_slots_per_rank[ranks[u]] += 1;
        edge_exit_slot.insert(eidx, exit_slots_per_rank[ranks[u]]);
        enter_slots_per_rank[ranks[v]] += 1;
        edge_enter_slot.insert(eidx, enter_slots_per_rank[ranks[v]]);
    }

    // Compute rank_y dynamically expanding inter-rank gaps for corridor tracks.
    let mut rank_y = vec![0i32; num_ranks];
    let top_reserve = if enter_slots_per_rank[0] > 0 {
        enter_slots_per_rank[0] as i32 * CORRIDOR_STEP + ARROW_PULLBACK + 8
    } else {
        0
    };
    rank_y[0] = top_reserve;
    for r in 0..num_ranks.saturating_sub(1) {
        let tracks = (exit_slots_per_rank[r] + enter_slots_per_rank[r + 1]) as i32;
        let needed_gap = if tracks > 0 {
            (tracks + 1) * CORRIDOR_STEP + ARROW_PULLBACK + 12
        } else {
            V_GAP
        };
        let gap = V_GAP.max(needed_gap);
        rank_y[r + 1] = rank_y[r] + CARD_H + gap;
    }

    // Compute exact corridor y coordinates per gutter edge.
    let mut edge_y_exit: BTreeMap<usize, i32> = BTreeMap::new();
    let mut edge_y_enter: BTreeMap<usize, i32> = BTreeMap::new();
    for &(eidx, u, v) in right_gutter_edges.iter().chain(left_gutter_edges.iter()) {
        let ru = ranks[u];
        let rv = ranks[v];
        let y_exit = rank_y[ru] + CARD_H + edge_exit_slot[&eidx] as i32 * CORRIDOR_STEP;
        let y_enter = rank_y[rv] - ARROW_PULLBACK - edge_enter_slot[&eidx] as i32 * CORRIDOR_STEP;
        edge_y_exit.insert(eidx, y_exit);
        edge_y_enter.insert(eidx, y_enter);
    }

    // Shared O(k log k) interval lane packing for right cycle gutter & left overflow gutter.
    let mut right_intervals = Vec::with_capacity(right_gutter_edges.len());
    for &(eidx, u, v) in &right_gutter_edges {
        let ye = edge_y_exit[&eidx];
        let yn = edge_y_enter[&eidx];
        right_intervals.push((eidx, ye.min(yn), ye.max(yn), comp_nodes[u], comp_nodes[v]));
    }
    let right_lanes: BTreeMap<usize, usize> =
        pack_gutter_lanes(&right_intervals).into_iter().collect();

    let mut left_intervals = Vec::with_capacity(left_gutter_edges.len());
    for &(eidx, u, v) in &left_gutter_edges {
        let ye = edge_y_exit[&eidx];
        let yn = edge_y_enter[&eidx];
        left_intervals.push((eidx, ye.min(yn), ye.max(yn), comp_nodes[u], comp_nodes[v]));
    }
    let left_lanes: BTreeMap<usize, usize> =
        pack_gutter_lanes(&left_intervals).into_iter().collect();

    // Compute bounds across all cards, dummies, and self-loops before placing gutters.
    let min_card_left = local_x.iter().copied().min().unwrap_or(0);
    let max_card_right = local_x.iter().map(|&x| x + CARD_W).max().unwrap_or(CARD_W);
    let max_self_loop_right = self_loop_edges
        .iter()
        .map(|&(_, u)| local_x[u] + CARD_W + SELF_LOOP_OFFSET)
        .max()
        .unwrap_or(max_card_right);
    let right_gutter_base = (max_card_right + GUTTER_GAP).max(max_self_loop_right + LANE_PITCH);
    let left_gutter_base = min_card_left - GUTTER_GAP;

    let mut edge_gutter_x: BTreeMap<usize, i32> = BTreeMap::new();
    for &(eidx, _, _) in &right_gutter_edges {
        let lane = right_lanes[&eidx] as i32;
        edge_gutter_x.insert(eidx, right_gutter_base + lane * LANE_PITCH);
    }
    for &(eidx, _, _) in &left_gutter_edges {
        let lane = left_lanes[&eidx] as i32;
        edge_gutter_x.insert(eidx, left_gutter_base - lane * LANE_PITCH);
    }

    // Stage 7: Distribute top and bottom card ports sorted by neighbor x.
    let mut out_ports: BTreeMap<(usize, usize), i32> = BTreeMap::new(); // (u_local, eidx) -> port_x
    let mut in_ports: BTreeMap<(usize, usize), i32> = BTreeMap::new(); // (v_local, eidx) -> port_x

    let mut out_by_node: Vec<Vec<(i32, usize)>> = vec![Vec::new(); k];
    let mut in_by_node: Vec<Vec<(i32, usize)>> = vec![Vec::new(); k];

    for &(eidx, u, v) in &direct_forward {
        out_by_node[u].push((local_x[v] + CARD_W / 2, eidx));
        in_by_node[v].push((local_x[u] + CARD_W / 2, eidx));
    }
    for (&pos, chain) in &edge_dummy_chains {
        let (eidx, u, v) = safe_forward[pos];
        let first_dx = dummy_nodes[chain[0]].x + DUMMY_W / 2;
        let last_dx = dummy_nodes[*chain.last().unwrap()].x + DUMMY_W / 2;
        out_by_node[u].push((first_dx, eidx));
        in_by_node[v].push((last_dx, eidx));
    }
    for &(eidx, u, v) in right_gutter_edges.iter().chain(left_gutter_edges.iter()) {
        let gx = edge_gutter_x[&eidx];
        out_by_node[u].push((gx, eidx));
        in_by_node[v].push((gx, eidx));
    }

    for (u, list) in out_by_node.iter_mut().enumerate() {
        list.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let ports = distribute_ports(local_x[u], list.len());
        for (idx, &(_, eidx)) in list.iter().enumerate() {
            out_ports.insert((u, eidx), ports[idx]);
        }
    }
    for (v, list) in in_by_node.iter_mut().enumerate() {
        list.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let ports = distribute_ports(local_x[v], list.len());
        for (idx, &(_, eidx)) in list.iter().enumerate() {
            in_ports.insert((v, eidx), ports[idx]);
        }
    }

    // Materialize edge waypoints and compute exact component bounding box.
    let mut edge_routes: BTreeMap<usize, (RouteKind, Vec<(i32, i32)>)> = BTreeMap::new();
    for &(eidx, u, v) in &direct_forward {
        let wps = vec![
            (out_ports[&(u, eidx)], rank_y[ranks[u]] + CARD_H),
            (in_ports[&(v, eidx)], rank_y[ranks[v]] - ARROW_PULLBACK),
        ];
        edge_routes.insert(eidx, (RouteKind::Forward, wps));
    }
    for (&pos, chain) in &edge_dummy_chains {
        let (eidx, u, v) = safe_forward[pos];
        let mut wps = Vec::with_capacity(2 + chain.len() * 2);
        wps.push((out_ports[&(u, eidx)], rank_y[ranks[u]] + CARD_H));
        for &d_idx in chain {
            let dn = &dummy_nodes[d_idx];
            let dx = dn.x + DUMMY_W / 2;
            let dy_top = rank_y[dn.rank];
            wps.push((dx, dy_top));
            wps.push((dx, dy_top + CARD_H));
        }
        wps.push((in_ports[&(v, eidx)], rank_y[ranks[v]] - ARROW_PULLBACK));
        edge_routes.insert(eidx, (RouteKind::Forward, wps));
    }
    for &(eidx, u, v) in &right_gutter_edges {
        let wps = gutter_waypoints(
            out_ports[&(u, eidx)],
            rank_y[ranks[u]] + CARD_H,
            edge_y_exit[&eidx],
            edge_gutter_x[&eidx],
            edge_y_enter[&eidx],
            in_ports[&(v, eidx)],
            rank_y[ranks[v]] - ARROW_PULLBACK,
        );
        edge_routes.insert(eidx, (RouteKind::CycleGutter, wps));
    }
    for &(eidx, u, v) in &left_gutter_edges {
        let wps = gutter_waypoints(
            out_ports[&(u, eidx)],
            rank_y[ranks[u]] + CARD_H,
            edge_y_exit[&eidx],
            edge_gutter_x[&eidx],
            edge_y_enter[&eidx],
            in_ports[&(v, eidx)],
            rank_y[ranks[v]] - ARROW_PULLBACK,
        );
        edge_routes.insert(eidx, (RouteKind::OverflowLeft, wps));
    }
    for &(eidx, u) in &self_loop_edges {
        let wps = self_loop_waypoints(local_x[u], rank_y[ranks[u]]);
        edge_routes.insert(eidx, (RouteKind::SelfLoop, wps));
    }

    let mut min_x = min_card_left;
    let mut max_x = max_card_right;
    let mut min_y = 0i32;
    let mut max_y = rank_y[num_ranks - 1] + CARD_H;

    for dn in &dummy_nodes {
        min_x = min_x.min(dn.x);
        max_x = max_x.max(dn.x + DUMMY_W);
    }
    for (_, wps) in edge_routes.values() {
        for &(wx, wy) in wps {
            min_x = min_x.min(wx);
            max_x = max_x.max(wx);
            min_y = min_y.min(wy);
            max_y = max_y.max(wy);
        }
    }

    let mut node_positions = BTreeMap::new();
    for (local, &gidx) in comp_nodes.iter().enumerate() {
        node_positions.insert(
            gidx,
            (local_x[local], rank_y[ranks[local]], ranks[local] as i32),
        );
    }

    let focal_band_y = focal_rank_opt.map(|fr| rank_y[fr]);

    ComponentLayout {
        node_positions,
        edge_routes,
        focal_band_y,
        min_x,
        max_x,
        min_y,
        max_y,
        rank_count: num_ranks,
        sweeps_used,
    }
}

fn gutter_waypoints(
    out_x: i32,
    out_y: i32,
    y_exit: i32,
    gutter_x: i32,
    y_enter: i32,
    in_x: i32,
    in_y: i32,
) -> Vec<(i32, i32)> {
    vec![
        (out_x, out_y),
        (out_x, y_exit),
        (gutter_x, y_exit),
        (gutter_x, y_enter),
        (in_x, y_enter),
        (in_x, in_y),
    ]
}

fn distribute_ports(card_x: i32, count: usize) -> Vec<i32> {
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![card_x + CARD_W / 2];
    }
    let left = card_x + 20;
    let right = card_x + CARD_W - 20;
    let span = right - left;
    (0..count)
        .map(|i| left + (span * i as i32) / (count as i32 - 1))
        .collect()
}

type EdgeTriple = (usize, usize, usize);

fn ensure_acyclic_forward(n: usize, edges: &[EdgeTriple]) -> (Vec<EdgeTriple>, Vec<EdgeTriple>) {
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (pos, &(_eidx, u, v)) in edges.iter().enumerate() {
        adj[u].push((v, pos));
    }
    let mut color = vec![0u8; n];
    let mut is_cycle_back = vec![false; edges.len()];

    fn dfs(u: usize, adj: &[Vec<(usize, usize)>], color: &mut [u8], is_back: &mut [bool]) {
        color[u] = 1;
        for &(v, pos) in &adj[u] {
            match color[v] {
                1 => is_back[pos] = true,
                0 => dfs(v, adj, color, is_back),
                _ => {}
            }
        }
        color[u] = 2;
    }

    for u in 0..n {
        if color[u] == 0 {
            dfs(u, &adj, &mut color, &mut is_cycle_back);
        }
    }

    let mut fwd = Vec::new();
    let mut back = Vec::new();
    for (pos, &edge) in edges.iter().enumerate() {
        if is_cycle_back[pos] {
            back.push(edge);
        } else {
            fwd.push(edge);
        }
    }
    (fwd, back)
}

fn assign_ranks(
    n: usize,
    forward_edges: &[(usize, usize, usize)],
    comp_roots: &[usize],
    direction: SliceDirection,
) -> (Vec<usize>, Option<usize>) {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut rev_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(_, u, v) in forward_edges {
        adj[u].push(v);
        rev_adj[v].push(u);
        indeg[v] += 1;
    }

    // Topological order via Kahn's algorithm.
    let mut q: VecDeque<usize> = VecDeque::new();
    let mut indeg_tmp = indeg.clone();
    for (u, &deg) in indeg_tmp.iter().enumerate() {
        if deg == 0 {
            q.push_back(u);
        }
    }
    let mut topo = Vec::with_capacity(n);
    while let Some(u) = q.pop_front() {
        topo.push(u);
        for &v in &adj[u] {
            indeg_tmp[v] -= 1;
            if indeg_tmp[v] == 0 {
                q.push_back(v);
            }
        }
    }

    // Attempt Both waist pinning when applicable.
    if direction == SliceDirection::Both && !comp_roots.is_empty() {
        let root_flags: Vec<bool> = (0..n).map(|u| comp_roots.contains(&u)).collect();
        let mut inter_root_path = false;
        'check_roots: for &r in comp_roots {
            let mut seen = vec![false; n];
            let mut rq = VecDeque::new();
            seen[r] = true;
            rq.push_back(r);
            while let Some(u) = rq.pop_front() {
                for &v in &adj[u] {
                    if root_flags[v] && v != r {
                        inter_root_path = true;
                        break 'check_roots;
                    }
                    if !seen[v] {
                        seen[v] = true;
                        rq.push_back(v);
                    }
                }
            }
        }

        if !inter_root_path {
            // Find strict ancestors of any root.
            let mut is_ancestor = vec![false; n];
            let mut aq = VecDeque::new();
            for &r in comp_roots {
                aq.push_back(r);
            }
            while let Some(v) = aq.pop_front() {
                for &u in &rev_adj[v] {
                    if !root_flags[u] && !is_ancestor[u] {
                        is_ancestor[u] = true;
                        aq.push_back(u);
                    }
                }
            }

            // Compute signed ranks: roots = 0, ancestors < 0, rest >= 0.
            let mut signed_rank = vec![0i32; n];
            // Reverse topological order for ancestors.
            for &u in topo.iter().rev() {
                if is_ancestor[u] {
                    let mut min_next = 0i32;
                    for &v in &adj[u] {
                        if root_flags[v] || is_ancestor[v] {
                            min_next = min_next.min(signed_rank[v] - 1);
                        }
                    }
                    signed_rank[u] = min_next.min(-1);
                }
            }
            // Forward topological order for non-ancestor, non-root nodes.
            for &v in &topo {
                if !root_flags[v] && !is_ancestor[v] {
                    let mut r = 0i32;
                    for &u in &rev_adj[v] {
                        r = r.max(signed_rank[u] + 1);
                    }
                    signed_rank[v] = r;
                }
            }

            let min_r = signed_rank.iter().copied().min().unwrap_or(0);
            let shifted: Vec<usize> = signed_rank
                .into_iter()
                .map(|r| (r - min_r) as usize)
                .collect();
            let focal_rank = (-min_r) as usize;
            return (shifted, Some(focal_rank));
        }
    }

    // Standard longest-path ranking.
    let mut ranks = vec![0usize; n];
    for &u in &topo {
        let next_r = ranks[u] + 1;
        for &v in &adj[u] {
            if next_r > ranks[v] {
                ranks[v] = next_r;
            }
        }
    }
    (ranks, None)
}

fn item_key(item: LayerItem) -> (bool, usize) {
    match item {
        LayerItem::Real(u) => (false, u),
        LayerItem::Dummy(d) => (true, d),
    }
}

fn barycenter_sort(
    items: &mut [LayerItem],
    nbrs: &BTreeMap<(bool, usize), Vec<(bool, usize)>>,
    nbr_pos: &BTreeMap<(bool, usize), usize>,
    dummy_nodes: &[DummyNode],
) {
    let mut annotated: Vec<(LayerItem, usize, usize, usize)> = items
        .iter()
        .enumerate()
        .map(|(cur_pos, &item)| {
            let key = item_key(item);
            let (sum, count) = nbrs
                .get(&key)
                .map(|list| {
                    let mut s = 0usize;
                    let mut c = 0usize;
                    for nk in list {
                        if let Some(&p) = nbr_pos.get(nk) {
                            s += p;
                            c += 1;
                        }
                    }
                    (s, c)
                })
                .unwrap_or((cur_pos, 1));
            let (s, c) = if count == 0 {
                (cur_pos, 1)
            } else {
                (sum, count)
            };
            (item, s, c, cur_pos)
        })
        .collect();

    annotated.sort_by(|a, b| {
        let lhs = (a.1 as u64) * (b.2 as u64);
        let rhs = (b.1 as u64) * (a.2 as u64);
        lhs.cmp(&rhs).then_with(|| a.3.cmp(&b.3)).then_with(|| {
            canonical_item_key(a.0, dummy_nodes).cmp(&canonical_item_key(b.0, dummy_nodes))
        })
    });

    for (i, entry) in annotated.into_iter().enumerate() {
        items[i] = entry.0;
    }
}

fn canonical_item_key(item: LayerItem, dummy_nodes: &[DummyNode]) -> (usize, usize, usize) {
    match item {
        LayerItem::Real(u) => (0, u, 0),
        LayerItem::Dummy(d) => (1, dummy_nodes[d].edge_idx, dummy_nodes[d].step),
    }
}

fn relax_rank_x(
    items: &[LayerItem],
    nbrs: &BTreeMap<(bool, usize), Vec<(bool, usize)>>,
    item_x: &mut BTreeMap<(bool, usize), i32>,
) {
    let mut desired = Vec::with_capacity(items.len());
    for &item in items {
        let key = item_key(item);
        let cur_x = item_x[&key];
        let mut centers: Vec<i32> = nbrs
            .get(&key)
            .map(|list| {
                list.iter()
                    .filter_map(|nk| {
                        item_x.get(nk).map(|&nx| {
                            let nw = if nk.0 { DUMMY_W } else { CARD_W };
                            nx + nw / 2
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let target_x = if centers.is_empty() {
            cur_x
        } else {
            centers.sort_unstable();
            let med = if centers.len() % 2 == 1 {
                centers[centers.len() / 2]
            } else {
                (centers[centers.len() / 2 - 1] + centers[centers.len() / 2]) / 2
            };
            med - item.width() / 2
        };
        desired.push((key, item.width(), target_x));
    }

    // Clamp left-to-right so adjacent items never overlap horizontally.
    let mut prev_right: Option<i32> = None;
    for (key, w, target_x) in desired {
        let x = if let Some(pr) = prev_right {
            target_x.max(pr + H_GAP)
        } else {
            target_x
        };
        item_x.insert(key, x);
        prev_right = Some(x + w);
    }
}
