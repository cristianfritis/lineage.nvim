use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::cache::{path_to_uri, DirtyBufferOverlay, FileIrCache};
use lineage_engine::export::html::{
    middle_elide, render_html, safe_json_for_script, xml_escape_and_strip_ctrl,
};
use lineage_engine::export::ir::{
    AbsenceClass, ExportCompleteness, ExportEdge, ExportFile, ExportGraph, ExportNode,
    NodePartition,
};
use lineage_engine::export::layout::{
    compute_layout, RouteKind, SvgLayout, ARROW_PULLBACK, CARD_H,
};
use lineage_engine::export::ExportOptions;
use lineage_engine::protocol::schema::{
    AbsenceSummary, CanonicalPos, CanonicalRange, ExportFormat, ExportResult, ExportSourceMode,
    FileEntry, FlowNode, NodeKind, Phase2Status, RpcId, RpcRequest, RpcResponse, SliceDirection,
    SliceEnvelope, SliceGraph, SliceProjections, SliceStats, SCHEMA_VERSION,
};
use lineage_engine::protocol::transport::{handle_request, TransportState};
use lineage_engine::resolver::NullLspProvider;
use lineage_engine::slicer::SlicerEngine;

fn default_html_opts(include_snippets: bool) -> ExportOptions {
    ExportOptions::for_format(
        ExportFormat::Html,
        None,
        false,
        Some(include_snippets),
        false,
        false,
        500,
        Phase2Status::NotNeeded,
        ExportSourceMode::FreshSlice,
        0,
        None,
    )
}

fn make_node(id: &str, file_idx: usize, line: u32, label: &str, root: bool) -> ExportNode {
    ExportNode {
        id: id.to_string(),
        raw_id: id.to_string(),
        kind: NodeKind::SourceNode,
        partition: NodePartition::Upstream,
        file_index: file_idx,
        norm_path: format!("src/mod_{file_idx}.ts"),
        range: CanonicalRange {
            start: CanonicalPos { line, col: 0 },
            end: CanonicalPos { line, col: 10 },
        },
        label: label.to_string(),
        snippet: Some(format!("const {label} = source();")),
        absence_class: AbsenceClass::NonNull,
        maybe_null: false,
        maybe_undefined: false,
        is_unverified: false,
        unknown_reasons: Vec::new(),
        is_guard: false,
        guard_text: None,
        is_merge: false,
        is_danger_sink: false,
        danger_kind: None,
        risk_messages: Vec::new(),
        is_frontier: false,
        is_root: root,
        in_cycle: false,
        provenance: "local".to_string(),
    }
}

fn make_edge(from: &str, to: &str, is_back_edge: bool) -> ExportEdge {
    ExportEdge {
        from: from.to_string(),
        to: to.to_string(),
        label: None,
        is_back_edge,
        cycle_group_id: is_back_edge.then_some(0),
        provenance: "local".to_string(),
    }
}

fn empty_completeness() -> ExportCompleteness {
    ExportCompleteness {
        source_mode: ExportSourceMode::FreshSlice,
        phase2_status: Phase2Status::NotNeeded,
        phase2_wait_ms: 0,
        badges: Vec::new(),
        truncation: Vec::new(),
    }
}

/// Checks if a segment (x1, y1)->(x2, y2) strictly enters the interior of box [bx1, bx2] x [by1, by2].
fn segment_intersects_box_interior(
    p1: (i32, i32),
    p2: (i32, i32),
    bmin: (i32, i32),
    bmax: (i32, i32),
) -> bool {
    let (x1, y1) = p1;
    let (x2, y2) = p2;
    let (bx1, by1) = bmin;
    let (bx2, by2) = bmax;
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;
    let dx = (x2 - x1) as f64;
    let dy = (y2 - y1) as f64;
    let checks = [
        (-dx, (x1 - bx1) as f64),
        (dx, (bx2 - x1) as f64),
        (-dy, (y1 - by1) as f64),
        (dy, (by2 - y1) as f64),
    ];
    for (p, q) in checks {
        if p.abs() < 1e-9 {
            if q <= 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return false;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return false;
                }
                if r < t1 {
                    t1 = r;
                }
            }
        }
    }
    t0 < t1 - 1e-6
}

fn verify_geometry(layout: &SvgLayout) {
    assert!(layout.view_width > 0 && layout.view_height > 0);
    // 1. Zero node-node AABB overlaps
    for i in 0..layout.nodes.len() {
        let a = &layout.nodes[i];
        assert!(a.x >= 0 && a.y >= 0);
        for j in (i + 1)..layout.nodes.len() {
            let b = &layout.nodes[j];
            let x_overlap = a.x < b.x + b.w && a.x + a.w > b.x;
            let y_overlap = a.y < b.y + b.h && a.y + a.h > b.y;
            assert!(
                !(x_overlap && y_overlap),
                "Node overlap between {} ({},{}) and {} ({},{})",
                a.id,
                a.x,
                a.y,
                b.id,
                b.x,
                b.y
            );
        }
    }

    let node_map: BTreeMap<&str, &lineage_engine::export::layout::LayoutNode> =
        layout.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // 2. Monotonic ranks for Forward edges + zero segment-vs-card-AABB intersections + ARROW_PULLBACK
    for e in &layout.edges {
        let u = node_map[e.from.as_str()];
        let v = node_map[e.to.as_str()];
        if e.route_kind == RouteKind::Forward {
            assert!(
                u.rank < v.rank,
                "Forward edge {} -> {} violates rank order ({} >= {})",
                e.from,
                e.to,
                u.rank,
                v.rank
            );
        }
        assert!(
            e.waypoints.len() >= 2,
            "Edge {} -> {} missing waypoints",
            e.from,
            e.to
        );

        let last = *e.waypoints.last().unwrap();
        if e.route_kind == RouteKind::SelfLoop {
            assert_eq!(last.0, v.x + v.w + ARROW_PULLBACK);
        } else {
            assert_eq!(last.1, v.y - ARROW_PULLBACK);
        }

        for seg in e.waypoints.windows(2) {
            let (x1, y1) = seg[0];
            let (x2, y2) = seg[1];
            for card in &layout.nodes {
                assert!(
                    !segment_intersects_box_interior(
                        (x1, y1),
                        (x2, y2),
                        (card.x, card.y),
                        (card.x + card.w, card.y + card.h)
                    ),
                    "Edge {} -> {} segment ({},{})->({},{}) intersects card {} [{},{},{},{}]",
                    e.from,
                    e.to,
                    x1,
                    y1,
                    x2,
                    y2,
                    card.id,
                    card.x,
                    card.y,
                    card.x + card.w,
                    card.y + card.h
                );
            }
        }
    }
}

#[test]
fn test_empty_and_single_node_and_self_loop() {
    let opts = default_html_opts(false);
    let empty_graph = ExportGraph {
        direction: SliceDirection::To,
        root_ids: Vec::new(),
        files: Vec::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
        unknown_boundaries: Vec::new(),
        completeness: empty_completeness(),
        null_only: false,
        exclude_tests: false,
        truncated_nodes: 0,
        truncated_edges: 0,
    };
    let empty_layout = compute_layout(&empty_graph, &opts);
    assert!(empty_layout.empty_state);
    assert_eq!(empty_layout.view_width, 640);
    assert_eq!(empty_layout.view_height, 240);

    let single_graph = ExportGraph {
        direction: SliceDirection::To,
        root_ids: vec!["N0".into()],
        files: vec![ExportFile {
            file_id: 0,
            path: "src/a.ts".into(),
            is_external: false,
        }],
        nodes: vec![make_node("N0", 0, 1, "selfNode", true)],
        edges: vec![make_edge("N0", "N0", true)],
        unknown_boundaries: Vec::new(),
        completeness: empty_completeness(),
        null_only: false,
        exclude_tests: false,
        truncated_nodes: 0,
        truncated_edges: 0,
    };
    let single_layout = compute_layout(&single_graph, &opts);
    assert!(single_layout.trivial_single);
    verify_geometry(&single_layout);
    assert_eq!(single_layout.edges[0].route_kind, RouteKind::SelfLoop);
}

#[test]
fn test_multi_node_cycle_and_wcc_stacking() {
    let opts = default_html_opts(false);
    let graph = ExportGraph {
        direction: SliceDirection::To,
        root_ids: vec!["N0".into()],
        files: vec![ExportFile {
            file_id: 0,
            path: "src/cyc.ts".into(),
            is_external: false,
        }],
        nodes: vec![
            make_node("N0", 0, 1, "a", true),
            make_node("N1", 0, 2, "b", false),
            make_node("N2", 0, 3, "c", false),
            make_node("N3", 0, 10, "wcc2_a", false),
            make_node("N4", 0, 11, "wcc2_b", false),
        ],
        edges: vec![
            make_edge("N0", "N1", false),
            make_edge("N1", "N2", false),
            make_edge("N2", "N0", true),
            make_edge("N3", "N4", false),
        ],
        unknown_boundaries: Vec::new(),
        completeness: empty_completeness(),
        null_only: false,
        exclude_tests: false,
        truncated_nodes: 0,
        truncated_edges: 0,
    };
    let layout = compute_layout(&graph, &opts);
    verify_geometry(&layout);
    let nmap: BTreeMap<&str, _> = layout.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    assert!(nmap["N0"].rank < nmap["N1"].rank);
    assert!(nmap["N1"].rank < nmap["N2"].rank);
    // WCC2 stacked strictly below WCC1
    assert!(nmap["N3"].y > nmap["N2"].y + CARD_H);
    assert_eq!(layout.edges[2].route_kind, RouteKind::CycleGutter);
}

#[test]
fn test_both_waist_pinning_and_fallbacks() {
    let opts = default_html_opts(false);
    let both_graph = ExportGraph {
        direction: SliceDirection::Both,
        root_ids: vec!["N1".into()],
        files: vec![ExportFile {
            file_id: 0,
            path: "src/both.ts".into(),
            is_external: false,
        }],
        nodes: vec![
            make_node("N0", 0, 1, "upstream", false),
            make_node("N1", 0, 2, "focal", true),
            make_node("N2", 0, 3, "downstream", false),
        ],
        edges: vec![make_edge("N0", "N1", false), make_edge("N1", "N2", false)],
        unknown_boundaries: Vec::new(),
        completeness: empty_completeness(),
        null_only: false,
        exclude_tests: false,
        truncated_nodes: 0,
        truncated_edges: 0,
    };
    let layout = compute_layout(&both_graph, &opts);
    verify_geometry(&layout);
    assert!(layout.focal_band_y.is_some());
    let nmap: BTreeMap<&str, _> = layout.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    assert!(nmap["N0"].rank < nmap["N1"].rank);
    assert!(nmap["N1"].rank < nmap["N2"].rank);

    // Fallback when root_ids is empty (e.g., pruned by --null-only)
    let mut pruned_graph = both_graph.clone();
    pruned_graph.root_ids.clear();
    let pruned_layout = compute_layout(&pruned_graph, &opts);
    verify_geometry(&pruned_layout);
    assert!(pruned_layout.focal_band_y.is_none());

    // Fallback when inter-root DAG edge exists
    let mut inter_root = both_graph;
    inter_root.root_ids = vec!["N0".into(), "N1".into()];
    let inter_layout = compute_layout(&inter_root, &opts);
    verify_geometry(&inter_layout);
    assert!(inter_layout.focal_band_y.is_none());
}

#[test]
fn test_600_dummy_cap_overflow_and_200_node_perf() {
    let opts = default_html_opts(false);
    let mut nodes = Vec::new();
    let mut files = Vec::new();
    for f in 0..15 {
        files.push(ExportFile {
            file_id: f as u32,
            path: format!("src/pkg_{f}/file.ts"),
            is_external: false,
        });
    }
    for i in 0..200 {
        nodes.push(make_node(
            &format!("N{i}"),
            i % 15,
            i as u32 + 1,
            &format!("sym_{i}"),
            i == 0,
        ));
    }
    let mut edges = Vec::new();
    // Chain of 200 ranks -> span 199
    for i in 0..199 {
        edges.push(make_edge(&format!("N{i}"), &format!("N{}", i + 1), false));
    }
    // Add long-span edges exceeding 600 dummy waypoints (up to 800 edges total)
    let mut added = edges.len();
    for i in 0..180 {
        for jump in [20usize, 40, 60] {
            if i + jump < 200 && added < 790 {
                edges.push(make_edge(
                    &format!("N{i}"),
                    &format!("N{}", i + jump),
                    false,
                ));
                added += 1;
            }
        }
    }
    // Plus cycle back-edges
    edges.push(make_edge("N150", "N10", true));
    edges.push(make_edge("N180", "N5", true));

    let graph = ExportGraph {
        direction: SliceDirection::To,
        root_ids: vec!["N0".into()],
        files,
        nodes,
        edges,
        unknown_boundaries: Vec::new(),
        completeness: empty_completeness(),
        null_only: false,
        exclude_tests: false,
        truncated_nodes: 0,
        truncated_edges: 0,
    };

    let t0 = Instant::now();
    let layout = compute_layout(&graph, &opts);
    let elapsed_ms = t0.elapsed().as_millis();

    assert!(
        elapsed_ms < 50,
        "200-node / ~800-edge layout took {elapsed_ms}ms (must be < 50ms)"
    );
    assert!(layout.metrics.dummy_nodes_added <= 600);
    assert!(layout.metrics.dummy_cap_hit);
    assert!(layout.metrics.overflow_edges > 0);
    assert!(layout
        .edges
        .iter()
        .any(|e| e.route_kind == RouteKind::OverflowLeft));
    verify_geometry(&layout);
}

#[test]
fn test_html_security_csp_escaping_palette_and_snippets() {
    let evil = "</script><img onerror=alert(1)>\"'&\x00\x07\u{2028}\u{2029}";
    let mut n0 = make_node("N0", 0, 1, evil, true);
    n0.guard_text = Some(evil.to_string());
    n0.snippet = Some(format!("let x = {evil};"));
    let mut n1 = make_node("N1", 1, 2, "consumer", false);
    n1.absence_class = AbsenceClass::Nullable;
    n1.maybe_null = true;
    n1.is_unverified = true;

    let mut files = Vec::new();
    for i in 0..14 {
        files.push(ExportFile {
            file_id: i as u32,
            path: format!("src/f{i:02}.ts"),
            is_external: false,
        });
    }
    let graph = ExportGraph {
        direction: SliceDirection::To,
        root_ids: vec!["N0".into()],
        files,
        nodes: vec![n0, n1],
        edges: vec![make_edge("N0", "N1", false)],
        unknown_boundaries: Vec::new(),
        completeness: empty_completeness(),
        null_only: false,
        exclude_tests: false,
        truncated_nodes: 0,
        truncated_edges: 0,
    };

    let opts_off = default_html_opts(false);
    let layout = compute_layout(&graph, &opts_off);
    let html_off = render_html(&graph, &layout, &opts_off);

    assert!(html_off.starts_with("<!DOCTYPE html>"));
    assert!(html_off.contains("<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; object-src 'none';\">"));
    assert!(!html_off.contains("frame-ancestors"));
    assert!(!html_off.contains("<img onerror"));
    // Check there is no raw </script><img inside the meta JSON island
    let meta_start = html_off.find("id=\"lineage-meta\">").unwrap();
    let meta_end = html_off[meta_start..].find("</script>").unwrap() + meta_start;
    let meta_slice = &html_off[meta_start..meta_end];
    assert!(!meta_slice.contains("</script>"));
    assert!(meta_slice.contains("\\u003c/script\\u003e"));
    assert!(meta_slice.contains("\\u2028"));
    assert!(meta_slice.contains("\\u2029"));

    // Sequential color_idx % 12 over files
    let parsed_meta: serde_json::Value =
        serde_json::from_str(&meta_slice["id=\"lineage-meta\">".len()..]).unwrap();
    assert_eq!(parsed_meta["files"][0]["color_idx"], 0);
    assert_eq!(parsed_meta["files"][12]["color_idx"], 0);
    assert_eq!(parsed_meta["files"][13]["color_idx"], 1);

    // Snippets OFF -> #lineage-snippets is {}
    let snip_start = html_off.find("id=\"lineage-snippets\">").unwrap();
    let snip_end = html_off[snip_start..].find("</script>").unwrap() + snip_start;
    assert_eq!(
        &html_off[snip_start + "id=\"lineage-snippets\">".len()..snip_end],
        "{}"
    );
    assert!(!html_off.contains("let x ="));

    // Snippets ON -> snippet only in #lineage-snippets, NEVER inside <svg>
    let opts_on = default_html_opts(true);
    let html_on = render_html(&graph, &layout, &opts_on);
    let svg_start = html_on.find("<svg ").unwrap();
    let svg_end = html_on.find("</svg>").unwrap();
    let svg_slice = &html_on[svg_start..svg_end];
    assert!(!svg_slice.contains("let x ="));
    assert!(!svg_slice.contains("marker-start="));
    assert!(!svg_slice.contains("marker-mid="));
    assert!(html_on.contains("Snippets: ON (local)"));
    assert!(!html_on.contains(env!("CARGO_PKG_VERSION")));

    // Helper checks
    assert_eq!(xml_escape_and_strip_ctrl("a\x00<b>"), "a&lt;b&gt;");
    assert_eq!(
        safe_json_for_script("<>&\u{2028}"),
        "\\u003c\\u003e\\u0026\\u2028"
    );
    assert_eq!(
        middle_elide("src/very/long/path/session.ts:42", 18)
            .chars()
            .count(),
        18
    );
}

#[test]
fn test_cli_vs_rpc_byte_identity_and_determinism() {
    let dir = std::env::temp_dir().join(format!("lineage_html_cli_rpc_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file_path = dir.join("sample.ts");
    std::fs::write(
        &file_path,
        "function src(flag: boolean) {\n  return flag ? null : 'ok';\n}\nexport function run(flag: boolean) {\n  const v = src(flag);\n  return v;\n}\n",
    )
    .unwrap();

    let engine = Arc::new(SlicerEngine::new(
        Arc::new(FileIrCache::new(16 * 1024 * 1024)),
        Arc::new(DirtyBufferOverlay::new()),
        Arc::new(NullLspProvider),
        vec![Arc::new(TypeScriptAdapter::new())],
    ));
    let (tx, rx) = crossbeam_channel::unbounded();
    let state = TransportState::new(engine, None, tx);
    let uri = path_to_uri(&file_path);

    let req = RpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(RpcId::Num(1)),
        method: "lineage/export".into(),
        params: serde_json::json!({
            "format": "html",
            "uri": uri,
            "position": { "line": 5, "character": 9 },
            "direction": "to",
            "no_lsp": true,
            "workspace_root": dir.to_string_lossy(),
        }),
    };
    handle_request(&state, req);
    let resp_line = rx.recv().unwrap();
    let resp: RpcResponse = serde_json::from_str(&resp_line).unwrap();
    let rpc_res: ExportResult = serde_json::from_value(resp.result.unwrap()).unwrap();

    // CLI invocation
    let bin = env!("CARGO_BIN_EXE_lineage-engine");
    let target = format!("{}:6:9", file_path.display());
    let cli_out = Command::new(bin)
        .args([
            "export",
            &target,
            "--format",
            "html",
            "--direction",
            "to",
            "--no-lsp",
            "--workspace-root",
            dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(cli_out.status.success());
    let cli_html = String::from_utf8(cli_out.stdout).unwrap();

    assert_eq!(
        rpc_res.content, cli_html,
        "RPC and CLI HTML output must be byte-identical"
    );

    // Ensure `slice --format html` is rejected at parse time
    let bad_slice = Command::new(bin)
        .args(["slice", &target, "--format", "html"])
        .output()
        .unwrap();
    assert!(!bad_slice.status.success());

    // Ensure --from-envelope stdin works for --format html
    let env_json = serde_json::to_string(&SliceEnvelope {
        schema_version: SCHEMA_VERSION,
        query_id: 1,
        trace_id: "t".into(),
        direction: SliceDirection::To,
        root_ids: vec!["r".into()],
        files: vec![FileEntry {
            file_id: 0,
            uri: path_to_uri(&file_path),
        }],
        graph: Some(SliceGraph {
            nodes: vec![FlowNode {
                id: "r".into(),
                kind: NodeKind::SourceNode,
                file_id: 0,
                range: CanonicalRange {
                    start: CanonicalPos { line: 0, col: 0 },
                    end: CanonicalPos { line: 0, col: 5 },
                },
                label: "r".into(),
                snippet: None,
                absence: AbsenceSummary {
                    present: true,
                    maybe_null: false,
                    maybe_undefined: false,
                    unknown_reasons: Vec::new(),
                },
                provenance: "local".into(),
                is_guard: false,
                guard_text: None,
                is_danger_sink: false,
                danger_kind: None,
            }],
            edges: Vec::new(),
        }),
        projections: SliceProjections::default(),
        truncation: Vec::new(),
        stats: SliceStats::default(),
    })
    .unwrap();
    let mut child = Command::new(bin)
        .args([
            "export",
            "--from-envelope",
            "-",
            "--format",
            "html",
            "--no-lsp",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(env_json.as_bytes())
        .unwrap();
    let out_env = child.wait_with_output().unwrap();
    assert!(out_env.status.success());
    assert!(String::from_utf8(out_env.stdout)
        .unwrap()
        .contains("<!DOCTYPE html>"));

    let _ = std::fs::remove_dir_all(&dir);
}
