use std::collections::BTreeMap;
use std::fmt::Write;

use crate::export::ir::{AbsenceClass, ExportGraph, ExportNode};
use crate::export::ExportOptions;
use crate::protocol::schema::SliceDirection;

pub fn render_dot(graph: &ExportGraph, opts: &ExportOptions) -> String {
    let mut out = String::with_capacity(2048);
    let dir_str = match graph.direction {
        SliceDirection::To => "to",
        SliceDirection::From => "from",
        SliceDirection::Both => "both",
    };
    let _ = writeln!(
        out,
        "// lineage.nvim export · direction={dir_str} · nodes={} · edges={}",
        graph.nodes.len(),
        graph.edges.len()
    );
    if !graph.completeness.badges.is_empty() {
        let _ = writeln!(
            out,
            "// status: {}",
            graph.completeness.badges.join("; ").replace('\n', " ")
        );
    }

    out.push_str("digraph lineage {\n");
    out.push_str("  rankdir=TB;\n");
    out.push_str(
        "  node [shape=box, style=\"rounded,filled\", fontname=\"monospace\", fontsize=10];\n",
    );
    out.push_str("  edge [fontname=\"monospace\", fontsize=9];\n");

    if !graph.completeness.badges.is_empty() {
        let _ = writeln!(
            out,
            "  labelloc=\"t\";\n  label=\"{}\";",
            escape_dot(&graph.completeness.badges.join(" · "))
        );
    }

    if graph.nodes.is_empty() {
        out.push_str("  N0 [label=\"No flow nodes\", fillcolor=\"#e6f4ea\", color=\"#1e8e3e\"];\n");
        out.push_str("}\n");
        return out;
    }

    let mut by_file: BTreeMap<usize, Vec<&ExportNode>> = BTreeMap::new();
    for node in &graph.nodes {
        by_file.entry(node.file_index).or_default().push(node);
    }

    let is_single_trivial = graph.nodes.len() == 1
        && graph.edges.is_empty()
        && matches!(graph.nodes[0].absence_class, AbsenceClass::NonNull)
        && !graph.nodes[0].is_unverified;

    for (file_idx, nodes) in by_file {
        let file_label = graph
            .files
            .get(file_idx)
            .map(|f| f.path.as_str())
            .unwrap_or("<unknown>");
        let _ = writeln!(out, "  subgraph cluster_f{file_idx} {{");
        let _ = writeln!(out, "    label=\"{}\";", escape_dot(file_label));
        out.push_str("    style=rounded;\n    color=\"#9aa0a6\";\n");

        for node in nodes {
            let badges = node_badges(node, is_single_trivial).join(" · ");
            let mut label_text = format!(
                "{}\n:{} · {}",
                node.label,
                node.range.start.line + 1,
                badges
            );
            if opts.include_snippets {
                if let Some(snip) = &node.snippet {
                    if !snip.is_empty() {
                        label_text.push('\n');
                        label_text.push_str(snip);
                    }
                }
            }
            let (fill, stroke, style, penwidth) = node_attrs(node);
            let _ = writeln!(
                out,
                "    {} [label=\"{}\", fillcolor=\"{}\", color=\"{}\", style=\"{}\", penwidth={}];",
                node.id,
                escape_dot(&label_text),
                fill,
                stroke,
                style,
                penwidth
            );
        }
        out.push_str("  }\n");
    }

    for edge in &graph.edges {
        if edge.is_back_edge {
            let lbl = match &edge.label {
                Some(l) if !l.is_empty() => format!("cycle ({l})"),
                _ => "cycle".to_string(),
            };
            let _ = writeln!(
                out,
                "  {} -> {} [style=dashed, color=\"#d93025\", label=\"{}\"];",
                edge.from,
                edge.to,
                escape_dot(&lbl)
            );
        } else if let Some(lbl) = &edge.label {
            if !lbl.is_empty() {
                let _ = writeln!(
                    out,
                    "  {} -> {} [label=\"{}\"];",
                    edge.from,
                    edge.to,
                    escape_dot(lbl)
                );
            } else {
                let _ = writeln!(out, "  {} -> {};", edge.from, edge.to);
            }
        } else {
            let _ = writeln!(out, "  {} -> {};", edge.from, edge.to);
        }
    }

    out.push_str("}\n");
    out
}

fn node_badges(node: &ExportNode, trivial: bool) -> Vec<&'static str> {
    let mut b = Vec::new();
    match node.absence_class {
        AbsenceClass::NonNull => b.push("NonNull"),
        AbsenceClass::Nullable => b.push("Nullable"),
        AbsenceClass::Absent => b.push("Absent"),
    }
    if node.is_unverified {
        b.push("Unverified");
    }
    if node.is_guard {
        b.push("Guard");
    }
    if node.is_danger_sink {
        b.push("Local Risk");
    }
    if node.is_frontier {
        b.push("Frontier");
    }
    if trivial {
        b.push("trivial");
    }
    b
}

fn node_attrs(node: &ExportNode) -> (&'static str, &'static str, &'static str, &'static str) {
    if node.is_danger_sink {
        ("#fce8e6", "#c5221f", "rounded,filled", "2.5")
    } else if node.is_frontier {
        ("#f1f3f4", "#5f6368", "rounded,filled,dotted", "1.5")
    } else if node.is_guard {
        ("#e8f0fe", "#1a73e8", "rounded,filled", "2.0")
    } else if matches!(node.absence_class, AbsenceClass::Absent) {
        ("#f8d7da", "#b31412", "rounded,filled", "2.5")
    } else if matches!(node.absence_class, AbsenceClass::Nullable) && node.is_unverified {
        ("#fce8e6", "#f9ab00", "rounded,filled,dashed", "2.0")
    } else if matches!(node.absence_class, AbsenceClass::Nullable) {
        ("#fce8e6", "#d93025", "rounded,filled", "2.0")
    } else if node.is_unverified {
        ("#fef7e0", "#f9ab00", "rounded,filled,dashed", "2.0")
    } else {
        ("#e6f4ea", "#1e8e3e", "rounded,filled", "1.5")
    }
}

pub fn escape_dot(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}
