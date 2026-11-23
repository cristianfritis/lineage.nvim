use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use lineage_engine::adapters::rust::RustAdapter;
use lineage_engine::adapters::{ImportResolution, LanguageAdapter};
use lineage_engine::cache::{DirtyBufferOverlay, FileIrCache};
use lineage_engine::protocol::schema::{
    LspPosition, OffsetEncoding, SliceDirection, SliceEnvelope, SliceParams,
};
use lineage_engine::resolver::NullLspProvider;
use lineage_engine::slicer::SlicerEngine;

fn make_engine() -> SlicerEngine {
    let cache = Arc::new(FileIrCache::new(128 * 1024 * 1024));
    let dirty = Arc::new(DirtyBufferOverlay::new());
    let resolver = Arc::new(NullLspProvider);
    let adapters: Vec<Arc<dyn LanguageAdapter>> = vec![Arc::new(RustAdapter::new())];
    SlicerEngine::new(cache, dirty, resolver, adapters)
}

fn slice_at_cursor(marked_src: &str) -> SliceEnvelope {
    let offset = marked_src.find("<|>").expect("missing <|> cursor marker");
    let clean = marked_src.replacen("<|>", "", 1);
    let prefix = &clean[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = prefix.rfind('\n').map(|i| offset - i - 1).unwrap_or(offset) as u32;

    let uri = "file:///golden/test.rs".to_string();
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
            "golden-rs",
            &AtomicBool::new(false),
            None::<fn(_)>,
        )
        .unwrap()
}

fn golden_rust_cases() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("01_const_int", "fn f() -> i32 { let <|>x = 42; x }", "NonNull"),
        ("02_const_str", "fn f() -> &'static str { let <|>s = \"hi\"; s }", "NonNull"),
        ("03_some_present", "fn f() -> Option<i32> { let <|>x = Some(1); x }", "NonNull"),
        ("04_none_absent", "fn f() -> Option<i32> { let <|>x = None; x }", "Absent"),
        ("05_ok_present", "fn f() -> Result<i32, ()> { let <|>x = Ok(1); x }", "NonNull"),
        ("06_err_absent", "fn f() -> Result<i32, i32> { let <|>x = Err(1); x }", "Absent"),
        ("07_option_param", "fn f(x: Option<i32>) -> Option<i32> { <|>x }", "Nullable"),
        ("08_result_param", "fn f(x: Result<i32, ()>) -> Result<i32, ()> { <|>x }", "Nullable"),
        ("09_if_let_some", "fn f(opt: Option<i32>) -> i32 { if let Some(u) = opt { <|>u } else { 0 } }", "NonNull"),
        ("10_if_let_ok", "fn f(res: Result<i32, ()>) -> i32 { if let Ok(u) = res { <|>u } else { 0 } }", "NonNull"),
        ("11_let_else_some", "fn f(opt: Option<i32>) -> i32 { let Some(u) = opt else { return 0; }; <|>u }", "NonNull"),
        ("12_let_else_ok", "fn f(res: Result<i32, ()>) -> i32 { let Ok(u) = res else { return 0; }; <|>u }", "NonNull"),
        ("13_try_question_mark", "fn f(opt: Option<i32>) -> Option<i32> { let <|>u = opt?; Some(u) }", "NonNull"),
        ("14_is_some_narrows", "fn f(x: Option<i32>) -> Option<i32> { if x.is_some() { return <|>x; } None }", "NonNull"),
        ("15_is_none_early_return", "fn f(x: Option<i32>) -> Option<i32> { if x.is_none() { return None; } <|>x }", "NonNull"),
        ("16_is_ok_narrows", "fn f(x: Result<i32, ()>) -> Result<i32, ()> { if x.is_ok() { return <|>x; } Err(()) }", "NonNull"),
        ("17_is_err_early_return", "fn f(x: Result<i32, ()>) -> Result<i32, ()> { if x.is_err() { return Err(()); } <|>x }", "NonNull"),
        ("18_match_some_unwrap", "fn f(opt: Option<i32>) -> i32 { match opt { Some(u) => <|>u, None => 0 } }", "NonNull"),
        ("19_match_ok_unwrap", "fn f(res: Result<i32, ()>) -> i32 { match res { Ok(u) => <|>u, Err(_) => 0 } }", "NonNull"),
        ("20_phi_if_none_and_some", "fn f(c: bool) -> Option<i32> { let <|>x = if c { Some(1) } else { None }; x }", "Nullable"),
        ("21_stdlib_hashmap_get", "fn f(m: HashMap<String, i32>) { let <|>v = m.get(\"k\"); }", "Nullable"),
        ("22_stdlib_slice_get", "fn f(s: &[i32]) { let <|>v = s.get(0); }", "Nullable"),
        ("23_stdlib_vec_pop", "fn f(v: &mut Vec<i32>) { let <|>x = v.pop(); }", "Nullable"),
        ("24_stdlib_iter_next", "fn f(it: &mut dyn Iterator<Item=i32>) { let <|>x = it.next(); }", "Nullable"),
        ("25_stdlib_map_combinator", "fn f(opt: Option<i32>) { let <|>x = opt.map(|v| v + 1); }", "Nullable"),
        ("26_stdlib_and_then", "fn f(opt: Option<i32>) { let <|>x = opt.and_then(|v| Some(v)); }", "Nullable"),
        ("27_stdlib_unwrap_or", "fn f(opt: Option<i32>) -> i32 { let <|>x = opt.unwrap_or(0); x }", "NonNull"),
        ("28_stdlib_unwrap_or_else", "fn f(opt: Option<i32>) -> i32 { let <|>x = opt.unwrap_or_else(|| 0); x }", "NonNull"),
        ("29_stdlib_ok_or", "fn f(opt: Option<i32>) { let <|>x = opt.ok_or(\"err\"); }", "Nullable"),
        ("30_stdlib_fs_read", "fn f() { let <|>s = std::fs::read_to_string(\"a.txt\"); }", "Nullable"),
        ("31_unwrap_survivor_present", "fn f(opt: Option<i32>) -> i32 { let <|>x = opt.unwrap(); x }", "NonNull"),
        ("32_expect_survivor_present", "fn f(opt: Option<i32>) -> i32 { let <|>x = opt.expect(\"missing\"); x }", "NonNull"),
        ("33_unsafe_block_unknown", "fn f() { let <|>x = unsafe { 1 }; }", "Unverified"),
        ("34_raw_ptr_deref_unknown", "fn f(p: *const i32) { let <|>x = *p; }", "Unverified"),
        ("35_readonly_self_preserves", "impl S { fn f(&self) { self.user = 1; self.inspect(); let <|>u = self.user; } }", "NonNull"),
        ("36_mut_self_downgrades", "impl S { fn f(&mut self) { self.user = 1; self.reset(); let <|>u = self.user; } }", "Unverified"),
        ("37_async_await_unwrap", "async fn f() { let <|>x = 42.await; }", "NonNull"),
        ("38_impl_method_none", "impl S { fn g() -> Option<i32> { let <|>x = None; x } }", "Absent"),
        ("39_broken_syntax_sibling", "fn f() { let broken = ; let <|>ok = 1; }", "NonNull"),
        ("40_module_top_level", "const <|>X: i32 = 10;", "NonNull"),
        ("41_str_strip_prefix", "fn f(s: &str) { let <|>x = s.strip_prefix(\"a\"); }", "Nullable"),
        ("42_path_parent", "fn f(p: &Path) { let <|>x = p.parent(); }", "Nullable"),
    ]
}

#[test]
fn test_42_golden_rust_cases() {
    for (name, src, expected_prefix) in golden_rust_cases() {
        let env = slice_at_cursor(src);
        let card = env.projections.hover_card.expect("hover_card missing");
        assert!(
            card.verdict_line.contains(expected_prefix),
            "Case {name} failed: expected '{expected_prefix}' in verdict '{}'",
            card.verdict_line
        );
    }
}

#[test]
fn test_rust_unwrap_and_expect_emit_danger_sinks() {
    let env = slice_at_cursor("fn f(opt: Option<i32>) -> i32 { let x = <|>opt.unwrap(); x }");
    let card = env.projections.hover_card.unwrap();
    assert!(!card.local_risks.is_empty(), "Expected DangerSink on opt.unwrap()");
}

#[test]
fn test_rust_cargo_workspace_and_mod_resolution() {
    let tmp = std::env::temp_dir().join("lineage_test_cargo_ws");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("crates/core_lib/src")).unwrap();
    std::fs::create_dir_all(tmp.join("crates/app/src")).unwrap();
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("crates/core_lib/Cargo.toml"),
        "[package]\nname = \"core-lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("crates/core_lib/src/auth.rs"),
        "pub fn verify() -> Option<i32> { None }\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("crates/app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(tmp.join("crates/app/src/utils.rs"), "pub fn helper() {}\n").unwrap();
    std::fs::write(
        tmp.join("crates/app/src/main.rs"),
        "mod utils;\nuse core_lib::auth::verify;\n",
    )
    .unwrap();

    let adapter = RustAdapter::new();
    let res_mod = adapter.resolve_static_import(
        &tmp.join("crates/app/src/main.rs"),
        "mod:utils",
        Some(&tmp),
        Instant::now(),
    );
    assert!(matches!(res_mod, ImportResolution::Resolved(p) if p.ends_with("utils.rs")));

    let res_ws = adapter.resolve_static_import(
        &tmp.join("crates/app/src/main.rs"),
        "core_lib::auth::verify",
        Some(&tmp),
        Instant::now(),
    );
    assert!(matches!(res_ws, ImportResolution::Resolved(p) if p.ends_with("auth.rs")));

    let res_crate = adapter.resolve_static_import(
        &tmp.join("crates/app/src/main.rs"),
        "crate::utils::helper",
        Some(&tmp),
        Instant::now(),
    );
    assert!(matches!(res_crate, ImportResolution::Resolved(p) if p.ends_with("utils.rs")));

    let _ = std::fs::remove_dir_all(&tmp);
}
