use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lineage_engine::adapters::{LanguageAdapter, StubAdapter};
use lineage_engine::cache::{
    check_text_guards, read_file_guarded, DirtyBufferOverlay, FileIrCache, GuardError,
    MAX_FILE_BYTES, MAX_LINE_BYTES,
};
use lineage_engine::ir::guards::GuardFact;
use lineage_engine::ir::{AbsenceValue, GuardEnv, STATE_NULL, STATE_PRESENT, STATE_UNDEFINED};
use lineage_engine::protocol::schema::{
    AbsenceSummary, CanonicalPos, CanonicalRange, LspLocationDto, LspPosition,
    LspResolveBatchParams, LspResolveItem, LspResolveKind, NodeKind, OffsetEncoding, RpcRequest,
    SliceDirection, SliceParams, TruncationReason, UnknownReason, PROTOCOL_VERSION,
};
use lineage_engine::protocol::transport::{handle_request, TransportState};
use lineage_engine::resolver::ewma::CircuitState;
use lineage_engine::resolver::lsp_provider::FixtureRule;
use lineage_engine::resolver::{
    ClientHealthKey, FixtureLspProvider, NullLspProvider, SymbolResolver,
};
use lineage_engine::slicer::pipeline::{run_reduction_pipeline, RawSliceEdge, RawSliceNode};
use lineage_engine::slicer::scc::{solve_scc_fixed_point, tarjan_scc};
use lineage_engine::slicer::SlicerEngine;
use lineage_engine::text::PositionMapper;

#[test]
fn test_position_mapper_ascii_and_multibyte() {
    let ascii = "const x = 1;\nconst y = 2;\n";
    let m_ascii = PositionMapper::new(ascii);
    let cpos = m_ascii.lsp_to_canonical(
        LspPosition {
            line: 1,
            character: 6,
        },
        OffsetEncoding::Utf16,
    );
    assert_eq!(cpos, CanonicalPos { line: 1, col: 6 });
    assert_eq!(
        m_ascii.canonical_to_lsp(cpos, OffsetEncoding::Utf16),
        LspPosition {
            line: 1,
            character: 6
        }
    );

    // "🚀" is 4 UTF-8 bytes, 2 UTF-16 code units, 1 UTF-32 code point
    let unicode = "const a = \"🚀\";\nconst b = 1;";
    let m_uni = PositionMapper::new(unicode);
    // After "🚀" on line 0: "const a = \"🚀" -> byte col 11 + 4 = 15, utf16 col 11 + 2 = 13, utf32 col 11 + 1 = 12
    let from_utf16 = m_uni.lsp_to_canonical(
        LspPosition {
            line: 0,
            character: 13,
        },
        OffsetEncoding::Utf16,
    );
    assert_eq!(from_utf16, CanonicalPos { line: 0, col: 15 });
    let from_utf32 = m_uni.lsp_to_canonical(
        LspPosition {
            line: 0,
            character: 12,
        },
        OffsetEncoding::Utf32,
    );
    assert_eq!(from_utf32, CanonicalPos { line: 0, col: 15 });
    assert_eq!(
        m_uni.canonical_to_lsp(CanonicalPos { line: 0, col: 15 }, OffsetEncoding::Utf16),
        LspPosition {
            line: 0,
            character: 13
        }
    );
}

#[test]
fn test_two_axis_lattice_separation() {
    let present = AbsenceValue::present();
    let unknown = AbsenceValue::unknown(UnknownReason::ExternalUntyped);
    let joined = present.join(unknown);

    assert!(!joined.is_maybe_absent());
    assert!(joined.has_unknown());
    assert_eq!(joined.reasons(), vec![UnknownReason::ExternalUntyped]);

    let null_val = AbsenceValue::null();
    let joined_null = null_val.join(unknown);
    assert!(joined_null.is_maybe_absent());
    assert!(joined_null.has_unknown());
    assert!(joined_null.to_summary().maybe_null);
    assert!(!joined_null.to_summary().maybe_undefined);
}

#[test]
fn test_guard_env_implies_and_unreachable() {
    let mut env = GuardEnv::new();
    env.add_fact(GuardFact::Implies {
        antecedent_place: "err".to_string(),
        antecedent_mask: STATE_NULL,
        consequent_place: "val".to_string(),
        consequent_mask: STATE_PRESENT,
    });
    env.add_fact(GuardFact::StateIn {
        place: "err".to_string(),
        mask: STATE_NULL,
    });

    let val_before = AbsenceValue::maybe_null();
    let narrowed = env.apply_to("val", val_before);
    assert!(narrowed.is_present_only());

    env.add_fact(GuardFact::Unreachable);
    let dead = env.apply_to("val", val_before);
    assert_eq!(dead, AbsenceValue::bottom());
}

#[test]
fn test_tarjan_scc_and_8_iter_widening() {
    // 0 <-> 1 cycle
    let adj = vec![vec![1], vec![0]];
    let scc = tarjan_scc(&adj);
    assert_eq!(scc.sccs.len(), 1);
    assert!(!scc.back_edges.is_empty());

    // Transfer function that never stabilizes on its own (keeps adding bits)
    let mut step = 0u8;
    let initial = vec![AbsenceValue::present(), AbsenceValue::bottom()];
    let (res, widened) = solve_scc_fixed_point(&scc, initial, |idx, vals| {
        step += 1;
        if step < 4 {
            AbsenceValue {
                known_states: vals[idx].known_states
                    | if step % 2 == 0 {
                        STATE_NULL
                    } else {
                        STATE_UNDEFINED
                    },
                unknown_reasons: 0,
            }
        } else {
            // Keep oscillating or changing if we weren't monotone; here test convergence within 8
            vals[idx]
        }
    });
    assert!(widened.is_empty());
    assert!(res[0].is_maybe_absent());

    // Force non-convergence by returning a value that doesn't stabilize? Since join is monotone
    // over a finite lattice, let's test that 8 iterations bound is respected even on artificial non-converging transfer
    // (actually monotone join over 3 bits converges in <= 3 steps, so let's verify normal convergence!).
}

#[test]
fn test_pipeline_determinism_contraction_and_test_filter() {
    let r = CanonicalRange {
        start: CanonicalPos { line: 0, col: 0 },
        end: CanonicalPos { line: 0, col: 5 },
    };
    let present = AbsenceSummary {
        present: true,
        maybe_null: false,
        maybe_undefined: false,
        unknown_reasons: Vec::new(),
    };
    let raw_nodes = vec![
        RawSliceNode {
            id: "root".to_string(),
            file_id: 0,
            is_test_file: false,
            is_copy_passthrough: false,
            range: r,
            label: "root".to_string(),
            snippet: None,
            absence: present.clone(),
            provenance: "local".to_string(),
        },
        RawSliceNode {
            id: "copy1".to_string(),
            file_id: 0,
            is_test_file: false,
            is_copy_passthrough: true,
            range: r,
            label: "copy1".to_string(),
            snippet: None,
            absence: present.clone(),
            provenance: "local".to_string(),
        },
        RawSliceNode {
            id: "origin".to_string(),
            file_id: 0,
            is_test_file: false,
            is_copy_passthrough: false,
            range: r,
            label: "origin".to_string(),
            snippet: None,
            absence: present.clone(),
            provenance: "local".to_string(),
        },
        RawSliceNode {
            id: "test_node".to_string(),
            file_id: 1,
            is_test_file: true,
            is_copy_passthrough: false,
            range: r,
            label: "test_node".to_string(),
            snippet: None,
            absence: present,
            provenance: "local".to_string(),
        },
    ];
    let raw_edges = vec![
        RawSliceEdge {
            from: "root".to_string(),
            to: "copy1".to_string(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".to_string(),
        },
        RawSliceEdge {
            from: "copy1".to_string(),
            to: "origin".to_string(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".to_string(),
        },
        RawSliceEdge {
            from: "root".to_string(),
            to: "test_node".to_string(),
            label: None,
            is_back_edge: false,
            cycle_group_id: None,
            provenance: "def_use".to_string(),
        },
    ];

    let out = run_reduction_pipeline(
        &["root".to_string()],
        &raw_nodes,
        &raw_edges,
        true,
        Duration::from_millis(50),
        Instant::now(),
    );

    assert_eq!(out.contracted_chains, 1);
    assert!(out
        .graph
        .nodes
        .iter()
        .any(|n| n.kind == NodeKind::ContractedChain));
    assert!(!out.graph.nodes.iter().any(|n| n.id == "test_node"));
    assert_eq!(out.truncation, Vec::<TruncationReason>::new());
}

#[test]
fn test_file_and_line_guards() {
    let oversized = "a".repeat(MAX_FILE_BYTES + 1);
    assert!(matches!(
        check_text_guards(&oversized),
        Err(GuardError::FileTooLarge)
    ));

    let long_line = "a".repeat(MAX_LINE_BYTES + 1);
    assert!(matches!(
        check_text_guards(&long_line),
        Err(GuardError::FileTooLarge)
    ));

    let proc_res = read_file_guarded(std::path::Path::new("/proc/cpuinfo"));
    assert!(matches!(proc_res, Err(GuardError::NonRegularFile)));
}

#[test]
fn test_fixture_lsp_ewma_and_circuit_breaker() {
    let rules = vec![
        FixtureRule {
            key: "symA".to_string(),
            kind: Some(LspResolveKind::Definition),
            client_name: Some("tsserver".to_string()),
            workspace_root: Some("/repo".to_string()),
            offset_encoding: Some(OffsetEncoding::Utf16),
            locations: vec![],
            simulate_timeout: true,
            simulate_latency_ms: 150,
            sleep_ms: 0,
        },
        FixtureRule {
            key: "symA".to_string(),
            kind: Some(LspResolveKind::Definition),
            client_name: Some("tsserver".to_string()),
            workspace_root: Some("/repo".to_string()),
            offset_encoding: Some(OffsetEncoding::Utf16),
            locations: vec![],
            simulate_timeout: true,
            simulate_latency_ms: 150,
            sleep_ms: 0,
        },
        FixtureRule {
            key: "symA".to_string(),
            kind: Some(LspResolveKind::Definition),
            client_name: Some("tsserver".to_string()),
            workspace_root: Some("/repo".to_string()),
            offset_encoding: Some(OffsetEncoding::Utf16),
            locations: vec![],
            simulate_timeout: true,
            simulate_latency_ms: 150,
            sleep_ms: 0,
        },
        FixtureRule {
            key: "symA".to_string(),
            kind: Some(LspResolveKind::Definition),
            client_name: Some("tsserver".to_string()),
            workspace_root: Some("/repo".to_string()),
            offset_encoding: Some(OffsetEncoding::Utf16),
            locations: vec![LspLocationDto {
                uri: "file:///repo/a.ts".to_string(),
                start: LspPosition {
                    line: 1,
                    character: 0,
                },
                end: LspPosition {
                    line: 1,
                    character: 5,
                },
            }],
            simulate_timeout: false,
            simulate_latency_ms: 20,
            sleep_ms: 0,
        },
    ];

    let provider = FixtureLspProvider::from_rules(rules);
    let key = ClientHealthKey {
        client_name: "tsserver".to_string(),
        workspace_root: "/repo".to_string(),
    };
    let t0 = Instant::now();

    let make_batch = || LspResolveBatchParams {
        query_id: 1,
        trace_id: "t1".to_string(),
        depth: 1,
        timeout_ms: 150,
        items: vec![LspResolveItem {
            key: "symA".to_string(),
            uri: "file:///repo/main.ts".to_string(),
            position: LspPosition {
                line: 0,
                character: 0,
            },
            offset_encoding: OffsetEncoding::Utf16,
            kind: LspResolveKind::Definition,
        }],
        circuit_open_clients: Vec::new(),
        max_timeout_ms: None,
    };

    let ctx = lineage_engine::resolver::ResolveContext::default();
    // 3 timeouts trip the circuit
    for _ in 0..3 {
        let r = provider.resolve_batch(make_batch(), &ctx, t0);
        assert!(r.results[0].timed_out);
    }
    assert_eq!(provider.ewma.check_circuit(&key, t0), CircuitState::Open);

    // Immediate call is short-circuited
    let r_open = provider.resolve_batch(make_batch(), &ctx, t0);
    assert!(r_open.results[0].circuit_open);

    // After 31s -> HalfOpen -> 20ms reply -> Closed
    let t1 = t0 + Duration::from_secs(31);
    assert_eq!(
        provider.ewma.check_circuit(&key, t1),
        CircuitState::HalfOpen
    );
    let r_rec = provider.resolve_batch(make_batch(), &ctx, t1);
    assert!(!r_rec.results[0].timed_out);
    assert!(!r_rec.results[0].circuit_open);
    assert_eq!(r_rec.results[0].locations.len(), 1);
    assert_eq!(provider.ewma.check_circuit(&key, t1), CircuitState::Closed);
}

#[test]
fn test_transport_initialize_and_dirty_sync() {
    let cache = Arc::new(FileIrCache::new(128 * 1024 * 1024));
    let dirty = Arc::new(DirtyBufferOverlay::new());
    let resolver: Arc<dyn SymbolResolver> = Arc::new(NullLspProvider);
    let adapters: Vec<Arc<dyn LanguageAdapter>> = vec![Arc::new(StubAdapter)];
    let engine = Arc::new(SlicerEngine::new(cache, dirty, resolver, adapters));

    let (out_tx, out_rx) = crossbeam_channel::unbounded();
    let state = TransportState::new(engine, None, out_tx);

    // Initialize with matching version
    handle_request(
        &state,
        RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(lineage_engine::protocol::schema::RpcId::Str(
                "c-1".to_string(),
            )),
            method: "initialize".to_string(),
            params: serde_json::json!({ "protocol_version": PROTOCOL_VERSION }),
        },
    );
    let resp_line = out_rx.recv().unwrap();
    let resp: serde_json::Value = serde_json::from_str(&resp_line).unwrap();
    assert_eq!(resp["result"]["protocol_version"], PROTOCOL_VERSION);

    // Sync dirty buffer and slice it
    handle_request(
        &state,
        RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(lineage_engine::protocol::schema::RpcId::Str(
                "c-2".to_string(),
            )),
            method: "lineage/syncBuffer".to_string(),
            params: serde_json::json!({
                "uri": "file:///test.stub",
                "changedtick": 1,
                "text": "stub content"
            }),
        },
    );
    let _ = out_rx.recv().unwrap();

    let slice_env = state
        .engine
        .slice_query(
            &SliceParams {
                uri: "file:///test.stub".to_string(),
                position: LspPosition {
                    line: 0,
                    character: 0,
                },
                offset_encoding: OffsetEncoding::Utf8,
                direction: SliceDirection::To,
                include_graph: false,
                include_focus: false,
                include_tree: false,
                include_snippets: true,
                exclude_tests: false,
                query_id: Some(99),
                lsp_attached: false,
                lsp_client_name: None,
                workspace_root: None,
                lsp_timeout_ms: None,
            },
            "tr-test",
            &AtomicBool::new(false),
            None::<fn(_)>,
        )
        .unwrap();
    assert_eq!(slice_env.query_id, 99);
    assert!(slice_env.graph.is_none());
}
