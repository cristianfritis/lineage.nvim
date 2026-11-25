use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use lineage_engine::adapters::go::GoAdapter;
use lineage_engine::adapters::LanguageAdapter;
use lineage_engine::cache::{path_to_uri, DirtyBufferOverlay, FileIrCache};
use lineage_engine::protocol::schema::{
    LspPosition, OffsetEncoding, SliceDirection, SliceEnvelope, SliceParams,
};
use lineage_engine::resolver::NullLspProvider;
use lineage_engine::slicer::SlicerEngine;

fn make_engine() -> SlicerEngine {
    let cache = Arc::new(FileIrCache::new(32 * 1024 * 1024));
    let dirty = Arc::new(DirtyBufferOverlay::new());
    let resolver = Arc::new(NullLspProvider);
    let adapters: Vec<Arc<dyn LanguageAdapter>> = vec![Arc::new(GoAdapter::new())];
    SlicerEngine::new(cache, dirty, resolver, adapters)
}

fn slice_at_cursor(src_with_cursor: &str) -> SliceEnvelope {
    let offset = src_with_cursor
        .find("<|>")
        .expect("missing <|> cursor marker");
    let clean = src_with_cursor.replacen("<|>", "", 1);
    let prefix = &clean[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = prefix.rfind('\n').map(|i| offset - i - 1).unwrap_or(offset) as u32;

    let uri = "file:///golden/main.go".to_string();
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
                include_graph: true,
                include_focus: false,
                include_tree: false,
                include_snippets: true,
                exclude_tests: false,
                query_id: Some(1),
                lsp_attached: false,
                lsp_client_name: None,
                workspace_root: None,
                lsp_timeout_ms: None,
            },
            "golden-go",
            &AtomicBool::new(false),
            None,
        )
        .unwrap()
}

fn golden_go_cases() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("01_const_int", "package main\nfunc f() int { <|>x := 42; return x }", "NonNull"),
        ("02_const_str", "package main\nfunc f() string { <|>s := \"ok\"; return s }", "NonNull"),
        ("03_nil_literal", "package main\nfunc f() *int { <|>p := nil; return p }", "Absent"),
        ("04_zero_var_ptr", "package main\nfunc f() *int { var <|>p *int; return p }", "Absent"),
        ("05_zero_var_slice", "package main\nfunc f() []int { var <|>s []int; return s }", "Absent"),
        ("06_zero_var_map", "package main\nfunc f() map[string]int { var <|>m map[string]int; return m }", "Absent"),
        ("07_zero_var_func", "package main\nfunc f() func() { var <|>fn func(); return fn }", "Absent"),
        ("08_zero_var_chan", "package main\nfunc f() chan int { var <|>ch chan int; return ch }", "Absent"),
        ("09_zero_var_any", "package main\nfunc f() any { var <|>a any; return a }", "Absent"),
        ("10_zero_var_error", "package main\nfunc f() error { var <|>e error; return e }", "Absent"),
        ("11_zero_var_int", "package main\nfunc f() int { var <|>n int; return n }", "NonNull"),
        ("12_zero_var_bool", "package main\nfunc f() bool { var <|>b bool; return b }", "NonNull"),
        ("13_zero_var_string", "package main\nfunc f() string { var <|>s string; return s }", "NonNull"),
        ("14_if_neq_nil_guard", "package main\nfunc f(p *int) int { if p != nil { return *<|>p }; return 0 }", "NonNull"),
        ("15_if_eq_nil_early_ret", "package main\nfunc f(p *int) *int { if p == nil { return nil }; return <|>p }", "NonNull"),
        ("16_if_eq_nil_branch", "package main\nfunc f(p *int) *int { if p == nil { return <|>p }; return p }", "Absent"),
        ("17_val_err_implies_narrow", "package main\nfunc fetch() (*int, error) { x := 1; return &x, nil }\nfunc f() *int { val, err := fetch(); if err != nil { return nil }; return <|>val }", "NonNull"),
        ("18_val_err_unguarded", "package main\nfunc fetch(c bool) (*int, error) { if c { return nil, nil }; x := 1; return &x, nil }\nfunc f(c bool) *int { <|>val, _ := fetch(c); return val }", "Nullable"),
        ("19_comma_ok_map_guard", "package main\nfunc f(m map[string]string) string { v, ok := m[\"k\"]; if ok { return <|>v }; return \"\" }", "NonNull"),
        ("20_comma_ok_type_assert", "package main\nfunc f(x any) string { s, ok := x.(string); if ok { return <|>s }; return \"\" }", "NonNull"),
        ("21_comma_ok_unguarded", "package main\nfunc f(m map[string]string) string { <|>v, _ := m[\"k\"]; return v }", "Nullable"),
        ("22_phi_branch_nil", "package main\nfunc f(c bool) *int { x := 1; p := &x; if c { p = nil }; return <|>p }", "Nullable"),
        ("23_switch_branch_nil", "package main\nfunc f(k int) *int { var p *int; switch k { case 1: x := 1; p = &x; default: p = nil }; return <|>p }", "Nullable"),
        ("24_defer_named_return_unverified", "package main\nfunc f() (res *int) { x := 1; res = &x; defer func() { res = nil }(); return <|>res }", "Unverified"),
        ("25_go_stmt_unverified", "package main\nfunc f() int { go work(); <|>x := <-ch; return x }", "Unverified"),
        ("26_chan_recv_unverified", "package main\nfunc f(ch chan int) int { <|>x := <-ch; return x }", "Unverified"),
        ("27_stdlib_os_getenv", "package main\nimport \"os\"\nfunc f() string { <|>v := os.Getenv(\"HOME\"); return v }", "NonNull"),
        ("28_stdlib_os_open", "package main\nimport \"os\"\nfunc f() *os.File { <|>fp, _ := os.Open(\"a.txt\"); return fp }", "Nullable"),
        ("29_stdlib_os_readfile", "package main\nimport \"os\"\nfunc f() []byte { <|>b, _ := os.ReadFile(\"a.txt\"); return b }", "Nullable"),
        ("30_stdlib_errors_new", "package main\nimport \"errors\"\nfunc f() error { <|>e := errors.New(\"boom\"); return e }", "NonNull"),
        ("31_stdlib_fmt_sprintf", "package main\nimport \"fmt\"\nfunc f() string { <|>s := fmt.Sprintf(\"%d\", 1); return s }", "NonNull"),
        ("32_stdlib_len", "package main\nfunc f(xs []int) int { <|>n := len(xs); return n }", "NonNull"),
        ("33_stdlib_make", "package main\nfunc f() []int { <|>s := make([]int, 0); return s }", "NonNull"),
        ("34_stdlib_regexp_compile", "package main\nimport \"regexp\"\nfunc f() *regexp.Regexp { <|>r, _ := regexp.Compile(\"a\"); return r }", "Nullable"),
        ("35_stdlib_strconv_atoi", "package main\nimport \"strconv\"\nfunc f() int { <|>n, _ := strconv.Atoi(\"42\"); return n }", "Nullable"),
        ("36_method_receiver_const", "package main\ntype Svc struct{}\nfunc (s *Svc) Get() int { <|>v := 99; return v }", "NonNull"),
        ("37_method_receiver_nil", "package main\ntype Svc struct{}\nfunc (s *Svc) Get() *int { var <|>p *int; return p }", "Absent"),
        ("38_local_call_absent", "package main\nfunc helper() *int { return nil }\nfunc f() *int { <|>x := helper(); return x }", "Absent"),
        ("39_param_flow_1hop", "package main\nfunc id(p *int) *int { return p }\nfunc f(c bool) *int { var a *int; if c { x := 1; a = &x }; <|>out := id(a); return out }", "Nullable"),
        ("40_unresolved_call_unverified", "package main\nfunc f() int { <|>x := unknownHelper(); return x }", "Unverified"),
        ("41_broken_syntax_sibling", "package main\nfunc f() int {\n\tif {\n\t}\n\t<|>ok := 1\n\treturn ok\n}", "NonNull"),
        ("42_composite_literal_nonnull", "package main\ntype User struct{ Name string }\nfunc f() User { <|>u := User{Name: \"a\"}; return u }", "NonNull"),
    ]
}

#[test]
fn test_42_golden_go_cases() {
    for (name, src, expected_prefix) in golden_go_cases() {
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
fn test_go_mod_and_same_package_sibling_resolution() {
    let tmp = std::env::temp_dir().join(format!("lineage_go_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let pkg_dir = tmp.join("internal/auth");
    let cmd_dir = tmp.join("cmd/server");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::create_dir_all(&cmd_dir).unwrap();

    fs::write(tmp.join("go.mod"), "module example.com/app\n\ngo 1.22\n").unwrap();
    fs::write(
        pkg_dir.join("store.go"),
        "package auth\nfunc FindUser() *string {\n\treturn nil\n}\n",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("handler.go"),
        "package auth\nfunc Handle() *string {\n\tu := FindUser()\n\treturn u\n}\n",
    )
    .unwrap();
    fs::write(
        cmd_dir.join("main.go"),
        "package main\nimport \"example.com/app/internal/auth\"\nfunc Run() *string {\n\tu := auth.FindUser()\n\treturn u\n}\n",
    )
    .unwrap();

    let engine = make_engine();

    // 1. Same-package sibling call (handler.go -> store.go)
    let handler_uri = path_to_uri(&pkg_dir.join("handler.go"));
    let env_sib = engine
        .slice_query::<fn(SliceEnvelope)>(
            &SliceParams {
                uri: handler_uri,
                position: LspPosition {
                    line: 2,
                    character: 1,
                },
                offset_encoding: OffsetEncoding::Utf8,
                direction: SliceDirection::To,
                include_graph: false,
                include_focus: false,
                include_tree: false,
                include_snippets: false,
                exclude_tests: true,
                query_id: Some(1),
                lsp_attached: false,
                lsp_client_name: None,
                workspace_root: Some(tmp.to_string_lossy().to_string()),
                lsp_timeout_ms: None,
            },
            "go-sib",
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
    assert_eq!(
        env_sib.projections.hover_card.unwrap().status_badge,
        "Absent"
    );

    // 2. go.mod module import (cmd/server/main.go -> internal/auth/store.go)
    let main_uri = path_to_uri(&cmd_dir.join("main.go"));
    let env_mod = engine
        .slice_query::<fn(SliceEnvelope)>(
            &SliceParams {
                uri: main_uri,
                position: LspPosition {
                    line: 3,
                    character: 1,
                },
                offset_encoding: OffsetEncoding::Utf8,
                direction: SliceDirection::To,
                include_graph: false,
                include_focus: false,
                include_tree: false,
                include_snippets: false,
                exclude_tests: true,
                query_id: Some(2),
                lsp_attached: false,
                lsp_client_name: None,
                workspace_root: Some(tmp.to_string_lossy().to_string()),
                lsp_timeout_ms: None,
            },
            "go-mod",
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
    assert_eq!(
        env_mod.projections.hover_card.unwrap().status_badge,
        "Absent"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_go_danger_sinks_and_test_classification() {
    let adapter = GoAdapter::new();
    assert!(adapter.classify_test_file("file:///repo/auth_test.go"));
    assert!(adapter.classify_test_file("file:///repo/testdata/fixture.go"));
    assert!(!adapter.classify_test_file("file:///repo/auth.go"));

    let env =
        slice_at_cursor("package main\nfunc f() int {\n\tvar p *int\n\t<|>x := *p\n\treturn x\n}");
    let card = env.projections.hover_card.unwrap();
    assert!(
        !card.local_risks.is_empty(),
        "expected pointer dereference on nil pointer to emit local_risks"
    );
}
