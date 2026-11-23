use std::collections::{HashMap, HashSet};

use crate::ir::AbsenceValue;
use crate::protocol::schema::{
    CanonicalRange, FileEntry, FocusSpan, HoverCardProjection, HoverItem, HoverRisk, NodeKind,
    SliceGraph, TreeRow, TruncationReason, UnknownBoundary,
};

#[allow(clippy::too_many_arguments)]
pub fn project_hover_card(
    symbol: &str,
    root_absence: AbsenceValue,
    files: &[FileEntry],
    phase1_candidates: Vec<HoverItem>,
    phase2_candidates: Vec<HoverItem>,
    guards: Vec<String>,
    local_risks: Vec<HoverRisk>,
    unknown_boundaries: Vec<UnknownBoundary>,
    truncation: &[TruncationReason],
) -> HoverCardProjection {
    let _ = files;
    let unverified_count = unknown_boundaries.len();
    let has_truncation = !truncation.is_empty();
    let is_partial = unverified_count > 0 || has_truncation;

    let status_badge = if root_absence.is_definitely_absent() {
        "Absent".to_string()
    } else if root_absence.is_maybe_absent() {
        if root_absence.to_summary().maybe_null && root_absence.to_summary().maybe_undefined {
            "Nullable (null | undefined)".to_string()
        } else if root_absence.to_summary().maybe_null {
            "Nullable (null)".to_string()
        } else {
            "Nullable (undefined)".to_string()
        }
    } else if root_absence.is_present_only() {
        "NonNull".to_string()
    } else if is_partial {
        "Unverified".to_string()
    } else {
        "NonNull".to_string()
    };

    let verdict_line = if is_partial && unverified_count > 0 {
        format!(
            "{}: {} (partial — {} unverified [gu])",
            symbol, status_badge, unverified_count
        )
    } else if is_partial {
        format!("{}: {} (partial — truncated)", symbol, status_badge)
    } else {
        format!("{}: {}", symbol, status_badge)
    };

    let absent_origin_count = phase1_candidates
        .iter()
        .chain(phase2_candidates.iter())
        .filter(|i| i.is_absent_origin)
        .count();

    let is_trivial_peek = root_absence.is_present_only()
        && absent_origin_count == 0
        && local_risks.is_empty()
        && unknown_boundaries.is_empty()
        && !has_truncation;

    let peek_lines = if is_trivial_peek {
        let mut lines = vec![verdict_line.clone()];
        if let Some(first) = phase1_candidates.first() {
            lines.push(format!("  origin: {}", first.label));
        }
        lines
    } else {
        Vec::new()
    };

    // Phase 1 items: capped at 5 when open cross-file boundaries exist, up to 9 otherwise.
    let phase1_cap = if unverified_count > 0 { 5 } else { 9 };
    let mut seen: HashSet<(u32, CanonicalRange)> = HashSet::new();
    let mut phase1_items = Vec::new();
    for mut item in phase1_candidates {
        if phase1_items.len() >= phase1_cap {
            break;
        }
        if seen.insert((item.file_id, item.range)) {
            item.index = phase1_items.len() + 1;
            phase1_items.push(item);
        }
    }

    let mut phase2_items = Vec::new();
    let mut next_idx = phase1_items.len() + 1;
    for mut item in phase2_candidates {
        if next_idx > 9 {
            break;
        }
        if seen.insert((item.file_id, item.range)) {
            item.index = next_idx;
            next_idx += 1;
            phase2_items.push(item);
        }
    }

    HoverCardProjection {
        symbol: symbol.to_string(),
        verdict_line,
        status_badge,
        is_trivial_peek,
        peek_lines,
        phase1_items,
        phase2_items,
        guards,
        local_risks,
        unknown_boundaries,
        unverified_count,
    }
}

pub fn project_focus_spans(graph: &SliceGraph, files: &[FileEntry]) -> Vec<FocusSpan> {
    let file_map: HashMap<u32, &str> = files.iter().map(|f| (f.file_id, f.uri.as_str())).collect();
    // Deduplicate by (file_id, start_line), keeping highest-severity state:
    // absent (2) > unknown (1) > present (0).
    let mut by_line: HashMap<(u32, u32), (u8, FocusSpan)> = HashMap::new();

    for node in &graph.nodes {
        let uri = file_map.get(&node.file_id).copied().unwrap_or("").to_string();
        let is_absent =
            node.absence.maybe_null || node.absence.maybe_undefined || !node.absence.present;
        let is_unknown = !node.absence.unknown_reasons.is_empty();
        let (severity, badge, highlight) = if is_absent {
            (2u8, "[NULL]".to_string(), "LineageStateAbsent".to_string())
        } else if is_unknown {
            (1u8, "[?]".to_string(), "LineageStateUnknown".to_string())
        } else {
            (0u8, "[OK]".to_string(), "LineageStatePresent".to_string())
        };

        let role = if node.label.starts_with("narrow ") {
            "narrow".to_string()
        } else if is_unknown {
            "unknown".to_string()
        } else {
            node.provenance.clone()
        };

        let span = FocusSpan {
            file_id: node.file_id,
            uri,
            range: node.range,
            role,
            badge,
            highlight,
            label: node.label.clone(),
        };

        let key = (node.file_id, node.range.start.line);
        match by_line.get(&key) {
            Some((existing_sev, _)) if *existing_sev >= severity => {}
            _ => {
                by_line.insert(key, (severity, span));
            }
        }
    }

    let mut spans: Vec<FocusSpan> = by_line.into_values().map(|(_, s)| s).collect();
    spans.sort_by(|a, b| {
        a.file_id
            .cmp(&b.file_id)
            .then_with(|| a.range.start.cmp(&b.range.start))
    });
    spans
}

pub fn project_tree_rows(
    root_ids: &[String],
    graph: &SliceGraph,
    files: &[FileEntry],
) -> Vec<TreeRow> {
    let file_map: HashMap<u32, &str> = files.iter().map(|f| (f.file_id, f.uri.as_str())).collect();
    let node_map: HashMap<&str, &crate::protocol::schema::FlowNode> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut adj: HashMap<&str, Vec<&crate::protocol::schema::FlowEdge>> = HashMap::new();
    for edge in &graph.edges {
        adj.entry(edge.from.as_str()).or_default().push(edge);
    }

    let mut rows = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();

    #[allow(clippy::too_many_arguments)]
    fn dfs<'a>(
        node_id: &'a str,
        depth: u32,
        is_cycle: bool,
        root_origin: &str,
        file_map: &HashMap<u32, &str>,
        node_map: &HashMap<&str, &'a crate::protocol::schema::FlowNode>,
        adj: &HashMap<&str, Vec<&'a crate::protocol::schema::FlowEdge>>,
        visited: &mut HashSet<String>,
        rows: &mut Vec<TreeRow>,
    ) {
        let Some(node) = node_map.get(node_id).copied() else {
            return;
        };
        let uri = file_map.get(&node.file_id).copied().unwrap_or("").to_string();
        let is_absent =
            node.absence.maybe_null || node.absence.maybe_undefined || !node.absence.present;
        let is_unknown = !node.absence.unknown_reasons.is_empty();
        let state_badge = if is_absent {
            "[NULL]"
        } else if is_unknown {
            "[?]"
        } else {
            "[OK]"
        };
        let is_frontier = matches!(node.kind, NodeKind::FrontierNode | NodeKind::ClusterNode);
        let badge = if is_cycle {
            format!("{state_badge} [cycle]")
        } else if is_frontier {
            format!("{state_badge} [+]")
        } else {
            state_badge.to_string()
        };

        let expansion_token = if is_frontier {
            Some(format!(
                "exp:{}:{}:{}",
                node.id, node.file_id, node.range.start.line
            ))
        } else {
            None
        };

        let outgoing = adj.get(node.id.as_str()).map(|v| v.as_slice()).unwrap_or(&[]);
        let has_children = !is_cycle && !outgoing.is_empty();

        rows.push(TreeRow {
            node_id: node.id.clone(),
            depth,
            file_id: node.file_id,
            uri,
            range: node.range,
            label: node.label.clone(),
            badge,
            is_cycle,
            is_frontier,
            expansion_token,
            is_absent,
            group_key: root_origin.to_string(),
            provenance: node.provenance.clone(),
            has_children,
            snippet: node.snippet.clone(),
        });

        if is_cycle || is_frontier {
            return;
        }

        for edge in outgoing {
            let cycle_hit = edge.is_back_edge || !visited.insert(edge.to.clone());
            dfs(
                edge.to.as_str(),
                depth + 1,
                cycle_hit,
                root_origin,
                file_map,
                node_map,
                adj,
                visited,
                rows,
            );
        }
    }

    for root in root_ids {
        if let Some(root_node) = node_map.get(root.as_str()) {
            let origin = root_node.label.clone();
            if visited.insert(root.clone()) {
                dfs(
                    root.as_str(),
                    0,
                    false,
                    &origin,
                    &file_map,
                    &node_map,
                    &adj,
                    &mut visited,
                    &mut rows,
                );
            }
        }
    }

    rows
}
