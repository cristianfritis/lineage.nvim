use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use lineage_engine::adapters::python::PythonAdapter;
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
    let adapters: Vec<Arc<dyn LanguageAdapter>> = vec![Arc::new(PythonAdapter::new())];
    SlicerEngine::new(cache, dirty, resolver, adapters)
}

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
            "golden-py",
            &AtomicBool::new(false),
            None::<fn(_)>,
        )
        .unwrap()
}

fn golden_python_cases() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("01_const_int", "def f():\n    <|>x = 42\n    return x\n", "NonNull"),
        ("02_const_str", "def f():\n    <|>s = 'ok'\n    return s\n", "NonNull"),
        ("03_const_none", "def f():\n    <|>n = None\n    return n\n", "Absent"),
        ("04_const_list", "def f():\n    <|>xs = [1, 2]\n    return xs\n", "NonNull"),
        ("05_const_dict", "def f():\n    <|>d = {'a': 1}\n    return d\n", "NonNull"),
        ("06_default_none_param", "def f(x=None):\n    return <|>x\n", "Nullable"),
        ("07_typed_optional_param", "def f(x: Optional[str]):\n    return <|>x\n", "Nullable"),
        ("08_pep604_union_param", "def f(x: str | None):\n    return <|>x\n", "Nullable"),
        ("09_is_not_none_narrows", "def f(x: str | None):\n    if x is not None:\n        return <|>x\n    return 'a'\n", "NonNull"),
        ("10_is_none_else_narrows", "def f(x: str | None):\n    if x is None:\n        return 'a'\n    else:\n        return <|>x\n", "NonNull"),
        ("11_is_none_then_absent", "def f(x: str | None):\n    if x is None:\n        return <|>x\n    return 'a'\n", "Absent"),
        ("12_early_return_is_none", "def f(x: str | None):\n    if x is None:\n        return 'early'\n    return <|>x\n", "NonNull"),
        ("13_early_raise_excludes_none", "def f(x: str | None):\n    if x is None:\n        raise ValueError('bad')\n    return <|>x\n", "NonNull"),
        ("14_assert_is_not_none", "def f(x: str | None):\n    assert x is not None\n    return <|>x\n", "NonNull"),
        ("15_assert_truthy", "def f(x: str | None):\n    assert x\n    return <|>x\n", "NonNull"),
        ("16_truthy_then_present", "def f(x: str | None):\n    if x:\n        return <|>x\n    return 'a'\n", "NonNull"),
        ("17_truthy_else_maybe_absent", "def f(x: str | None):\n    if x:\n        return 'a'\n    else:\n        return <|>x\n", "Nullable"),
        ("18_not_x_else_present", "def f(x: str | None):\n    if not x:\n        return 'a'\n    else:\n        return <|>x\n", "NonNull"),
        ("19_walrus_guard", "def f(get_u):\n    if (u := get_u()) is not None:\n        return <|>u\n    return 'a'\n", "NonNull"),
        ("20_walrus_assign", "def f():\n    if (u := None) is None:\n        return <|>u\n    return 'a'\n", "Absent"),
        ("21_phi_merge_none", "def f(c):\n    x = 'ok'\n    if c:\n        x = None\n    return <|>x\n", "Nullable"),
        ("22_stdlib_dict_get", "def f(d):\n    <|>v = d.get('k')\n    return v\n", "Nullable"),
        ("23_stdlib_os_getenv", "import os\ndef f():\n    <|>v = os.getenv('PATH')\n    return v\n", "Nullable"),
        ("24_stdlib_re_search", "import re\ndef f(s):\n    <|>m = re.search('a', s)\n    return m\n", "Nullable"),
        ("25_stdlib_re_match", "import re\ndef f(s):\n    <|>m = re.match('a', s)\n    return m\n", "Nullable"),
        ("26_stdlib_next", "def f(it):\n    <|>v = next(it)\n    return v\n", "Nullable"),
        ("27_stdlib_len", "def f(xs):\n    <|>n = len(xs)\n    return n\n", "NonNull"),
        ("28_stdlib_json_loads", "import json\ndef f(s):\n    <|>v = json.loads(s)\n    return v\n", "NonNull"),
        ("29_stdlib_str_strip", "def f(s):\n    <|>v = s.strip()\n    return v\n", "NonNull"),
        ("30_stdlib_str_split", "def f(s):\n    <|>v = s.split(',')\n    return v\n", "NonNull"),
        ("31_async_def_await", "async def f():\n    <|>v = await get_val()\n    return v\n", "NonNull"),
        ("32_class_method_none", "class Svc:\n    def get(self):\n        <|>v = None\n        return v\n", "Absent"),
        ("33_class_method_present", "class Svc:\n    def get(self):\n        <|>v = 123\n        return v\n", "NonNull"),
        ("34_shallow_field_write", "def f(obj):\n    obj.name = 'alice'\n    return <|>obj.name\n", "NonNull"),
        ("35_shallow_field_none", "def f(obj):\n    obj.name = None\n    return <|>obj.name\n", "Absent"),
        ("36_comprehension_present", "def f(users):\n    <|>out = [u for u in users if u is not None]\n    return out\n", "NonNull"),
        ("37_module_top_level_present", "<|>TOP = 42\n", "NonNull"),
        ("38_module_top_level_none", "<|>TOP = None\n", "Absent"),
        ("39_broken_syntax_sibling", "def f():\n    broken = (\n    <|>ok = 1\n    return ok\n", "NonNull"),
        ("40_danger_sink_attr", "def f(x: str | None):\n    <|>y = x.strip\n    return y\n", "Nullable"),
        ("41_danger_sink_subscript", "def f(xs: list | None):\n    <|>y = xs[0]\n    return y\n", "Nullable"),
        ("42_eq_none_check", "def f(x: str | None):\n    if x != None:\n        return <|>x\n    return 'a'\n", "NonNull"),
    ]
}

#[test]
fn test_42_golden_python_cases() {
    for (name, src, expected_prefix) in golden_python_cases() {
        let env = slice_at_cursor("py", src);
        let card = env.projections.hover_card.expect("hover_card missing");
        assert!(
            card.verdict_line.contains(expected_prefix),
            "Case {name} failed: expected '{expected_prefix}' in verdict '{}'",
            card.verdict_line
        );
    }
}

#[test]
fn test_python_danger_sinks_reported() {
    let env = slice_at_cursor("py", "def f(x: str | None):\n    <|>y = x.strip\n    return y\n");
    let card = env.projections.hover_card.unwrap();
    assert!(!card.local_risks.is_empty());
}

#[test]
fn test_pyi_outer_wrapper_extraction() {
    let adapter = PythonAdapter::new();
    let pyi = r#"
def find_user(uid: int) -> Optional[User]: ...
def get_item(k: str) -> Item | None: ...
def generic_id(x: T) -> T: ...
def count_items() -> int: ...
"#;
    let u = adapter.extract_declaration_signature("file:///a.pyi", pyi, "find_user").unwrap();
    assert!(u.is_maybe_absent());
    let g = adapter.extract_declaration_signature("file:///a.pyi", pyi, "get_item").unwrap();
    assert!(g.is_maybe_absent());
    let gen = adapter.extract_declaration_signature("file:///a.pyi", pyi, "generic_id").unwrap();
    assert!(gen.has_unknown() && !gen.is_maybe_absent());
    let cnt = adapter.extract_declaration_signature("file:///a.pyi", pyi, "count_items").unwrap();
    assert!(!cnt.is_maybe_absent() && !cnt.has_unknown());
}

#[test]
fn test_python_pyproject_and_relative_import_resolution() {
    let tmp = std::env::temp_dir().join("lineage_test_py_project");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/myapp")).unwrap();
    std::fs::write(tmp.join("pyproject.toml"), "[project]\nname = \"myapp\"\n").unwrap();
    std::fs::write(tmp.join("src/myapp/__init__.py"), "").unwrap();
    std::fs::write(tmp.join("src/myapp/helpers.py"), "def get_user(): return None\n").unwrap();
    std::fs::write(tmp.join("src/myapp/main.py"), "from .helpers import get_user\n").unwrap();

    let adapter = PythonAdapter::new();
    let res_rel = adapter.resolve_static_import(
        &tmp.join("src/myapp/main.py"),
        ".helpers",
        Some(&tmp),
        Instant::now(),
    );
    assert!(matches!(res_rel, ImportResolution::Resolved(p) if p.ends_with("helpers.py")));

    let res_abs = adapter.resolve_static_import(
        &tmp.join("src/myapp/main.py"),
        "myapp.helpers",
        Some(&tmp),
        Instant::now(),
    );
    assert!(matches!(res_abs, ImportResolution::Resolved(p) if p.ends_with("helpers.py")));

    let res_escape = adapter.resolve_static_import(
        &tmp.join("src/myapp/main.py"),
        ".....etc.passwd",
        Some(&tmp),
        Instant::now(),
    );
    assert!(matches!(res_escape, ImportResolution::Unresolved));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_python_classify_test_file() {
    let adapter = PythonAdapter::new();
    assert!(adapter.classify_test_file("file:///repo/tests/test_auth.py"));
    assert!(adapter.classify_test_file("file:///repo/auth_test.py"));
    assert!(!adapter.classify_test_file("file:///repo/src/auth.py"));
    assert!(adapter.supports_extension("py"));
    assert!(adapter.supports_extension("pyi"));
    let _ = Path::new("");
}
