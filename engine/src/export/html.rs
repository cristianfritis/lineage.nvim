use std::collections::BTreeMap;
use std::fmt::Write;

use serde::Serialize;

use crate::export::ir::{AbsenceClass, ExportGraph, ExportNode};
use crate::export::layout::{RouteKind, SvgLayout, CARD_H, CARD_W};
use crate::export::ExportOptions;
use crate::protocol::schema::SliceDirection;

const VIEWER_CSS: &str = include_str!("assets/viewer.css");
const VIEWER_JS: &str = include_str!("assets/viewer.js");
const CSP_META: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; object-src 'none';";

#[derive(Serialize)]
struct MetaBounds {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Serialize)]
struct MetaNode<'a> {
    id: &'a str,
    symbol: &'a str,
    label: &'a str,
    file_index: usize,
    norm_path: &'a str,
    line_1based: u32,
    col_1based: u32,
    absence_class: &'static str,
    is_unverified: bool,
    is_guard: bool,
    is_danger_sink: bool,
    is_frontier: bool,
    is_root: bool,
    in_cycle: bool,
    unknown_reasons: Vec<String>,
    guard_text: Option<&'a str>,
    danger_kind: Option<&'a str>,
    risk_messages: &'a [String],
    provenance: &'a str,
    partition: &'static str,
    bounds: MetaBounds,
    topo_index: usize,
}

#[derive(Serialize)]
struct MetaEdge<'a> {
    id: &'a str,
    from: &'a str,
    to: &'a str,
    label: Option<&'a str>,
    is_back_edge: bool,
    cycle_group_id: Option<u32>,
}

#[derive(Serialize)]
struct MetaFile<'a> {
    index: usize,
    norm_path: &'a str,
    color_idx: usize,
}

#[derive(Serialize)]
struct MetaExportFilters {
    null_only: bool,
    exclude_tests: bool,
}

#[derive(Serialize)]
struct ViewerMeta<'a> {
    viewer_schema_version: u32,
    direction: &'static str,
    view_width: i32,
    view_height: i32,
    root_ids: &'a [String],
    files: Vec<MetaFile<'a>>,
    nodes: Vec<MetaNode<'a>>,
    edges: Vec<MetaEdge<'a>>,
    completeness_badges: &'a [String],
    truncation: Vec<String>,
    export_filters: MetaExportFilters,
    snippets_removed: bool,
}

pub fn xml_escape_and_strip_ctrl(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let code = ch as u32;
        if (code <= 0x1F && ch != '\t' && ch != '\n') || code == 0x7F {
            continue;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

pub fn safe_json_for_script(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for ch in json.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

pub fn middle_elide(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars || max_chars < 3 {
        return s.to_string();
    }
    let keep = max_chars - 1;
    let head = keep / 2;
    let tail = keep - head;
    let mut out = String::with_capacity(max_chars + 2);
    for &c in &chars[..head] {
        out.push(c);
    }
    out.push('…');
    for &c in &chars[chars.len() - tail..] {
        out.push(c);
    }
    out
}

pub fn render_html(graph: &ExportGraph, layout: &SvgLayout, opts: &ExportOptions) -> String {
    let dir_str = match graph.direction {
        SliceDirection::To => "to",
        SliceDirection::From => "from",
        SliceDirection::Both => "both",
    };

    let primary_node = graph
        .root_ids
        .first()
        .and_then(|rid| graph.nodes.iter().find(|n| &n.id == rid))
        .or_else(|| graph.nodes.first());

    let title_text = match primary_node {
        Some(n) => format!(
            "lineage: {} ({}) — {}:{}",
            n.label,
            dir_str,
            n.norm_path,
            n.range.start.line + 1
        ),
        None => format!("lineage: empty ({dir_str})"),
    };

    let mut file_indices: BTreeMap<usize, usize> = BTreeMap::new();
    for (idx, _f) in graph.files.iter().enumerate() {
        file_indices.insert(idx, idx % 12);
    }

    let layout_node_map: BTreeMap<&str, &crate::export::layout::LayoutNode> =
        layout.nodes.iter().map(|ln| (ln.id.as_str(), ln)).collect();

    let meta_nodes: Vec<MetaNode<'_>> = graph
        .nodes
        .iter()
        .map(|n| {
            let ln = layout_node_map.get(n.id.as_str()).copied();
            MetaNode {
                id: &n.id,
                symbol: &n.label,
                label: &n.label,
                file_index: n.file_index,
                norm_path: &n.norm_path,
                line_1based: n.range.start.line + 1,
                col_1based: n.range.start.col + 1,
                absence_class: match n.absence_class {
                    AbsenceClass::NonNull => "NonNull",
                    AbsenceClass::Nullable => "Nullable",
                    AbsenceClass::Absent => "Absent",
                },
                is_unverified: n.is_unverified,
                is_guard: n.is_guard,
                is_danger_sink: n.is_danger_sink,
                is_frontier: n.is_frontier,
                is_root: n.is_root,
                in_cycle: n.in_cycle,
                unknown_reasons: n.unknown_reasons.iter().map(|r| format!("{r:?}")).collect(),
                guard_text: n.guard_text.as_deref(),
                danger_kind: n.danger_kind.as_deref(),
                risk_messages: &n.risk_messages,
                provenance: &n.provenance,
                partition: match n.partition {
                    crate::export::ir::NodePartition::Upstream => "Upstream",
                    crate::export::ir::NodePartition::Focal => "Focal",
                    crate::export::ir::NodePartition::Downstream => "Downstream",
                },
                bounds: MetaBounds {
                    x: ln.map(|l| l.x).unwrap_or(0),
                    y: ln.map(|l| l.y).unwrap_or(0),
                    w: ln.map(|l| l.w).unwrap_or(CARD_W),
                    h: ln.map(|l| l.h).unwrap_or(CARD_H),
                },
                topo_index: ln.map(|l| l.topo_index).unwrap_or(0),
            }
        })
        .collect();

    let meta_edges: Vec<MetaEdge<'_>> = layout
        .edges
        .iter()
        .enumerate()
        .map(|(i, le)| {
            let raw = graph.edges.get(i);
            MetaEdge {
                id: &le.id,
                from: &le.from,
                to: &le.to,
                label: raw.and_then(|e| e.label.as_deref()),
                is_back_edge: le.is_back_edge,
                cycle_group_id: le.cycle_group_id,
            }
        })
        .collect();

    let meta_files: Vec<MetaFile<'_>> = graph
        .files
        .iter()
        .enumerate()
        .map(|(idx, f)| MetaFile {
            index: idx,
            norm_path: &f.path,
            color_idx: idx % 12,
        })
        .collect();

    let meta = ViewerMeta {
        viewer_schema_version: 1,
        direction: dir_str,
        view_width: layout.view_width,
        view_height: layout.view_height,
        root_ids: &graph.root_ids,
        files: meta_files,
        nodes: meta_nodes,
        edges: meta_edges,
        completeness_badges: &graph.completeness.badges,
        truncation: graph
            .completeness
            .truncation
            .iter()
            .map(|t| format!("{t:?}"))
            .collect(),
        export_filters: MetaExportFilters {
            null_only: graph.null_only,
            exclude_tests: graph.exclude_tests,
        },
        snippets_removed: false,
    };

    let mut snippets_map: BTreeMap<&str, &str> = BTreeMap::new();
    if opts.include_snippets {
        for n in &graph.nodes {
            if let Some(snip) = n.snippet.as_deref() {
                if !snip.is_empty() {
                    snippets_map.insert(&n.id, snip);
                }
            }
        }
    }

    let meta_json =
        safe_json_for_script(&serde_json::to_string(&meta).unwrap_or_else(|_| "{}".into()));
    let snippets_json =
        safe_json_for_script(&serde_json::to_string(&snippets_map).unwrap_or_else(|_| "{}".into()));

    let mut out = String::with_capacity(32 * 1024);
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    let _ = writeln!(
        out,
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{CSP_META}\">"
    );
    let _ = writeln!(
        out,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
    );
    let _ = writeln!(
        out,
        "<title>{}</title>",
        xml_escape_and_strip_ctrl(&title_text)
    );
    out.push_str("<style>\n");
    out.push_str(VIEWER_CSS);
    out.push_str("\n</style>\n</head>\n<body>\n<div class=\"app\">\n");

    // Topbar
    out.push_str("<header class=\"topbar\">\n");
    out.push_str("  <div class=\"topbar-group\">\n");
    out.push_str(
        "    <div class=\"brand\"><span class=\"brand-dot\"></span><span>lineage</span></div>\n",
    );
    let _ = writeln!(
        out,
        "    <span class=\"meta-text\">{} · {} nodes · {} edges</span>",
        dir_str,
        graph.nodes.len(),
        graph.edges.len()
    );
    if opts.include_snippets {
        out.push_str(
            "    <span id=\"snippet-pill\" class=\"pill pill-on\">Snippets: ON (local)</span>\n",
        );
    } else {
        out.push_str(
            "    <span id=\"snippet-pill\" class=\"pill pill-off\">Snippets: OFF</span>\n",
        );
    }
    for badge in &graph.completeness.badges {
        let _ = writeln!(
            out,
            "    <span class=\"pill pill-warn\">{}</span>",
            xml_escape_and_strip_ctrl(badge)
        );
    }
    if graph.truncated_nodes > 0 {
        let _ = writeln!(
            out,
            "    <span class=\"pill pill-warn\">Showing {} of {} nodes</span>",
            graph.nodes.len(),
            graph.nodes.len() + graph.truncated_nodes
        );
    }
    if graph.null_only {
        out.push_str("    <span class=\"pill pill-locked\">Exported: Null-only</span>\n");
    }
    if graph.exclude_tests {
        out.push_str("    <span class=\"pill pill-locked\">Tests excluded</span>\n");
    }
    out.push_str("  </div>\n");

    out.push_str("  <div class=\"topbar-group\">\n");
    out.push_str("    <button type=\"button\" id=\"btn-null-only\"><span class=\"filter-dot dot-nullable\"></span><span class=\"filter-label\">Null-only</span></button>\n");
    out.push_str("    <button type=\"button\" id=\"btn-guards\"><span class=\"filter-dot dot-guard\"></span><span class=\"filter-label\">Guards</span></button>\n");
    out.push_str("    <button type=\"button\" id=\"btn-unverified\"><span class=\"filter-dot dot-unverified\"></span><span class=\"filter-label\">Unverified</span></button>\n");
    out.push_str("    <span class=\"divider-v\"></span>\n");
    out.push_str("    <button type=\"button\" id=\"btn-theme\">Theme</button>\n");
    out.push_str(
        "    <button type=\"button\" id=\"btn-save-no-snippets\">Save Without Snippets</button>\n",
    );
    out.push_str("  </div>\n</header>\n");

    let mut file_node_counts: BTreeMap<usize, usize> = BTreeMap::new();
    for n in &graph.nodes {
        *file_node_counts.entry(n.file_index).or_insert(0) += 1;
    }

    // Workspace: Legend + SVG Canvas + Inspector
    out.push_str("<main class=\"workspace\">\n");
    out.push_str("  <aside id=\"legend-panel\" class=\"legend-panel\">\n");
    let _ = writeln!(
        out,
        "    <div class=\"legend-header\"><span class=\"legend-title\">Files ({})</span><button type=\"button\" id=\"btn-legend-toggle\" title=\"Toggle legend\">≡</button></div>",
        graph.files.len()
    );
    out.push_str("    <div class=\"legend-list\">\n");
    for (idx, f) in graph.files.iter().enumerate() {
        let cidx = idx % 12;
        let fcount = file_node_counts.get(&idx).copied().unwrap_or(0);
        let _ = writeln!(
            out,
            "      <button type=\"button\" class=\"legend-item\" data-file-index=\"{idx}\" title=\"{path}\"><span class=\"legend-item-left\"><span class=\"legend-swatch file-accent-{cidx}\"></span><span class=\"legend-path\">{short}</span></span><span class=\"file-count\">{fcount}</span></button>",
            path = xml_escape_and_strip_ctrl(&f.path),
            short = xml_escape_and_strip_ctrl(&middle_elide(&f.path, 24)),
        );
    }
    out.push_str("    </div>\n");
    out.push_str("    <div class=\"legend-footer\">\n");
    out.push_str("      <span class=\"legend-status-item\"><span class=\"status-dot dot-nonnull\"></span><span>✓ NonNull</span></span>\n");
    out.push_str("      <span class=\"legend-status-item\"><span class=\"status-dot dot-nullable\"></span><span>◐ Nullable</span></span>\n");
    out.push_str("      <span class=\"legend-status-item\"><span class=\"status-dot dot-absent\"></span><span>∅ Absent</span></span>\n");
    out.push_str("      <span class=\"legend-status-item\"><span class=\"status-dot dot-unverified\"></span><span>? Unverified</span></span>\n");
    out.push_str("      <span class=\"legend-status-item\"><span class=\"status-dot dot-guard\"></span><span>◇ Guard</span></span>\n");
    out.push_str("      <span class=\"legend-status-item\"><span class=\"status-dot dot-danger\"></span><span>⚠ Risk</span></span>\n");
    out.push_str("      <span class=\"legend-status-item\"><span class=\"status-dot dot-frontier\"></span><span>⋯ Frontier</span></span>\n");
    out.push_str("      <span class=\"legend-status-item\"><span class=\"legend-line\"></span><span>↻ Cycle</span></span>\n");
    out.push_str("    </div>\n  </aside>\n");

    out.push_str("  <div id=\"canvas-wrap\" class=\"canvas-wrap\">\n");
    let _ = writeln!(
        out,
        "    <svg id=\"graph-svg\" viewBox=\"0 0 {} {}\" width=\"100%\" height=\"100%\">",
        layout.view_width, layout.view_height
    );
    out.push_str("      <defs>\n");
    for arrow_id in [
        "arrow-nonnull",
        "arrow-nullable",
        "arrow-absent",
        "arrow-unverified",
        "arrow-danger",
        "arrow-cycle",
        "arrow-overflow",
    ] {
        let _ = writeln!(
            out,
            "        <marker id=\"{arrow_id}\" viewBox=\"0 0 10 10\" refX=\"8\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto\"><path d=\"M 0 1 L 10 5 L 0 9 z\"/></marker>"
        );
    }
    for n in &graph.nodes {
        let _ = writeln!(
            out,
            "        <clipPath id=\"clip-{id}\"><rect x=\"10\" y=\"25\" width=\"220\" height=\"44\"/></clipPath>",
            id = xml_escape_and_strip_ctrl(&n.id)
        );
    }
    out.push_str("      </defs>\n");
    out.push_str("      <g id=\"viewport\">\n");

    if let Some(fy) = layout.focal_band_y {
        let x2 = (layout.view_width - 16).max(320);
        let up_y = fy - 14;
        let dn_y = fy + CARD_H + 14;
        let _ = writeln!(
            out,
            "        <line class=\"band-line\" x1=\"16\" y1=\"{up_y}\" x2=\"{x2}\" y2=\"{up_y}\"/>"
        );
        let _ = writeln!(
            out,
            "        <rect class=\"band-pill-bg\" x=\"16\" y=\"{}\" width=\"136\" height=\"20\" rx=\"10\"/>",
            up_y - 10
        );
        let _ = writeln!(
            out,
            "        <text class=\"band-label\" x=\"84\" y=\"{}\" text-anchor=\"middle\">▲ Upstream origins</text>",
            up_y + 4
        );
        let _ = writeln!(
            out,
            "        <line class=\"band-line\" x1=\"16\" y1=\"{dn_y}\" x2=\"{x2}\" y2=\"{dn_y}\"/>"
        );
        let _ = writeln!(
            out,
            "        <rect class=\"band-pill-bg\" x=\"16\" y=\"{}\" width=\"164\" height=\"20\" rx=\"10\"/>",
            dn_y - 10
        );
        let _ = writeln!(
            out,
            "        <text class=\"band-label\" x=\"98\" y=\"{}\" text-anchor=\"middle\">▼ Downstream consumers</text>",
            dn_y + 4
        );
    }

    let node_by_id: BTreeMap<&str, &ExportNode> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    out.push_str("        <g id=\"edges-layer\">\n");
    for (edge_idx, le) in layout.edges.iter().enumerate() {
        if le.waypoints.is_empty() {
            continue;
        }
        let d = match le.route_kind {
            RouteKind::Forward => smooth_forward_path(&le.waypoints),
            RouteKind::CycleGutter | RouteKind::OverflowLeft | RouteKind::SelfLoop => {
                rounded_orthogonal_path(&le.waypoints, 10)
            }
        };
        let from_node = node_by_id.get(le.from.as_str()).copied();
        let to_node = node_by_id.get(le.to.as_str()).copied();
        let raw_label = graph.edges.get(edge_idx).and_then(|e| e.label.as_deref());
        let (cls, marker) = edge_style(le.route_kind, from_node, to_node);
        let pill_svg = edge_intent_pill(le.route_kind, &le.waypoints, raw_label, to_node);
        let _ = writeln!(
            out,
            "          <g class=\"edge-group\" id=\"edge-{id}\" data-edge-id=\"{id}\" data-from=\"{from}\" data-to=\"{to}\"><path class=\"{cls}\" d=\"{d}\" marker-end=\"{marker}\"/>{pill_svg}</g>",
            id = xml_escape_and_strip_ctrl(&le.id),
            from = xml_escape_and_strip_ctrl(&le.from),
            to = xml_escape_and_strip_ctrl(&le.to),
        );
    }
    out.push_str("        </g>\n");

    out.push_str("        <g id=\"nodes-layer\">\n");
    if layout.empty_state {
        out.push_str("          <text class=\"empty-label\" x=\"320\" y=\"120\" text-anchor=\"middle\">No flow nodes</text>\n");
    } else {
        for ln in &layout.nodes {
            if let Some(&n) = node_by_id.get(ln.id.as_str()) {
                render_svg_node(&mut out, n, ln);
            }
        }
    }
    out.push_str("        </g>\n");
    out.push_str("        <g id=\"active-overlay\"></g>\n");
    out.push_str("      </g>\n    </svg>\n");
    out.push_str(
        "    <div class=\"hint-pill\">Click any node to isolate causal cone · Esc to clear</div>\n",
    );
    out.push_str("    <div class=\"canvas-controls\">\n");
    out.push_str(
        "      <button type=\"button\" id=\"btn-zoom-out\" title=\"Zoom out\">−</button>\n",
    );
    out.push_str("      <button type=\"button\" id=\"btn-zoom-in\" title=\"Zoom in\">+</button>\n");
    out.push_str("      <span class=\"divider-v\"></span>\n");
    out.push_str("      <button type=\"button\" id=\"btn-fit-view\">Fit View</button>\n");
    out.push_str("      <button type=\"button\" id=\"btn-fit-active\">Fit Active</button>\n");
    out.push_str("      <button type=\"button\" id=\"btn-center-focus\">Center Focus</button>\n");
    out.push_str("      <button type=\"button\" id=\"btn-reset\">Reset</button>\n");
    out.push_str("    </div>\n");
    out.push_str("  </div>\n");
    out.push_str("  <aside id=\"inspector\" class=\"inspector\"></aside>\n");
    out.push_str("</main>\n</div>\n");

    let _ = writeln!(
        out,
        "<script type=\"application/json\" id=\"lineage-meta\">{meta_json}</script>"
    );
    let _ = writeln!(
        out,
        "<script type=\"application/json\" id=\"lineage-snippets\">{snippets_json}</script>"
    );
    out.push_str("<script>\n");
    out.push_str(VIEWER_JS);
    out.push_str("\n</script>\n</body>\n</html>\n");
    out
}

fn node_visual_state(node: &ExportNode) -> (&'static str, &'static str, &'static str, i32) {
    if node.is_danger_sink {
        ("state-danger", "pill-danger", "⚠ Risk", 64)
    } else if node.is_frontier {
        ("state-frontier", "pill-frontier", "⋯ Frontier", 78)
    } else if node.is_guard {
        ("state-guard", "pill-guard", "◇ Guard", 72)
    } else if matches!(node.absence_class, AbsenceClass::Absent) {
        ("state-absent", "pill-absent", "∅ Absent", 76)
    } else if matches!(node.absence_class, AbsenceClass::Nullable) && node.is_unverified {
        (
            "state-nullable-unverified",
            "pill-nullable-unverified",
            "◐? Nullable",
            88,
        )
    } else if matches!(node.absence_class, AbsenceClass::Nullable) {
        ("state-nullable", "pill-nullable", "◐ Nullable", 82)
    } else if node.is_unverified {
        ("state-unverified", "pill-unverified", "? Unverified", 88)
    } else {
        ("state-nonnull", "pill-nonnull", "✓ NonNull", 78)
    }
}

fn edge_style(
    route_kind: RouteKind,
    from_node: Option<&ExportNode>,
    to_node: Option<&ExportNode>,
) -> (&'static str, &'static str) {
    match route_kind {
        RouteKind::CycleGutter | RouteKind::SelfLoop => {
            ("edge-path edge-sem-cycle", "url(#arrow-cycle)")
        }
        RouteKind::OverflowLeft => ("edge-path edge-sem-overflow", "url(#arrow-overflow)"),
        RouteKind::Forward => {
            if to_node.is_some_and(|n| n.is_danger_sink) {
                ("edge-path edge-sem-danger", "url(#arrow-danger)")
            } else if from_node.is_some_and(|n| n.is_guard)
                && to_node.is_some_and(|n| {
                    matches!(n.absence_class, AbsenceClass::NonNull) && !n.is_unverified
                })
            {
                ("edge-path edge-sem-nonnull", "url(#arrow-nonnull)")
            } else if from_node.is_some_and(|n| n.is_unverified || n.is_frontier)
                || to_node.is_some_and(|n| n.is_frontier)
            {
                ("edge-path edge-sem-unverified", "url(#arrow-unverified)")
            } else if let Some(src) = from_node {
                match src.absence_class {
                    AbsenceClass::Absent => ("edge-path edge-sem-absent", "url(#arrow-absent)"),
                    AbsenceClass::Nullable => {
                        ("edge-path edge-sem-nullable", "url(#arrow-nullable)")
                    }
                    AbsenceClass::NonNull => ("edge-path edge-sem-nonnull", "url(#arrow-nonnull)"),
                }
            } else {
                ("edge-path", "url(#arrow-overflow)")
            }
        }
    }
}

fn smooth_forward_path(wps: &[(i32, i32)]) -> String {
    if wps.len() < 2 {
        return String::new();
    }
    let mut d = String::with_capacity(wps.len() * 24);
    let (x0, y0) = wps[0];
    let _ = write!(d, "M {x0} {y0}");
    for seg in wps.windows(2) {
        let (ax, ay) = seg[0];
        let (bx, by) = seg[1];
        if ax == bx {
            let _ = write!(d, " L {bx} {by}");
        } else {
            let mid_y = (ay + by) / 2;
            let _ = write!(d, " C {ax} {mid_y} {bx} {mid_y} {bx} {by}");
        }
    }
    d
}

fn rounded_orthogonal_path(wps: &[(i32, i32)], radius: i32) -> String {
    if wps.is_empty() {
        return String::new();
    }
    if wps.len() <= 2 {
        let mut d = String::new();
        for (i, &(wx, wy)) in wps.iter().enumerate() {
            if i == 0 {
                let _ = write!(d, "M {wx} {wy}");
            } else {
                let _ = write!(d, " L {wx} {wy}");
            }
        }
        return d;
    }
    let mut d = String::with_capacity(wps.len() * 28);
    let _ = write!(d, "M {} {}", wps[0].0, wps[0].1);
    for i in 1..(wps.len() - 1) {
        let (px, py) = wps[i - 1];
        let (cx, cy) = wps[i];
        let (nx, ny) = wps[i + 1];
        let dx1 = cx - px;
        let dy1 = cy - py;
        let dx2 = nx - cx;
        let dy2 = ny - cy;
        let l1 = dx1.abs() + dy1.abs();
        let l2 = dx2.abs() + dy2.abs();
        let cross = dx1 * dy2 - dy1 * dx2;
        if cross == 0 || l1 == 0 || l2 == 0 {
            let _ = write!(d, " L {cx} {cy}");
            continue;
        }
        let r = radius.min(l1 / 2).min(l2 / 2);
        if r <= 0 {
            let _ = write!(d, " L {cx} {cy}");
            continue;
        }
        let bx = cx - dx1.signum() * r;
        let by = cy - dy1.signum() * r;
        let ax = cx + dx2.signum() * r;
        let ay = cy + dy2.signum() * r;
        let _ = write!(d, " L {bx} {by} Q {cx} {cy} {ax} {ay}");
    }
    let last = wps[wps.len() - 1];
    let _ = write!(d, " L {} {}", last.0, last.1);
    d
}

fn edge_intent_pill(
    route_kind: RouteKind,
    wps: &[(i32, i32)],
    raw_label: Option<&str>,
    to_node: Option<&ExportNode>,
) -> String {
    let label_text = match route_kind {
        RouteKind::CycleGutter => match raw_label.map(str::trim).filter(|s| !s.is_empty()) {
            Some(l) => format!("↻ {}", middle_elide(l, 12)),
            None => "↻ cycle".to_string(),
        },
        RouteKind::SelfLoop => "↻ self".to_string(),
        RouteKind::Forward | RouteKind::OverflowLeft => {
            let Some(l) = raw_label.map(str::trim).filter(|s| !s.is_empty()) else {
                return String::new();
            };
            middle_elide(l, 14)
        }
    };

    if wps.len() < 2 {
        return String::new();
    }
    let (mx, my) = match route_kind {
        RouteKind::CycleGutter | RouteKind::OverflowLeft if wps.len() >= 4 => {
            (wps[2].0, (wps[2].1 + wps[3].1) / 2)
        }
        RouteKind::SelfLoop if wps.len() >= 3 => (wps[1].0, (wps[1].1 + wps[2].1) / 2),
        _ if wps.len() == 2 => ((wps[0].0 + wps[1].0) / 2, (wps[0].1 + wps[1].1) / 2),
        _ => {
            let mid = wps.len() / 2;
            (
                (wps[mid - 1].0 + wps[mid].0) / 2,
                (wps[mid - 1].1 + wps[mid].1) / 2,
            )
        }
    };

    let char_len = label_text.chars().count() as i32;
    let pw = (char_len * 6 + 14).clamp(32, 108);
    let px = mx - pw / 2;
    let py = my - 9;
    let txt_cls = match route_kind {
        RouteKind::CycleGutter | RouteKind::SelfLoop => "edge-pill-txt pill-txt-cycle",
        _ if to_node.is_some_and(|n| n.is_danger_sink) => "edge-pill-txt pill-txt-danger",
        _ => "edge-pill-txt",
    };
    format!(
        "<rect class=\"edge-pill-bg\" x=\"{px}\" y=\"{py}\" width=\"{pw}\" height=\"18\" rx=\"9\"/><text class=\"{txt_cls}\" x=\"{mx}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
        my + 3,
        xml_escape_and_strip_ctrl(&label_text)
    )
}

fn node_role_label(node: &ExportNode) -> String {
    let base = if node.is_guard {
        "guard"
    } else if node.is_merge {
        "merge"
    } else if node.is_frontier {
        "frontier"
    } else if node.provenance.contains("param") {
        "param"
    } else if node.label.contains('(')
        || node.provenance.contains("call")
        || node.provenance.contains("return")
    {
        "fn"
    } else if node.is_unverified {
        "external"
    } else {
        "val"
    };
    if node.in_cycle {
        format!("{base} · ↻")
    } else {
        base.to_string()
    }
}

fn render_svg_node(out: &mut String, node: &ExportNode, ln: &crate::export::layout::LayoutNode) {
    let (state_cls, pill_cls, pill_label, pill_w) = node_visual_state(node);
    let id_esc = xml_escape_and_strip_ctrl(&node.id);
    let color_idx = node.file_index % 12;

    let _ = writeln!(
        out,
        "          <g class=\"node-group\" id=\"node-{id_esc}\" data-node-id=\"{id_esc}\" data-file-index=\"{}\" transform=\"translate({},{})\">",
        node.file_index, ln.x, ln.y
    );
    if node.is_root {
        let _ = writeln!(
            out,
            "            <rect class=\"node-halo\" x=\"-4\" y=\"-4\" width=\"{}\" height=\"{}\" rx=\"13\"/>",
            CARD_W + 8,
            CARD_H + 8
        );
    }
    let _ = writeln!(
        out,
        "            <rect class=\"node-bg {state_cls}\" x=\"0\" y=\"0\" width=\"{CARD_W}\" height=\"{CARD_H}\" rx=\"10\"/>"
    );
    let _ = writeln!(
        out,
        "            <rect class=\"node-tint {state_cls}\" x=\"0\" y=\"0\" width=\"{CARD_W}\" height=\"{CARD_H}\" rx=\"10\"/>"
    );

    // Top-left: FOCUS pill on focal node, or concise role micro-label
    if node.is_root {
        out.push_str(
            "            <rect class=\"focus-pill-bg\" x=\"10\" y=\"6\" width=\"60\" height=\"17\" rx=\"8.5\"/>\n",
        );
        out.push_str(
            "            <text class=\"focus-pill-text\" x=\"40\" y=\"18\" text-anchor=\"middle\">◎ FOCUS</text>\n",
        );
    } else {
        let role = node_role_label(node);
        let _ = writeln!(
            out,
            "            <text class=\"node-role\" x=\"12\" y=\"18\">{}</text>",
            xml_escape_and_strip_ctrl(&role)
        );
    }

    // Top-right: self-explanatory state pill
    let pill_x = CARD_W - 10 - pill_w;
    let pill_cx = pill_x + pill_w / 2;
    let _ = writeln!(
        out,
        "            <rect class=\"state-pill-bg {pill_cls}\" x=\"{pill_x}\" y=\"6\" width=\"{pill_w}\" height=\"17\" rx=\"8.5\"/>"
    );
    let _ = writeln!(
        out,
        "            <text class=\"state-pill-text {pill_cls}\" x=\"{pill_cx}\" y=\"18\" text-anchor=\"middle\">{pill_label}</text>"
    );

    // Clipped variable symbol + 7px file color dot + file:line
    let sym = xml_escape_and_strip_ctrl(&middle_elide(&node.label, 28));
    let loc = xml_escape_and_strip_ctrl(&middle_elide(
        &format!("{}:{}", node.norm_path, node.range.start.line + 1),
        29,
    ));
    let _ = writeln!(
        out,
        "            <g clip-path=\"url(#clip-{id_esc})\"><text class=\"node-label\" x=\"12\" y=\"41\">{sym}</text><circle class=\"file-accent-{color_idx}\" cx=\"15.5\" cy=\"54.5\" r=\"3.5\"/><text class=\"node-loc\" x=\"24\" y=\"58\">{loc}</text></g>"
    );
    out.push_str("          </g>\n");
}
