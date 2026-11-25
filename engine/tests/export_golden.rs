use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::unbounded;
use lineage_engine::adapters::go::GoAdapter;
use lineage_engine::adapters::python::PythonAdapter;
use lineage_engine::adapters::rust::RustAdapter;
use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::cache::{path_to_uri, uri_to_path, DirtyBufferOverlay, FileIrCache};
use lineage_engine::export::ir::ExportGraph;
use lineage_engine::export::mermaid::MERMAID_MAX_BYTES;
use lineage_engine::export::sanitize::PathSanitizer;
use lineage_engine::export::{render_export, ExportOptions};
use lineage_engine::protocol::schema::{
    AbsenceSummary, CanonicalPos, CanonicalRange, EdgeSide, ExportFormat, ExportResult,
    ExportSourceMode, FileEntry, FlowEdge, FlowNode, LspPosition, NodeKind, OffsetEncoding,
    Phase2Status, RpcId, RpcRequest, RpcResponse, SliceDirection, SliceEnvelope, SliceGraph,
    SliceParams, SliceProjections, SliceStats, SCHEMA_VERSION,
};
use lineage_engine::protocol::transport::{handle_request, TransportState};
use lineage_engine::resolver::{FixtureLspProvider, NullLspProvider};
use lineage_engine::slicer::pipeline::{run_reduction_pipeline, RawSliceEdge, RawSliceNode};
use lineage_engine::slicer::projectors::project_tree_rows;
use lineage_engine::slicer::SlicerEngine;

fn make_engine() -> Arc<SlicerEngine> {
    Arc::new(SlicerEngine::new(
        Arc::new(FileIrCache::new(16 * 1024 * 1024)),
        Arc::new(DirtyBufferOverlay::new()),
        Arc::new(NullLspProvider),
        vec![
            Arc::new(TypeScriptAdapter::new()),
            Arc::new(PythonAdapter::new()),
            Arc::new(RustAdapter::new()),
            Arc::new(GoAdapter::new()),
        ],
    ))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lineage_export_test_{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn abs_present() -> AbsenceSummary {
    AbsenceSummary {
        present: true,
        maybe_null: false,
        maybe_undefined: false,
        unknown_reasons: Vec::new(),
    }
}

fn abs_nullable() -> AbsenceSummary {
    AbsenceSummary {
        present: true,
        maybe_null: true,
        maybe_undefined: false,
        unknown_reasons: Vec::new(),
    }
}

fn abs_absent() -> AbsenceSummary {
    AbsenceSummary {
        present: false,
        maybe_null: true,
        maybe_undefined: false,
        unknown_reasons: Vec::new(),
    }
}

fn range_line(line: u32) -> CanonicalRange {
    CanonicalRange {
        start: CanonicalPos { line, col: 0 },
        end: CanonicalPos { line, col: 10 },
    }
}

#[test]
fn test_block0_uri_normalization_and_diamond_vs_cycle() {
    assert_eq!(
        uri_to_path("file:///home/user/my%20repo/src/a.ts"),
        PathBuf::from("/home/user/my repo/src/a.ts")
    );
    assert_eq!(
        uri_to_path("file:///C:/Users/dev/repo/src/a.ts"),
        PathBuf::from("C:/Users/dev/repo/src/a.ts")
    );

    // Diamond topology: R -> A -> C and R -> B -> C (no cycle!)
    let diamond_nodes = vec![
        RawSliceNode {
            id: "R".into(),
            file_id: 0,
            is_test_file: false,
            is_copy_passthrough: false,
            range: range_line(0),
            label: "R".into(),
            snippet: None,
            absence: abs_nullable(),
            provenance: "local".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        },
        RawSliceNode {
            id: "A".into(),
            file_id: 0,
            is_test_file: false,
            is_copy_passthrough: false,
            range: range_line(1),
            label: "A".into(),
            snippet: None,
            absence: abs_present(),
            provenance: "local".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        },
        RawSliceNode {
            id: "B".into(),
            file_id: 0,
            is_test_file: false,
            is_copy_passthrough: false,
            range: range_line(2),
            label: "B".into(),
            snippet: None,
            absence: abs_absent(),
            provenance: "local".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        },
        RawSliceNode {
            id: "C".into(),
            file_id: 0,
            is_test_file: false,
            is_copy_passthrough: false,
            range: range_line(3),
            label: "C".into(),
            snippet: None,
            absence: abs_absent(),
            provenance: "local".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        },
    ];
    let diamond_edges = vec![
        RawSliceEdge {
            from: "R".into(),
            to: "A".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
        RawSliceEdge {
            from: "R".into(),
            to: "B".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
        RawSliceEdge {
            from: "A".into(),
            to: "C".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
        RawSliceEdge {
            from: "B".into(),
            to: "C".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
    ];
    let out = run_reduction_pipeline(
        &["R".into()],
        &diamond_nodes,
        &diamond_edges,
        false,
        std::time::Duration::from_millis(50),
        Instant::now(),
    );
    assert!(
        out.graph
            .edges
            .iter()
            .all(|e| !e.is_back_edge && e.cycle_group_id.is_none()),
        "Diamond topology must have zero back-edges or cycle groups"
    );
    let rows = project_tree_rows(
        &["R".into()],
        &out.graph,
        &[FileEntry {
            file_id: 0,
            uri: "file:///a.ts".into(),
        }],
    );
    assert!(
        rows.iter().all(|r| !r.is_cycle),
        "Diamond fan-in must not be mislabeled [cycle] in project_tree_rows"
    );
    assert_eq!(
        rows.iter().filter(|r| r.node_id == "C").count(),
        2,
        "Diamond should render C on both branches without [cycle]"
    );

    // 3-node cycle A -> B -> C -> A
    let mut cyc_nodes = diamond_nodes[1..4].to_vec();
    cyc_nodes
        .iter_mut()
        .for_each(|n| n.is_copy_passthrough = false);
    let cyc_edges = vec![
        RawSliceEdge {
            from: "A".into(),
            to: "B".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
        RawSliceEdge {
            from: "B".into(),
            to: "C".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
        RawSliceEdge {
            from: "C".into(),
            to: "A".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
    ];
    let cyc_out = run_reduction_pipeline(
        &["A".into()],
        &cyc_nodes,
        &cyc_edges,
        false,
        std::time::Duration::from_millis(50),
        Instant::now(),
    );
    assert_eq!(cyc_out.graph.edges.len(), 3);
    assert!(
        cyc_out
            .graph
            .edges
            .iter()
            .all(|e| e.cycle_group_id.is_some()),
        "All 3 cycle edges must share an SCC cycle_group_id"
    );
    let back_count = cyc_out
        .graph
        .edges
        .iter()
        .filter(|e| e.is_back_edge)
        .count();
    assert_eq!(
        back_count, 1,
        "Only the single DFS feedback edge (C -> A) must have is_back_edge = true"
    );
}

#[test]
fn test_block0_projections_cross_file_guards_danger_and_phase2_merge() {
    let dir = temp_dir("block0_full");
    let lib_path = dir.join("helper.ts");
    let main_path = dir.join("main.ts");
    std::fs::write(
        &lib_path,
        "export function getMaybeNull(flag: boolean): string | null {\n  if (flag) return null;\n  return 'ok';\n}\n",
    )
    .unwrap();
    let main_src = concat!(
        "import { getMaybeNull } from './helper';\n",
        "export function useFlow(obj: { field: string | null }, flag: boolean) {\n",
        "  let val = obj.field;\n",
        "  if (flag) {\n",
        "    val = getMaybeNull(flag);\n",
        "  }\n",
        "  if (val !== null) {\n",
        "    return val;\n",
        "  }\n",
        "  const bad = val.toUpperCase();\n",
        "  return bad;\n",
        "}\n",
    );
    std::fs::write(&main_path, main_src).unwrap();

    let engine = make_engine();
    let main_uri = path_to_uri(&main_path);
    let cancel = AtomicBool::new(false);
    let params = SliceParams {
        uri: main_uri.clone(),
        position: LspPosition {
            line: 2,
            character: 6,
        },
        offset_encoding: OffsetEncoding::Utf8,
        direction: SliceDirection::To,
        include_graph: true,
        include_focus: true,
        include_tree: true,
        include_snippets: true,
        exclude_tests: false,
        query_id: Some(1),
        lsp_attached: false,
        lsp_client_name: None,
        workspace_root: Some(dir.to_string_lossy().to_string()),
        lsp_timeout_ms: None,
    };

    let (_, field_graph, _) = engine
        .slice_phase1_with_snapshot(&params, "tr-field", &cancel)
        .unwrap();
    assert!(
        field_graph
            .edges
            .iter()
            .any(|e| e.provenance == "project_field"),
        "Expected project_field edge in SliceGraph"
    );

    let xcall_params = SliceParams {
        position: LspPosition {
            line: 4,
            character: 10,
        },
        ..params.clone()
    };
    let (env, graph, _) = engine
        .slice_phase1_with_snapshot(&xcall_params, "tr-b0", &cancel)
        .unwrap();
    // Cross-file callee from ./helper must be emitted with file_id > 0
    assert!(
        env.files
            .iter()
            .any(|f| f.uri.contains("helper.ts") && f.file_id > 0),
        "Expected helper.ts in files table"
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|n| n.file_id > 0 && n.provenance == "cross_file_call"),
        "Expected cross-file callee node with real file_id"
    );

    // Check guard & danger_sink on separate query targeting narrowed `val` and danger sink
    let guard_params = SliceParams {
        position: LspPosition {
            line: 7,
            character: 11,
        },
        ..params.clone()
    };
    let (_, guard_graph, _) = engine
        .slice_phase1_with_snapshot(&guard_params, "tr-guard", &cancel)
        .unwrap();
    assert!(
        guard_graph
            .nodes
            .iter()
            .any(|n| n.is_guard && n.guard_text.is_some()),
        "Expected typed is_guard + guard_text on Pi node"
    );

    let danger_params = SliceParams {
        position: LspPosition {
            line: 9,
            character: 14,
        },
        ..params.clone()
    };
    let (danger_env, danger_graph, _) = engine
        .slice_phase1_with_snapshot(&danger_params, "tr-danger", &cancel)
        .unwrap();
    let card = danger_env.projections.hover_card.unwrap();
    if !card.local_risks.is_empty() {
        assert!(card.local_risks.iter().all(|r| r.node_id.is_some()));
        assert!(danger_graph
            .nodes
            .iter()
            .any(|n| n.is_danger_sink && n.danger_kind.is_some()));
    }

    // Check Phase 2 LSP merge using FixtureLspProvider
    let ext_path = dir.join("ext_impl.ts");
    std::fs::write(
        &ext_path,
        "export function extFn(): string | null {\n  return null;\n}\n",
    )
    .unwrap();
    let fixture_path = dir.join("lsp_fixture.json");
    let fixture_json = serde_json::json!({
        "rules": [
            {
                "key": "extFn",
                "kind": "definition",
                "locations": [
                    {
                        "uri": path_to_uri(&ext_path),
                        "start": { "line": 0, "character": 16 },
                        "end": { "line": 0, "character": 21 }
                    }
                ]
            }
        ]
    });
    std::fs::write(
        &fixture_path,
        serde_json::to_string_pretty(&fixture_json).unwrap(),
    )
    .unwrap();
    let resolver = FixtureLspProvider::from_json_file(&fixture_path).unwrap();
    engine.set_resolver(Arc::new(resolver));

    let ext_call_src = concat!(
        "import { extFn } from './nonexistent_pkg';\n",
        "export function caller() {\n",
        "  const res = extFn();\n",
        "  return res;\n",
        "}\n",
    );
    let call_path = dir.join("call_ext.ts");
    std::fs::write(&call_path, ext_call_src).unwrap();
    let call_params = SliceParams {
        uri: path_to_uri(&call_path),
        position: LspPosition {
            line: 3,
            character: 9,
        },
        offset_encoding: OffsetEncoding::Utf8,
        direction: SliceDirection::To,
        include_graph: true,
        include_focus: false,
        include_tree: false,
        include_snippets: false,
        exclude_tests: false,
        query_id: Some(2),
        lsp_attached: true,
        lsp_client_name: None,
        workspace_root: Some(dir.to_string_lossy().to_string()),
        lsp_timeout_ms: Some(500),
    };
    let (_, _, p2_ctx) = engine
        .slice_phase1_with_snapshot(&call_params, "tr-p2", &cancel)
        .unwrap();
    let mut ctx = p2_ctx.expect("expected Phase2Context");
    ctx.phase2_ceiling_ms = 500;
    let (delta_env, full_graph, status) = engine
        .execute_phase2_with_status(ctx, &cancel)
        .expect("expected phase2 delta");
    assert_eq!(status, Phase2Status::Complete);
    assert!(
        full_graph
            .nodes
            .iter()
            .any(|n| n.provenance == "lsp_definition"),
        "Phase 2 must merge lsp_definition nodes into SliceGraph"
    );
    assert!(
        full_graph
            .edges
            .iter()
            .any(|e| e.provenance == "lsp_definition"),
        "Phase 2 must merge lsp_definition edges into SliceGraph"
    );
    let root_node = full_graph
        .nodes
        .iter()
        .find(|n| delta_env.root_ids.contains(&n.id))
        .unwrap();
    assert!(
        root_node.absence.maybe_null,
        "Resolved Phase 2 absence must propagate into root FlowNode"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_path_sanitization_and_text_scrubbing() {
    let ws = PathBuf::from("/home/alice/repo");
    let sanitizer = PathSanitizer::new(Some(&ws), false);

    let inside = sanitizer.sanitize_uri("file:///home/alice/repo/src/auth/session.ts");
    assert_eq!(inside.display, "src/auth/session.ts");
    assert!(!inside.is_external);

    let ext = sanitizer.sanitize_uri("file:///home/alice/.cargo/registry/src/tokio/lib.rs");
    assert!(ext.is_external);
    assert!(ext.display.starts_with("<external>/"));
    assert!(!ext.display.contains("alice"));
    assert!(!ext.display.contains("/home/"));

    let win_ext = sanitizer.sanitize_uri("file:///C:/Users/bob/proj/dep.go");
    assert!(win_ext.is_external);
    assert!(win_ext.display.starts_with("<external>/"));
    assert!(!win_ext.display.contains("bob"));
    assert!(!win_ext.display.contains("C:"));

    let scrubbed = sanitizer.scrub_text(
        "Failed reading file:///home/alice/repo/src/auth.ts and /Users/carol/secret/key.ts",
    );
    assert!(!scrubbed.contains("file://"));
    assert!(!scrubbed.contains("alice"));
    assert!(!scrubbed.contains("carol"));
    assert!(scrubbed.contains("src/auth.ts"));

    let abs_san = PathSanitizer::new(Some(&ws), true);
    assert_eq!(
        abs_san
            .sanitize_uri("file:///home/alice/repo/src/auth.ts")
            .display,
        "/home/alice/repo/src/auth.ts"
    );
}

#[test]
fn test_escaping_stress_and_45kb_mermaid_ceiling() {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let adversarial_labels = [
        "Map<string, Foo | null>",
        "crate::auth::load(\"secret\")",
        "end subgraph class %% comment; [a]{b}(c)",
        "line1\nline2\\slash",
    ];
    for (i, lbl) in adversarial_labels.iter().enumerate() {
        nodes.push(FlowNode {
            id: format!("caller:{i}:0"),
            kind: NodeKind::SourceNode,
            file_id: 0,
            range: range_line(i as u32),
            label: (*lbl).to_string(),
            snippet: Some(format!(
                "const x = \"{lbl}\"; // file:///home/alice/repo/src/a.ts"
            )),
            absence: if i == 0 { abs_absent() } else { abs_nullable() },
            provenance: "local_ssi".into(),
            is_guard: i == 2,
            guard_text: (i == 2).then(|| "x !== null".into()),
            is_danger_sink: i == 3,
            danger_kind: (i == 3).then(|| "MemberAccess".into()),
        });
        if i > 0 {
            edges.push(FlowEdge {
                from: format!("caller:{i}:0"),
                to: format!("caller:{}:0", i - 1),
                label: Some("arg | \"pipe\"; %%".into()),
                is_back_edge: false,
                cycle_group_id: None,
                provenance: "def_use".into(),
                edge_side: Some(EdgeSide::Upstream),
            });
        }
    }

    let env = SliceEnvelope {
        schema_version: SCHEMA_VERSION,
        query_id: 99,
        trace_id: "tr-stress".into(),
        root_ids: vec!["caller:3:0".into()],
        direction: SliceDirection::To,
        files: vec![FileEntry {
            file_id: 0,
            uri: "file:///home/alice/repo/src/stress.ts".into(),
        }],
        graph: Some(SliceGraph { nodes, edges }),
        projections: SliceProjections::default(),
        truncation: Vec::new(),
        stats: SliceStats::default(),
    };

    let opts = ExportOptions::for_format(
        ExportFormat::Mermaid,
        Some(PathBuf::from("/home/alice/repo")),
        false,
        Some(true),
        false,
        false,
        500,
        Phase2Status::NotNeeded,
        ExportSourceMode::FreshSlice,
        0,
        None,
    );
    let out = render_export(&env, &opts, "tr-stress");
    assert!(
        !out.result.content.contains("caller:0:0"),
        "Raw colon IDs must be replaced with N0..Nk"
    );
    assert!(out.result.content.contains("N0(\"<b>"));
    assert!(out.result.content.contains("N3([\"<b>◎ "));
    assert!(!out.result.content.contains("file://"));
    assert!(!out.result.content.contains("alice"));
    assert!(out.result.content.contains("&lt;string"));
    assert!(
        !out.result.content.contains("&#58;"),
        "Must not emit numeric colon entity"
    );
    assert!(
        !out.result.content.contains("&#40;"),
        "Must not emit numeric paren entity"
    );
    assert!(
        !out.result.content.contains("&#123;"),
        "Must not emit numeric brace entity"
    );
    assert!(out.result.content.contains("linkStyle "));
    assert!(!out.result.content.contains(",color:#"));
    assert!(out.result.content.contains("fill:#e38b2926"));
    // Unused classDefs (like nonnull, frontier, unverified) must be omitted
    assert!(!out.result.content.contains("classDef nonnull "));
    assert!(!out.result.content.contains("classDef frontier "));

    // 45 KB Mermaid ceiling test with 190 long-labeled nodes + snippets
    let mut big_nodes = Vec::new();
    let mut big_edges = Vec::new();
    for i in 0..190u32 {
        big_nodes.push(FlowNode {
            id: format!("n{i}"),
            kind: NodeKind::SourceNode,
            file_id: 0,
            range: range_line(i),
            label: format!("node_{i}_{}", "X".repeat(220)),
            snippet: Some(format!("const v{i} = \"{}\";", "Y".repeat(110))),
            absence: abs_nullable(),
            provenance: "local_ssi".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        });
        if i > 0 {
            big_edges.push(FlowEdge {
                from: format!("n{i}"),
                to: format!("n{}", i - 1),
                label: Some("flow".into()),
                is_back_edge: false,
                cycle_group_id: None,
                provenance: "def_use".into(),
                edge_side: Some(EdgeSide::Upstream),
            });
        }
    }
    let big_env = SliceEnvelope {
        schema_version: SCHEMA_VERSION,
        query_id: 100,
        trace_id: "tr-big".into(),
        root_ids: vec!["n189".into()],
        direction: SliceDirection::To,
        files: vec![FileEntry {
            file_id: 0,
            uri: "file:///home/alice/repo/src/big.ts".into(),
        }],
        graph: Some(SliceGraph {
            nodes: big_nodes,
            edges: big_edges,
        }),
        projections: SliceProjections::default(),
        truncation: Vec::new(),
        stats: SliceStats::default(),
    };
    let big_out = render_export(&big_env, &opts, "tr-big");
    assert!(
        big_out.result.content.len() <= MERMAID_MAX_BYTES,
        "Mermaid output {} bytes exceeded 45 KB ceiling",
        big_out.result.content.len()
    );
    assert!(
        !big_out.result.warnings.is_empty(),
        "Expected 45 KB size-guard warning"
    );
}

#[test]
fn test_null_only_causal_path_preservation_and_perf() {
    let nodes = vec![
        FlowNode {
            id: "origin_null".into(),
            kind: NodeKind::SourceNode,
            file_id: 0,
            range: range_line(0),
            label: "null origin".into(),
            snippet: None,
            absence: abs_absent(),
            provenance: "local_ssi".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        },
        FlowNode {
            id: "mid_nullable".into(),
            kind: NodeKind::SourceNode,
            file_id: 0,
            range: range_line(1),
            label: "mid".into(),
            snippet: None,
            absence: abs_nullable(),
            provenance: "local_ssi".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        },
        FlowNode {
            id: "guard_ok".into(),
            kind: NodeKind::SourceNode,
            file_id: 0,
            range: range_line(2),
            label: "guard".into(),
            snippet: None,
            absence: abs_present(),
            provenance: "local_ssi".into(),
            is_guard: true,
            guard_text: Some("x !== null".into()),
            is_danger_sink: false,
            danger_kind: None,
        },
        FlowNode {
            id: "const_ok".into(),
            kind: NodeKind::SourceNode,
            file_id: 0,
            range: range_line(10),
            label: "const ok".into(),
            snippet: None,
            absence: abs_present(),
            provenance: "local_ssi".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        },
        FlowNode {
            id: "other_ok".into(),
            kind: NodeKind::SourceNode,
            file_id: 0,
            range: range_line(11),
            label: "other ok".into(),
            snippet: None,
            absence: abs_present(),
            provenance: "local_ssi".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        },
    ];
    let edges = vec![
        FlowEdge {
            from: "mid_nullable".into(),
            to: "origin_null".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
        FlowEdge {
            from: "guard_ok".into(),
            to: "mid_nullable".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
        FlowEdge {
            from: "other_ok".into(),
            to: "const_ok".into(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".into(),
            edge_side: Some(EdgeSide::Upstream),
        },
    ];
    let env = SliceEnvelope {
        schema_version: SCHEMA_VERSION,
        query_id: 1,
        trace_id: "tr-null".into(),
        root_ids: vec!["guard_ok".into()],
        direction: SliceDirection::To,
        files: vec![FileEntry {
            file_id: 0,
            uri: "file:///ws/a.ts".into(),
        }],
        graph: Some(SliceGraph { nodes, edges }),
        projections: SliceProjections::default(),
        truncation: Vec::new(),
        stats: SliceStats::default(),
    };
    let opts = ExportOptions::for_format(
        ExportFormat::Mermaid,
        Some(PathBuf::from("/ws")),
        true,
        None,
        false,
        false,
        500,
        Phase2Status::NotNeeded,
        ExportSourceMode::FreshSlice,
        0,
        None,
    );
    let graph = ExportGraph::build(&env, &opts);
    let labels: Vec<&str> = graph.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"null origin"));
    assert!(labels.contains(&"mid"));
    assert!(labels.contains(&"guard"));
    assert!(!labels.contains(&"const ok"));
    assert!(!labels.contains(&"other ok"));
    assert_eq!(graph.edges.len(), 2);

    // 80-node export must render in < 50ms
    let mut perf_nodes = Vec::new();
    let mut perf_edges = Vec::new();
    for i in 0..80u32 {
        perf_nodes.push(FlowNode {
            id: format!("p{i}"),
            kind: NodeKind::SourceNode,
            file_id: 0,
            range: range_line(i),
            label: format!("step_{i}"),
            snippet: None,
            absence: if i == 0 { abs_absent() } else { abs_nullable() },
            provenance: "local_ssi".into(),
            is_guard: false,
            guard_text: None,
            is_danger_sink: false,
            danger_kind: None,
        });
        if i > 0 {
            perf_edges.push(FlowEdge {
                from: format!("p{i}"),
                to: format!("p{}", i - 1),
                label: None,
                is_back_edge: false,
                cycle_group_id: None,
                provenance: "def_use".into(),
                edge_side: Some(EdgeSide::Upstream),
            });
        }
    }
    let perf_env = SliceEnvelope {
        schema_version: SCHEMA_VERSION,
        query_id: 2,
        trace_id: "tr-perf".into(),
        root_ids: vec!["p79".into()],
        direction: SliceDirection::To,
        files: vec![FileEntry {
            file_id: 0,
            uri: "file:///ws/perf.ts".into(),
        }],
        graph: Some(SliceGraph {
            nodes: perf_nodes,
            edges: perf_edges,
        }),
        projections: SliceProjections::default(),
        truncation: Vec::new(),
        stats: SliceStats::default(),
    };
    let t0 = Instant::now();
    let _ = render_export(&perf_env, &opts, "tr-perf");
    assert!(
        t0.elapsed().as_millis() < 50,
        "80-node export took {}ms (>= 50ms)",
        t0.elapsed().as_millis()
    );
}

#[test]
fn test_multilang_golden_exports_and_optional_graphviz() {
    let dir = temp_dir("multilang");
    let cases = [
        (
            "flow.ts",
            "export function f(flag: boolean) {\n  let x: string | null = 'a';\n  if (flag) { x = null; }\n  return x;\n}\n",
            3u32,
            9u32,
        ),
        (
            "flow.py",
            "def f(flag):\n    x = 'a'\n    if flag:\n        x = None\n    return x\n",
            4u32,
            11u32,
        ),
        (
            "flow.rs",
            "pub fn f(flag: bool) -> Option<&'static str> {\n    let mut x = Some(\"a\");\n    if flag { x = None; }\n    x\n}\n",
            3u32,
            4u32,
        ),
        (
            "flow.go",
            "package main\nfunc f(flag bool) *string {\n\ts := \"a\"\n\tvar x *string = &s\n\tif flag {\n\t\tx = nil\n\t}\n\treturn x\n}\n",
            7u32,
            8u32,
        ),
    ];

    let engine = make_engine();
    let cancel = AtomicBool::new(false);
    let dot_available = Command::new("dot").arg("-V").output().is_ok();

    for (name, code, line, col) in cases {
        let fpath = dir.join(name);
        std::fs::write(&fpath, code).unwrap();
        let params = SliceParams {
            uri: path_to_uri(&fpath),
            position: LspPosition {
                line,
                character: col,
            },
            offset_encoding: OffsetEncoding::Utf8,
            direction: SliceDirection::To,
            include_graph: false,
            include_focus: false,
            include_tree: false,
            include_snippets: false,
            exclude_tests: false,
            query_id: Some(1),
            lsp_attached: false,
            lsp_client_name: None,
            workspace_root: Some(dir.to_string_lossy().to_string()),
            lsp_timeout_ms: None,
        };
        let (mut env, graph, _) = engine
            .slice_phase1_with_snapshot(&params, "tr-lang", &cancel)
            .unwrap();
        env.graph = Some(graph);

        for fmt in [ExportFormat::Mermaid, ExportFormat::Dot] {
            let opts = ExportOptions::for_format(
                fmt,
                Some(dir.clone()),
                false,
                None,
                false,
                false,
                500,
                Phase2Status::NotNeeded,
                ExportSourceMode::FreshSlice,
                0,
                None,
            );
            let a = render_export(&env, &opts, "tr-1").result.content;
            let b = render_export(&env, &opts, "tr-2").result.content;
            assert_eq!(
                a, b,
                "Export must be deterministic across trace_ids for {name}"
            );
            assert!(!a.contains("file://"), "No raw URIs in {name} export");

            if fmt == ExportFormat::Mermaid {
                assert!(a.contains("flowchart TD"));
                assert!(!a.contains("subgraph "));
            } else {
                assert!(a.contains("digraph lineage"));
                if dot_available {
                    let mut child = Command::new("dot")
                        .arg("-Tsvg")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    child
                        .stdin
                        .as_mut()
                        .unwrap()
                        .write_all(a.as_bytes())
                        .unwrap();
                    let status = child.wait_with_output().unwrap();
                    assert!(
                        status.status.success(),
                        "dot -Tsvg failed on {name}: {}",
                        String::from_utf8_lossy(&status.stderr)
                    );
                }
            }
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_lru_snapshot_hit_expand_merge_eviction_and_cli_rpc_parity() {
    let dir = temp_dir("lru_parity");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let fpath = src_dir.join("mod.ts");
    let code = concat!(
        "function helper(v: string | null) {\n",
        "  return v;\n",
        "}\n",
        "export function entry(flag: boolean) {\n",
        "  const x = flag ? null : 'ok';\n",
        "  const y = helper(x);\n",
        "  return y;\n",
        "}\n",
    );
    std::fs::write(&fpath, code).unwrap();

    let engine = make_engine();
    let (out_tx, out_rx) = unbounded::<String>();
    let state = TransportState::new(engine, None, out_tx);
    let uri = path_to_uri(&fpath);
    let ws_str = dir.to_string_lossy().to_string();

    // 1. Slice query_id=10 (include_graph=false on wire, snapshot saved in LRU)
    handle_request(
        &state,
        RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(RpcId::Num(1)),
            method: "lineage/slice".into(),
            params: serde_json::json!({
                "uri": uri,
                "position": { "line": 6, "character": 9 },
                "direction": "to",
                "include_graph": false,
                "include_tree": true,
                "query_id": 10,
                "workspace_root": ws_str,
            }),
        },
    );
    let slice_resp: RpcResponse = serde_json::from_str(&out_rx.recv().unwrap()).unwrap();
    assert!(slice_resp.result.as_ref().unwrap().get("graph").is_none());

    // 2. Expand with parent_query_id=10, parent_node_id="n0" -> collision-free exp1_ merge
    handle_request(
        &state,
        RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(RpcId::Num(2)),
            method: "lineage/expand".into(),
            params: serde_json::json!({
                "expansion_token": "exp:n0:0:5",
                "uri": uri,
                "position": { "line": 1, "character": 9 },
                "direction": "to",
                "workspace_root": ws_str,
                "parent_query_id": 10,
                "parent_node_id": "n0",
            }),
        },
    );
    let _exp_resp: RpcResponse = serde_json::from_str(&out_rx.recv().unwrap()).unwrap();

    // 3. Export from live LRU snapshot query_id=10
    handle_request(
        &state,
        RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(RpcId::Num(3)),
            method: "lineage/export".into(),
            params: serde_json::json!({
                "format": "mermaid",
                "query_id": 10,
                "workspace_root": ws_str,
            }),
        },
    );
    let exp_live: ExportResult = serde_json::from_value(
        serde_json::from_str::<RpcResponse>(&out_rx.recv().unwrap())
            .unwrap()
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(exp_live.source_mode, ExportSourceMode::LiveSnapshot);
    assert!(exp_live.node_count >= 2);

    // 4. Push 4 more queries (11..14) to evict query_id=10 from 4-slot LRU
    for qid in 11..=14u64 {
        handle_request(
            &state,
            RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(RpcId::Num(qid as i64)),
                method: "lineage/slice".into(),
                params: serde_json::json!({
                    "uri": uri,
                    "position": { "line": 6, "character": 9 },
                    "direction": "to",
                    "include_graph": false,
                    "query_id": qid,
                    "workspace_root": ws_str,
                }),
            },
        );
        let _ = out_rx.recv().unwrap();
    }

    // Exporting evicted query_id=10 with fallback uri+position falls back to FreshSlice
    for (idx, fmt) in ["mermaid", "dot"].iter().enumerate() {
        let qid_param = if idx == 0 {
            serde_json::json!(10)
        } else {
            serde_json::Value::Null
        };
        handle_request(
            &state,
            RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(RpcId::Str(format!("rpc-{fmt}"))),
                method: "lineage/export".into(),
                params: serde_json::json!({
                    "format": fmt,
                    "query_id": qid_param,
                    "uri": uri,
                    "position": { "line": 6, "character": 9 },
                    "direction": "to",
                    "no_lsp": true,
                    "workspace_root": ws_str,
                }),
            },
        );
        let rpc_res: ExportResult = serde_json::from_value(
            serde_json::from_str::<RpcResponse>(&out_rx.recv().unwrap())
                .unwrap()
                .result
                .unwrap(),
        )
        .unwrap();
        assert_eq!(rpc_res.source_mode, ExportSourceMode::FreshSlice);

        // Compare byte-for-byte with standalone CLI `lineage-engine export` (and `slice --format dot`)
        let bin = env!("CARGO_BIN_EXE_lineage-engine");
        let target_arg = format!("{}:7:9", fpath.display());
        let cli_out = Command::new(bin)
            .args([
                "export",
                &target_arg,
                "--format",
                fmt,
                "--direction",
                "to",
                "--no-lsp",
            ])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            cli_out.status.success(),
            "CLI export failed: {}",
            String::from_utf8_lossy(&cli_out.stderr)
        );
        let cli_str = String::from_utf8(cli_out.stdout).unwrap();
        assert_eq!(
            rpc_res.content, cli_str,
            "CLI and RPC {fmt} output must be byte-identical"
        );

        if *fmt == "dot" {
            let export_no_flag = Command::new(bin)
                .args([
                    "export",
                    &target_arg,
                    "--format",
                    "dot",
                    "--direction",
                    "to",
                ])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(export_no_flag.status.success());
            let slice_dot = Command::new(bin)
                .args(["slice", &target_arg, "--direction", "to", "--format", "dot"])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(slice_dot.status.success());
            assert_eq!(
                String::from_utf8(export_no_flag.stdout).unwrap(),
                String::from_utf8(slice_dot.stdout).unwrap(),
                "export --format dot and slice --format dot must be byte-identical"
            );
        }
    }

    // Invalid pairing `export --format json` must be rejected by CLI parser
    let bad_cli = Command::new(env!("CARGO_BIN_EXE_lineage-engine"))
        .args(["export", "src/mod.ts:7:9", "--format", "json"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(!bad_cli.status.success());

    // Legacy Debug labels & raw AST kinds humanize cleanly + semantic shapes render properly
    let legacy_env = SliceEnvelope {
        schema_version: SCHEMA_VERSION,
        query_id: 77,
        trace_id: "tr-legacy".into(),
        root_ids: vec!["root".into()],
        direction: SliceDirection::To,
        files: vec![
            FileEntry {
                file_id: 0,
                uri: "file:///ws/src/auth.ts".into(),
            },
            FileEntry {
                file_id: 1,
                uri: "file:///ws/src/helper.ts".into(),
            },
        ],
        graph: Some(SliceGraph {
            nodes: vec![
                FlowNode {
                    id: "fn_callee".into(),
                    kind: NodeKind::SourceNode,
                    file_id: 1,
                    range: range_line(0),
                    label: "fn loadToken".into(),
                    snippet: None,
                    absence: abs_nullable(),
                    provenance: "cross_file_call".into(),
                    is_guard: false,
                    guard_text: None,
                    is_danger_sink: false,
                    danger_kind: None,
                },
                FlowNode {
                    id: "call_site".into(),
                    kind: NodeKind::SourceNode,
                    file_id: 0,
                    range: range_line(3),
                    label:
                        "call Imported { module_specifier: \"./helper\", symbol: \"loadToken\" }"
                            .into(),
                    snippet: None,
                    absence: abs_nullable(),
                    provenance: "local_ssi".into(),
                    is_guard: false,
                    guard_text: None,
                    is_danger_sink: false,
                    danger_kind: None,
                },
                FlowNode {
                    id: "phi_merge".into(),
                    kind: NodeKind::SourceNode,
                    file_id: 0,
                    range: range_line(5),
                    label: "phi tok".into(),
                    snippet: None,
                    absence: abs_nullable(),
                    provenance: "local_ssi".into(),
                    is_guard: false,
                    guard_text: None,
                    is_danger_sink: false,
                    danger_kind: None,
                },
                FlowNode {
                    id: "root".into(),
                    kind: NodeKind::SourceNode,
                    file_id: 0,
                    range: range_line(8),
                    label: "binary_operator".into(),
                    snippet: None,
                    absence: abs_nullable(),
                    provenance: "local_ssi".into(),
                    is_guard: false,
                    guard_text: None,
                    is_danger_sink: true,
                    danger_kind: Some("MemberAccess".into()),
                },
            ],
            edges: vec![
                FlowEdge {
                    from: "call_site".into(),
                    to: "fn_callee".into(),
                    label: Some("calls".into()),
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "cross_file_call".into(),
                    edge_side: Some(EdgeSide::Upstream),
                },
                FlowEdge {
                    from: "phi_merge".into(),
                    to: "call_site".into(),
                    label: Some("phi".into()),
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "phi".into(),
                    edge_side: Some(EdgeSide::Upstream),
                },
                FlowEdge {
                    from: "root".into(),
                    to: "phi_merge".into(),
                    label: None,
                    is_back_edge: false,
                    cycle_group_id: None,
                    provenance: "def_use".into(),
                    edge_side: Some(EdgeSide::Upstream),
                },
            ],
        }),
        projections: SliceProjections::default(),
        truncation: Vec::new(),
        stats: SliceStats::default(),
    };
    let legacy_opts = ExportOptions::for_format(
        ExportFormat::Mermaid,
        Some(PathBuf::from("/ws")),
        false,
        None,
        false,
        false,
        500,
        Phase2Status::NotNeeded,
        ExportSourceMode::FreshSlice,
        0,
        None,
    );
    let legacy_mmd = render_export(&legacy_env, &legacy_opts, "tr-leg")
        .result
        .content;
    assert!(legacy_mmd.contains("<b>loadToken()</b>"));
    assert!(legacy_mmd.contains("{{\"<b>merge tok</b>"));
    assert!(legacy_mmd.contains("[[\"<b>fn loadToken</b>"));
    assert!(legacy_mmd.contains("([\"<b>◎ expr</b>"));
    assert!(legacy_mmd.contains("-->|\"returns\"|"));
    assert!(!legacy_mmd.contains("-->|\"phi\"|"));
    assert!(legacy_mmd.contains("==>|\"unguarded\"|"));
    assert!(legacy_mmd.contains("stroke:#14b8a6,stroke-width:2px"));
    assert!(legacy_mmd.contains("stroke:#ff3b6b,stroke-width:2.5px"));
    assert!(legacy_mmd.contains("⚠ Deref Danger"));
    assert!(!legacy_mmd.contains("Imported {"));
    assert!(!legacy_mmd.contains("binary_operator"));

    let _ = std::fs::remove_dir_all(&dir);
}
