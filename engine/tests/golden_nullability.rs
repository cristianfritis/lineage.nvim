use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use lineage_engine::adapters::python::PythonAdapter;
use lineage_engine::adapters::rust::RustAdapter;
use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::adapters::LanguageAdapter;
use lineage_engine::cache::{DirtyBufferOverlay, FileIrCache};
use lineage_engine::protocol::schema::{
    LspPosition, OffsetEncoding, SliceDirection, SliceEnvelope, SliceParams,
};
use lineage_engine::resolver::NullLspProvider;
use lineage_engine::slicer::SlicerEngine;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExpectedKind {
    NullableOrAbsent,
    NonNull,
    Unverified,
}

struct Probe {
    name: &'static str,
    ext: &'static str,
    src: &'static str,
    expected: ExpectedKind,
}

fn make_engine() -> SlicerEngine {
    let cache = Arc::new(FileIrCache::new(32 * 1024 * 1024));
    let dirty = Arc::new(DirtyBufferOverlay::new());
    let resolver = Arc::new(NullLspProvider);
    let adapters: Vec<Arc<dyn LanguageAdapter>> = vec![
        Arc::new(TypeScriptAdapter::new()),
        Arc::new(PythonAdapter::new()),
        Arc::new(RustAdapter::new()),
    ];
    SlicerEngine::new(cache, dirty, resolver, adapters)
}

fn slice_probe(ext: &str, src_with_cursor: &str) -> SliceEnvelope {
    let offset = src_with_cursor
        .find("<|>")
        .expect("probe must contain <|> cursor marker");
    let clean = src_with_cursor.replacen("<|>", "", 1);
    let prefix = &clean[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = prefix.rfind('\n').map(|i| offset - i - 1).unwrap_or(offset) as u32;

    let uri = format!("file:///golden_nullability/probe.{ext}");
    let engine = make_engine();
    engine.dirty_buffers.upsert(uri.clone(), 1, clean).unwrap();
    engine
        .slice_query::<fn(SliceEnvelope)>(
            &SliceParams {
                uri,
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
                workspace_root: None,
                lsp_timeout_ms: None,
            },
            "golden-nullability",
            &AtomicBool::new(false),
            None,
        )
        .unwrap()
}

fn all_probes() -> Vec<Probe> {
    vec![
        // --- TypeScript (11 probes) ---
        Probe {
            name: "ts_01_null_literal",
            ext: "ts",
            src: "function f() { const <|>x = null; return x; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "ts_02_undefined_literal",
            ext: "ts",
            src: "function f() { const <|>x = undefined; return x; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "ts_03_branch_null_merge",
            ext: "ts",
            src: "function f(c: boolean) { let x: string | null = 'ok'; if (c) { x = null; } return <|>x; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "ts_04_param_flow_1hop",
            ext: "ts",
            src: "function id(v: string) { return v; }\nfunction f(c: boolean) { let a: string | null = 'ok'; if (c) { a = null; } const <|>out = id(a); return out; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "ts_05_map_get_stdlib",
            ext: "ts",
            src: "function f(m: Map<string, number>) { const <|>v = m.get('k'); return v; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "ts_06_array_find_stdlib",
            ext: "ts",
            src: "function f(xs: number[]) { const <|>v = xs.find((n) => n > 0); return v; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "ts_07_nonnull_const",
            ext: "ts",
            src: "function f() { const <|>x = 42; return x; }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "ts_08_strict_not_null_guard",
            ext: "ts",
            src: "function f(x: string | null) { if (x !== null) { return <|>x; } return 'a'; }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "ts_09_not_undefined_guard",
            ext: "ts",
            src: "function f(x: string | undefined) { if (x !== undefined) { return <|>x; } return 'a'; }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "ts_10_unresolved_local_call_unverified",
            ext: "ts",
            src: "function f() { const <|>x = unknownHelper(); return x; }",
            expected: ExpectedKind::Unverified,
        },
        Probe {
            name: "ts_11_dynamic_callback_unverified",
            ext: "ts",
            src: "function f(cb: () => string) { const <|>x = cb(); return x; }",
            expected: ExpectedKind::Unverified,
        },
        // --- TSX / JavaScript (11 probes) ---
        Probe {
            name: "tsx_01_null_prop",
            ext: "tsx",
            src: "export function Card(flag: boolean) { let title: string | null = 'hi'; if (flag) { title = null; } const <|>out = title; return <div>{out}</div>; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "js_02_undefined_var",
            ext: "js",
            src: "function f() { let <|>u = undefined; return u; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "js_03_null_else_branch",
            ext: "js",
            src: "function f(c) { let x = 'ok'; if (c) { x = null; } if (x != null) { return 'ok'; } else { return <|>x; } }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "js_04_regexp_exec",
            ext: "js",
            src: "function f(re, s) { const <|>m = re.exec(s); return m; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "tsx_05_json_parse_nonnull",
            ext: "tsx",
            src: "function f(s: string) { const <|>v = JSON.parse(s); return v; }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "tsx_06_jsx_literal_nonnull",
            ext: "tsx",
            src: "export function Badge() { const <|>label = 'OK'; return <span>{label}</span>; }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "js_07_early_return_guard",
            ext: "js",
            src: "function f(x) { if (x == null) { return 'default'; } return <|>x; }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "js_08_array_pop_nullable",
            ext: "js",
            src: "function f(arr) { const <|>last = arr.pop(); return last; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "tsx_09_param_flow_1hop",
            ext: "tsx",
            src: "function wrap(v: string) { return v; }\nfunction f(c: boolean) { let a: string | null = 'x'; if (c) { a = null; } const <|>r = wrap(a); return r; }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "js_10_unresolved_call_unverified",
            ext: "js",
            src: "function f() { const <|>x = externalFetch(); return x; }",
            expected: ExpectedKind::Unverified,
        },
        Probe {
            name: "tsx_11_empty_fn_unverified",
            ext: "tsx",
            src: "export function Empty() {<|>}",
            expected: ExpectedKind::Unverified,
        },
        // --- Python (11 probes) ---
        Probe {
            name: "py_01_none_literal",
            ext: "py",
            src: "def f():\n    <|>x = None\n    return x\n",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "py_02_branch_none_merge",
            ext: "py",
            src: "def f(c):\n    x = 'ok'\n    if c:\n        x = None\n    return <|>x\n",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "py_03_dict_get",
            ext: "py",
            src: "def f(d):\n    <|>v = d.get('k')\n    return v\n",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "py_04_os_getenv",
            ext: "py",
            src: "import os\ndef f():\n    <|>v = os.getenv('HOME')\n    return v\n",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "py_05_re_search",
            ext: "py",
            src: "import re\ndef f(s):\n    <|>m = re.search('a', s)\n    return m\n",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "py_06_param_flow_1hop",
            ext: "py",
            src: "def passthrough(v):\n    return v\ndef f(c):\n    inp = 'ok'\n    if c:\n        inp = None\n    <|>out = passthrough(inp)\n    return out\n",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "py_07_const_nonnull",
            ext: "py",
            src: "def f():\n    <|>x = 123\n    return x\n",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "py_08_is_not_none_guard",
            ext: "py",
            src: "def f(x: str | None):\n    if x is not None:\n        return <|>x\n    return 'a'\n",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "py_09_early_raise_guard",
            ext: "py",
            src: "def f(x: str | None):\n    if x is None:\n        raise ValueError('missing')\n    return <|>x\n",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "py_10_unresolved_call_unverified",
            ext: "py",
            src: "def f():\n    <|>v = unknown_fn()\n    return v\n",
            expected: ExpectedKind::Unverified,
        },
        Probe {
            name: "py_11_walrus_unverified_call",
            ext: "py",
            src: "def f(cb):\n    if (u := cb()) is not None:\n        return <|>u\n    return 'a'\n",
            expected: ExpectedKind::Unverified,
        },
        // --- Rust (11 probes) ---
        Probe {
            name: "rs_01_none_literal",
            ext: "rs",
            src: "fn f() -> Option<i32> { let <|>x = None; x }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "rs_02_branch_none_merge",
            ext: "rs",
            src: "fn f(c: bool) -> Option<i32> { let mut x = Some(1); if c { x = None; } <|>x }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "rs_03_slice_first",
            ext: "rs",
            src: "fn f(xs: &[i32]) { let <|>x = xs.first(); }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "rs_04_result_ok",
            ext: "rs",
            src: "fn f(r: Result<i32, ()>) { let <|>x = r.ok(); }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "rs_05_param_flow_1hop",
            ext: "rs",
            src: "fn id(v: Option<i32>) -> Option<i32> { v }\nfn f(c: bool) -> Option<i32> { let mut a = Some(1); if c { a = None; } let <|>out = id(a); out }",
            expected: ExpectedKind::NullableOrAbsent,
        },
        Probe {
            name: "rs_06_const_nonnull",
            ext: "rs",
            src: "fn f() -> i32 { let <|>x = 42; x }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "rs_07_if_let_some_guard",
            ext: "rs",
            src: "fn f(opt: Option<i32>) -> i32 { if let Some(u) = opt { return <|>u; } 0 }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "rs_08_unwrap_or_nonnull",
            ext: "rs",
            src: "fn f(opt: Option<i32>) -> i32 { let <|>x = opt.unwrap_or(0); x }",
            expected: ExpectedKind::NonNull,
        },
        Probe {
            name: "rs_09_unsafe_unverified",
            ext: "rs",
            src: "fn f() { let <|>x = unsafe { 1 }; }",
            expected: ExpectedKind::Unverified,
        },
        Probe {
            name: "rs_10_raw_ptr_unverified",
            ext: "rs",
            src: "fn f(p: *const i32) { let <|>x = *p; }",
            expected: ExpectedKind::Unverified,
        },
        Probe {
            name: "rs_11_unresolved_call_unverified",
            ext: "rs",
            src: "fn f() { let <|>x = unknown_helper(); }",
            expected: ExpectedKind::Unverified,
        },
    ]
}

#[test]
fn test_golden_nullability_suite_g5a_soundness() {
    let probes = all_probes();
    assert!(probes.len() >= 40, "expected >= 40 probes");

    let mut false_safe_nonnull = 0usize;
    for probe in &probes {
        let env = slice_probe(probe.ext, probe.src);
        let card = env.projections.hover_card.expect("hover_card missing");

        let is_unqualified_nonnull = card.status_badge == "NonNull" && card.unverified_count == 0;
        if probe.expected != ExpectedKind::NonNull && is_unqualified_nonnull {
            false_safe_nonnull += 1;
        }

        match probe.expected {
            ExpectedKind::NullableOrAbsent => {
                assert!(
                    card.status_badge.starts_with("Nullable") || card.status_badge == "Absent",
                    "Probe {} expected Nullable/Absent, got '{}' (verdict: '{}')",
                    probe.name,
                    card.status_badge,
                    card.verdict_line
                );
            }
            ExpectedKind::NonNull => {
                assert_eq!(
                    card.status_badge, "NonNull",
                    "Probe {} expected NonNull, got '{}' (verdict: '{}')",
                    probe.name, card.status_badge, card.verdict_line
                );
            }
            ExpectedKind::Unverified => {
                assert_eq!(
                    card.status_badge, "Unverified",
                    "Probe {} expected Unverified, got '{}' (verdict: '{}')",
                    probe.name, card.status_badge, card.verdict_line
                );
            }
        }
    }

    assert_eq!(
        false_safe_nonnull, 0,
        "Gate G5a failed: {false_safe_nonnull} false-safe NonNull verdicts"
    );
}
