use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use lineage_engine::adapters::typescript::tsconfig::safe_join;
use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::adapters::{ImportResolution, LanguageAdapter};
use lineage_engine::cache::{DirtyBufferOverlay, FileIrCache};
use lineage_engine::ir::{AbsenceValue, STATE_NULL, STATE_PRESENT, STATE_UNDEFINED};
use lineage_engine::protocol::schema::{
    LspPosition, OffsetEncoding, SliceDirection, SliceEnvelope, SliceParams, UnknownReason,
};
use lineage_engine::resolver::NullLspProvider;
use lineage_engine::slicer::SlicerEngine;

fn make_engine() -> SlicerEngine {
    let cache = Arc::new(FileIrCache::new(128 * 1024 * 1024));
    let dirty = Arc::new(DirtyBufferOverlay::new());
    let resolver = Arc::new(NullLspProvider);
    let adapters: Vec<Arc<dyn LanguageAdapter>> = vec![Arc::new(TypeScriptAdapter::new())];
    SlicerEngine::new(cache, dirty, resolver, adapters)
}

/// Helper: parses `<|>` cursor marker from source, syncs dirty buffer, and slices.
fn slice_at_cursor(ext: &str, marked_src: &str) -> SliceEnvelope {
    let offset = marked_src.find("<|>").expect("missing <|> cursor marker");
    let clean = marked_src.replacen("<|>", "", 1);
    let prefix = &clean[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = prefix.rfind('\n').map(|i| offset - i - 1).unwrap_or(offset) as u32;

    let uri = format!("file:///golden/test.{ext}");
    let engine = make_engine();
    engine.dirty_buffers.upsert(uri.clone(), 1, clean).unwrap();

    engine
        .slice_query(
            &SliceParams {
                uri,
                position: LspPosition {
                    line,
                    character: col,
                },
                offset_encoding: OffsetEncoding::Utf8,
                direction: SliceDirection::To,
                include_graph: true,
                include_focus: false,
                include_tree: false,
                include_snippets: true,
                exclude_tests: false,
                query_id: Some(1),
                lsp_attached: false,
                lsp_client_name: None,
                workspace_root: None,
            },
            "golden",
            &AtomicBool::new(false),
            None::<fn(_)>,
        )
        .unwrap()
}

fn golden_cases() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        ("01_const_present", "ts", "function f() { const <|>x = 42; return x; }", "NonNull"),
        ("02_const_string", "ts", "function f() { const <|>s = 'hello'; return s; }", "NonNull"),
        ("03_const_null", "ts", "function f() { const <|>n = null; return n; }", "Absent"),
        ("04_const_undefined", "ts", "function f() { const <|>u = undefined; return u; }", "Absent"),
        ("05_uninit_var", "ts", "function f() { let <|>u; return u; }", "Absent"),
        ("06_optional_chain", "ts", "function f(a: any) { const <|>v = a?.b; return v; }", "Nullable"),
        ("07_nullish_coalesce", "ts", "function f(a: string | null) { const <|>v = a ?? 'fallback'; return v; }", "NonNull"),
        ("08_non_null_assert", "ts", "function f(a: string | null) { const <|>v = a!; return v; }", "NonNull"),
        ("09_if_neq_null_retains_undef", "ts", "function f(x: string | null | undefined) { if (x !== null) { return <|>x; } }", "Nullable"),
        ("10_if_loose_neq_null_narrows_both", "ts", "function f(x: string | null | undefined) { if (x != null) { return <|>x; } }", "NonNull"),
        ("11_if_truthy_then_present", "ts", "function f(x: string | null) { if (x) { return <|>x; } }", "NonNull"),
        ("12_if_truthy_else_maybe_absent", "ts", "function f(x: string | null) { if (x) { return 'ok'; } else { return <|>x; } }", "Nullable"),
        ("13_early_return_excludes_dead_branch", "ts", "function f(x: string | null) { if (x === null) { return 'early'; } return <|>x; }", "NonNull"),
        ("14_early_throw_excludes_dead_branch", "ts", "function f(x: string | null) { if (x === null) { throw new Error('bad'); } return <|>x; }", "NonNull"),
        ("15_phi_merge_null_and_present", "ts", "function f(c: boolean) { let x = 'ok'; if (c) { x = null; } return <|>x; }", "Nullable"),
        ("16_phi_merge_undef_and_present", "ts", "function f(c: boolean) { let x = 1; if (c) { x = undefined; } return <|>x; }", "Nullable"),
        ("17_arrow_fn_present", "ts", "const f = () => { const <|>a = 1; return a; };", "NonNull"),
        ("18_arrow_fn_null", "ts", "const f = () => { const <|>a = null; return a; };", "Absent"),
        ("19_await_unwrap", "ts", "async function f() { const <|>v = await 123; return v; }", "NonNull"),
        ("20_stdlib_array_find", "ts", "function f(arr: number[]) { const <|>item = arr.find((x) => x > 0); return item; }", "Nullable"),
        ("21_stdlib_map_get", "ts", "function f(m: Map<string, number>) { const <|>v = m.get('k'); return v; }", "Nullable"),
        ("22_stdlib_regexp_exec", "ts", "function f(r: RegExp) { const <|>m = r.exec('abc'); return m; }", "Nullable"),
        ("23_stdlib_query_selector", "ts", "function f() { const <|>el = document.querySelector('#id'); return el; }", "Nullable"),
        ("24_stdlib_array_pop", "ts", "function f(arr: number[]) { const <|>v = arr.pop(); return v; }", "Nullable"),
        ("25_stdlib_array_shift", "ts", "function f(arr: number[]) { const <|>v = arr.shift(); return v; }", "Nullable"),
        ("26_tsx_component_present", "tsx", "export function Btn() { const <|>label = 'Click'; return <button>{label}</button>; }", "NonNull"),
        ("27_tsx_component_null", "tsx", "export function Empty() { const <|>el = null; return <div>{el}</div>; }", "Absent"),
        ("28_js_function_present", "js", "function f() { const <|>x = 10; return x; }", "NonNull"),
        ("29_js_function_null", "js", "function f() { const <|>x = null; return x; }", "Absent"),
        ("30_jsx_component_present", "jsx", "export function Card() { const <|>title = 'Hi'; return <h1>{title}</h1>; }", "NonNull"),
        ("31_mjs_module", "mjs", "export function f() { const <|>v = undefined; return v; }", "Absent"),
        ("32_cjs_module", "cjs", "function f() { const <|>v = 1; return v; }", "NonNull"),
        ("33_module_top_level", "ts", "const <|>top = 42;", "NonNull"),
        ("34_module_top_level_null", "ts", "const <|>top = null;", "Absent"),
        ("35_nullish_assign", "ts", "function f(x: string | null) { x ??= 'def'; return <|>x; }", "NonNull"),
        ("36_optional_param", "ts", "function f(x?: string) { return <|>x; }", "Nullable"),
        ("37_union_param_null", "ts", "function f(x: string | null) { return <|>x; }", "Nullable"),
        ("38_union_param_undef", "ts", "function f(x: string | undefined) { return <|>x; }", "Nullable"),
        ("39_class_method", "ts", "class Foo { bar() { const <|>v = null; return v; } }", "Absent"),
        ("40_broken_syntax_sibling", "ts", "function f() { const broken = ; const <|>ok = 1; return ok; }", "NonNull"),
        ("41_danger_sink_member", "ts", "function f(x: { a: number } | null) { const <|>y = x.a; return y; }", "Nullable"),
        ("42_danger_sink_subscript", "ts", "function f(arr: number[] | null) { const <|>y = arr[0]; return y; }", "Nullable"),
    ]
}

#[test]
fn test_danger_sinks_reported_in_local_risks() {
    let env = slice_at_cursor(
        "ts",
        "function f(x: { a: number } | null) { const <|>y = x.a; return y; }",
    );
    let card = env.projections.hover_card.unwrap();
    assert!(!card.local_risks.is_empty());
}

#[test]
fn test_42_golden_typescript_cases() {
    for (name, ext, src, expected_prefix) in golden_cases() {
        let env = slice_at_cursor(ext, src);
        let card = env.projections.hover_card.expect("hover_card missing");
        assert!(
            card.status_badge.starts_with(expected_prefix),
            "case {name}: expected badge starting with '{expected_prefix}', got '{}' (verdict: {})",
            card.status_badge,
            card.verdict_line
        );
    }
}

#[test]
fn test_tsx_and_jsx_have_zero_parse_errors() {
    let adapter = TypeScriptAdapter::new();
    let tsx = "export const App = () => <div className='ok'><span>hi</span></div>;";
    let skel = adapter.parse_skeleton("file:///App.tsx", tsx);
    assert!(skel.parse_errors.is_empty());

    let jsx = "export function App() { return <button>ok</button>; }";
    let skel_js = adapter.parse_skeleton("file:///App.jsx", jsx);
    assert!(skel_js.parse_errors.is_empty());
}

#[test]
fn test_dts_outer_wrapper_extraction() {
    let adapter = TypeScriptAdapter::new();
    let dts = r#"
        export declare function findUser(id: string): User | null;
        export declare function getOpt(k: string): string | undefined;
        export declare function id<T>(x: T): T;
        export declare function count(): number;
    "#;
    let r1 = adapter
        .extract_declaration_signature("file:///a.d.ts", dts, "findUser")
        .unwrap();
    assert_eq!(r1.known_states, STATE_PRESENT | STATE_NULL);

    let r2 = adapter
        .extract_declaration_signature("file:///a.d.ts", dts, "getOpt")
        .unwrap();
    assert_eq!(r2.known_states, STATE_PRESENT | STATE_UNDEFINED);

    let r3 = adapter
        .extract_declaration_signature("file:///a.d.ts", dts, "id")
        .unwrap();
    assert!(r3.has_unknown());
    assert_eq!(r3.reasons(), vec![UnknownReason::ExternalUntyped]);

    let r4 = adapter
        .extract_declaration_signature("file:///a.d.ts", dts, "count")
        .unwrap();
    assert_eq!(r4, AbsenceValue::present());
}

#[test]
fn test_tsconfig_path_alias_and_traversal_guard() {
    let tmp = std::env::temp_dir().join(format!("lineage_tsconfig_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/users")).unwrap();
    std::fs::write(
        tmp.join("tsconfig.json"),
        r#"{
            // JSONC comment
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@app/*": ["src/*"],
                },
            }
        }"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/users/index.ts"),
        "export function getUser() { return null; }",
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/main.ts"),
        "import { getUser } from '@app/users';",
    )
    .unwrap();

    let adapter = TypeScriptAdapter::new();
    let res = adapter.resolve_static_import(
        &tmp.join("src/main.ts"),
        "@app/users",
        Some(&tmp),
        Instant::now(),
    );
    match res {
        ImportResolution::Resolved(p) => {
            assert!(p.ends_with("src/users/index.ts"));
        }
        other => panic!("Expected Resolved, got {other:?}"),
    }

    // Traversal guard: `..` escape outside workspace_root must be rejected
    assert!(safe_join(&tmp.join("src"), "../../etc/passwd", Some(&tmp)).is_none());
    assert!(safe_join(
        Path::new("/repo/src"),
        "../lib/a.ts",
        Some(Path::new("/repo"))
    )
    .is_some());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_classify_test_file() {
    let adapter = TypeScriptAdapter::new();
    assert!(adapter.classify_test_file("file:///src/auth.test.ts"));
    assert!(adapter.classify_test_file("file:///src/auth.spec.tsx"));
    assert!(adapter.classify_test_file("file:///src/__tests__/auth.ts"));
    assert!(!adapter.classify_test_file("file:///src/auth.ts"));
}
