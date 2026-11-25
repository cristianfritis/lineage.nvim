use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::export::ir::{AbsenceClass, ExportEdge, ExportGraph, ExportNode};
use crate::export::ExportOptions;
use crate::protocol::schema::{SliceDirection, SliceEnvelope};

pub const MERMAID_MAX_BYTES: usize = 45_000;

const INIT_DIRECTIVE: &str =
    "%%{init: {'flowchart': {'curve': 'basis', 'nodeSpacing': 38, 'rankSpacing': 46, 'padding': 14}}}%%";

pub fn render_mermaid(
    envelope: &SliceEnvelope,
    graph: &ExportGraph,
    opts: &ExportOptions,
) -> (String, ExportGraph, Vec<String>) {
    let mut warnings = Vec::new();
    let mut active_graph = graph.clone();
    let mut rendered = render_mermaid_once(&active_graph, opts.include_snippets, &warnings);

    if rendered.len() > MERMAID_MAX_BYTES && opts.include_snippets {
        warnings.push("Mermaid output exceeded 45 KB; snippets omitted".to_string());
        rendered = render_mermaid_once(&active_graph, false, &warnings);
    }

    if rendered.len() > MERMAID_MAX_BYTES {
        let mut cap = active_graph.nodes.len().min(120);
        while rendered.len() > MERMAID_MAX_BYTES && cap > 5 {
            cap = (cap * 2 / 3).max(5);
            let mut tighter = opts.clone();
            tighter.include_snippets = false;
            tighter.max_nodes = cap;
            active_graph = ExportGraph::build(envelope, &tighter);
            let mut step_warnings = warnings.clone();
            step_warnings.push(format!(
                "Mermaid output exceeded 45 KB; pruned to {} nodes",
                active_graph.nodes.len()
            ));
            rendered = render_mermaid_once(&active_graph, false, &step_warnings);
            if rendered.len() <= MERMAID_MAX_BYTES || cap == 5 {
                warnings = step_warnings;
                break;
            }
        }
    }

    (rendered, active_graph, warnings)
}

fn render_mermaid_once(graph: &ExportGraph, include_snippets: bool, warnings: &[String]) -> String {
    let mut out = String::with_capacity(2048);
    let dir_str = match graph.direction {
        SliceDirection::To => "to",
        SliceDirection::From => "from",
        SliceDirection::Both => "both",
    };
    let _ = writeln!(out, "{INIT_DIRECTIVE}");
    let _ = writeln!(
        out,
        "%% lineage.nvim export · direction={dir_str} · nodes={} · edges={}",
        graph.nodes.len(),
        graph.edges.len()
    );
    if !graph.completeness.badges.is_empty() {
        let _ = writeln!(
            out,
            "%% status: {}",
            escape_comment(&graph.completeness.badges.join("; "))
        );
    }
    for w in warnings {
        let _ = writeln!(out, "%% warning: {}", escape_comment(w));
    }
    out.push_str("flowchart TD\n");

    if graph.nodes.is_empty() {
        out.push_str("  N0([\"<b>No flow nodes</b>\"]):::nonnull\n\n");
        let mut used = BTreeSet::new();
        used.insert("nonnull");
        emit_used_class_defs(&mut out, &used);
        return out;
    }

    let mut nodes_by_id: BTreeMap<&str, &ExportNode> = BTreeMap::new();
    let mut used_classes: BTreeSet<&'static str> = BTreeSet::new();

    for node in &graph.nodes {
        nodes_by_id.insert(node.id.as_str(), node);
        let loc_pill = format!("{}:{}", node.norm_path, node.range.start.line + 1);
        let badges = node_badges(node).join(" · ");
        let display_symbol = if node.is_root {
            format!("◎ {}", node.label)
        } else {
            node.label.clone()
        };
        let mut label = format!(
            "<b>{}</b><br/><small>{} · {}</small>",
            escape_mermaid_label(&display_symbol),
            escape_mermaid_label(&loc_pill),
            escape_mermaid_label(&badges)
        );
        if include_snippets {
            if let Some(snip) = &node.snippet {
                if !snip.is_empty() {
                    label.push_str("<br/><small><code>");
                    label.push_str(&escape_mermaid_label(snip));
                    label.push_str("</code></small>");
                }
            }
        }
        let cls = node_class(node);
        used_classes.insert(cls);
        let _ = writeln!(out, "{}", format_node_with_shape(node, &label, cls));
    }

    // Grouped link styles in deterministic first-seen order
    let mut style_groups: Vec<(&'static str, Vec<usize>)> = Vec::new();
    if !graph.edges.is_empty() {
        out.push('\n');
    }

    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        let from_node = nodes_by_id.get(edge.from.as_str()).copied();
        let to_node = nodes_by_id.get(edge.to.as_str()).copied();

        let style = edge_link_style(edge, from_node, to_node);
        if let Some((_, indices)) = style_groups.iter_mut().find(|(s, _)| *s == style) {
            indices.push(edge_idx);
        } else {
            style_groups.push((style, vec![edge_idx]));
        }

        if edge.is_back_edge {
            let lbl = match &edge.label {
                Some(l) if !l.is_empty() => format!("cycle ({l})"),
                _ => "cycle".to_string(),
            };
            let _ = writeln!(
                out,
                "  {} -.->|\"{}\"| {}",
                edge.from,
                escape_mermaid_label(&lbl),
                edge.to
            );
            continue;
        }

        let to_is_danger = to_node.is_some_and(|n| n.is_danger_sink);
        if to_is_danger {
            let lbl = edge
                .label
                .as_deref()
                .filter(|l| !l.is_empty())
                .unwrap_or("unguarded");
            let _ = writeln!(
                out,
                "  {} ==>|\"{}\"| {}",
                edge.from,
                escape_mermaid_label(lbl),
                edge.to
            );
            continue;
        }

        let is_unverified_edge = from_node.is_some_and(|n| n.is_unverified || n.is_frontier)
            || to_node.is_some_and(|n| n.is_frontier);
        if is_unverified_edge {
            let lbl = edge
                .label
                .as_deref()
                .filter(|l| !l.is_empty())
                .unwrap_or("unverified");
            let _ = writeln!(
                out,
                "  {} -.->|\"{}\"| {}",
                edge.from,
                escape_mermaid_label(lbl),
                edge.to
            );
            continue;
        }

        // Normal edge: omit redundant "phi" label when targeting a merge hexagon,
        // and normalize any legacy "calls" label to "returns".
        let to_is_merge = to_node.is_some_and(|n| n.is_merge);
        let clean_label = edge.label.as_deref().and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() || (to_is_merge && trimmed == "phi") {
                None
            } else if trimmed == "calls" {
                Some("returns")
            } else {
                Some(trimmed)
            }
        });

        if let Some(lbl) = clean_label {
            let _ = writeln!(
                out,
                "  {} -->|\"{}\"| {}",
                edge.from,
                escape_mermaid_label(lbl),
                edge.to
            );
        } else {
            let _ = writeln!(out, "  {} --> {}", edge.from, edge.to);
        }
    }

    if !style_groups.is_empty() {
        out.push('\n');
        for (style, indices) in style_groups {
            let idx_list: Vec<String> = indices.iter().map(|i| i.to_string()).collect();
            let _ = writeln!(out, "  linkStyle {} {}", idx_list.join(","), style);
        }
    }

    if !used_classes.is_empty() {
        out.push('\n');
        emit_used_class_defs(&mut out, &used_classes);
    }
    out
}

fn edge_link_style(
    edge: &ExportEdge,
    from_node: Option<&ExportNode>,
    to_node: Option<&ExportNode>,
) -> &'static str {
    let to_is_danger = to_node.is_some_and(|n| n.is_danger_sink);
    if to_is_danger || edge.label.as_deref() == Some("unguarded") {
        return "stroke:#ff3b6b,stroke-width:2.5px";
    }

    let is_cross_file_return = matches!(edge.label.as_deref(), Some("returns" | "calls"))
        || edge.provenance.contains("cross_file_call");
    if is_cross_file_return {
        return "stroke:#14b8a6,stroke-width:2px";
    }

    let is_guard_narrowed = from_node.is_some_and(|n| n.is_guard)
        && to_node
            .is_some_and(|n| matches!(n.absence_class, AbsenceClass::NonNull) && !n.is_unverified);
    if is_guard_narrowed {
        return "stroke:#2ea043,stroke-width:2px";
    }

    let is_unverified_or_cycle = edge.is_back_edge
        || from_node.is_some_and(|n| n.is_unverified || n.is_frontier)
        || to_node.is_some_and(|n| n.is_frontier);
    if is_unverified_or_cycle {
        return "stroke:#a371f7,stroke-width:1.75px";
    }

    if let Some(src) = from_node {
        match src.absence_class {
            AbsenceClass::Absent => "stroke:#f85149,stroke-width:2px",
            AbsenceClass::Nullable => "stroke:#e38b29,stroke-width:1.75px",
            AbsenceClass::NonNull => "stroke:#2ea043,stroke-width:1.75px",
        }
    } else {
        "stroke:#8b949e,stroke-width:1.75px"
    }
}

fn format_node_with_shape(node: &ExportNode, escaped_label: &str, cls: &str) -> String {
    if node.is_root {
        format!("  {}([\"{escaped_label}\"]):::{cls}", node.id)
    } else if node.is_guard {
        format!("  {}{{\"{escaped_label}\"}}:::{cls}", node.id)
    } else if node.is_merge {
        format!("  {}{{{{\"{escaped_label}\"}}}}:::{cls}", node.id)
    } else if node.label.starts_with("fn ") {
        format!("  {}[[\"{escaped_label}\"]]:::{cls}", node.id)
    } else {
        format!("  {}(\"{escaped_label}\"):::{cls}", node.id)
    }
}

fn node_badges(node: &ExportNode) -> Vec<&'static str> {
    let mut b = Vec::new();
    if node.is_danger_sink {
        b.push("⚠ Deref Danger");
    } else if node.is_guard {
        b.push("◇ Guard");
        if node.is_unverified {
            b.push("? Unverified");
        }
    } else {
        match (node.absence_class, node.is_unverified) {
            (AbsenceClass::Absent, false) => b.push("∅ Absent"),
            (AbsenceClass::Absent, true) => {
                b.push("∅ Absent");
                b.push("? Unverified");
            }
            (AbsenceClass::Nullable, true) => b.push("◐? Nullable · unverified"),
            (AbsenceClass::Nullable, false) => b.push("◐ Nullable"),
            (AbsenceClass::NonNull, true) => b.push("? Unverified"),
            (AbsenceClass::NonNull, false) => b.push("✓ NonNull"),
        }
    }
    if node.is_frontier {
        b.push("Frontier");
    }
    b
}

fn node_class(node: &ExportNode) -> &'static str {
    if node.is_danger_sink {
        "danger"
    } else if node.is_frontier {
        "frontier"
    } else if node.is_guard {
        "guard"
    } else if matches!(node.absence_class, AbsenceClass::Absent) {
        "absent"
    } else if matches!(node.absence_class, AbsenceClass::Nullable) && node.is_unverified {
        "nullable_unverified"
    } else if matches!(node.absence_class, AbsenceClass::Nullable) {
        "nullable"
    } else if node.is_unverified {
        "unverified"
    } else {
        "nonnull"
    }
}

fn emit_used_class_defs(out: &mut String, used: &BTreeSet<&'static str>) {
    const PALETTE: &[(&str, &str)] = &[
        (
            "nonnull",
            "fill:#2ea04326,stroke:#2ea043,stroke-width:1.5px",
        ),
        ("nullable", "fill:#e38b2926,stroke:#e38b29,stroke-width:2px"),
        (
            "nullable_unverified",
            "fill:#e38b2926,stroke:#a371f7,stroke-width:2px,stroke-dasharray: 5 3",
        ),
        ("absent", "fill:#f8514928,stroke:#f85149,stroke-width:2.5px"),
        (
            "unverified",
            "fill:#a371f726,stroke:#a371f7,stroke-width:2px,stroke-dasharray: 5 3",
        ),
        ("guard", "fill:#388bfd26,stroke:#388bfd,stroke-width:2px"),
        ("danger", "fill:#ff3b6b33,stroke:#ff3b6b,stroke-width:3px"),
        (
            "frontier",
            "fill:#76839022,stroke:#768390,stroke-width:1.5px,stroke-dasharray: 3 3",
        ),
    ];
    for &(name, style) in PALETTE {
        if used.contains(name) {
            let _ = writeln!(out, "  classDef {name} {style}");
        }
    }
}

/// Escapes text for use inside double-quoted Mermaid labels without emitting numeric `#<num>;`
/// entities (which Mermaid decodes partially, leaving orphaned `&` artifacts like `&:`).
pub fn escape_mermaid_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' | '\n' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

fn escape_comment(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace("%%", "% %")
}
