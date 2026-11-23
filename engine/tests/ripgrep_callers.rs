use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::adapters::LanguageAdapter;
use lineage_engine::cache::{DirtyBufferOverlay, FileIrCache};
use lineage_engine::protocol::schema::{
    LspPosition, OffsetEncoding, SliceDirection, SliceEnvelope, SliceParams, TruncationReason,
};
use lineage_engine::resolver::{
    discover_callers, discover_callers_with_limits, NullLspProvider,
};
use lineage_engine::slicer::SlicerEngine;

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("lineage_{prefix}_{nanos}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_standalone_ts_caller_discovery_and_quoted_specifier_prefilter() {
    let dir = temp_dir("ts_callers");
    let session_ts = dir.join("session.ts");
    let caller_ts = dir.join("handler.ts");
    let unrelated_ts = dir.join("unrelated.ts");
    let comment_ts = dir.join("comments.ts");

    fs::write(
        &session_ts,
        "export function findSession(id: string) { return null; }\n",
    )
    .unwrap();
    fs::write(
        &caller_ts,
        "import { findSession } from './session';\nexport function handle() { return findSession('1'); }\n",
    )
    .unwrap();
    // Mentions findSession without importing './session' -> filtered out by quoted-specifier prefilter
    fs::write(
        &unrelated_ts,
        "import { findSession } from './other';\nexport function other() { return findSession('2'); }\n",
    )
    .unwrap();
    // Imports './session' but only mentions findSession in a comment -> filtered out by Tree-sitter AST check
    fs::write(
        &comment_ts,
        "import './session';\n// findSession('3')\nexport const x = 1;\n",
    )
    .unwrap();

    let res = discover_callers(&dir, "findSession", &session_ts, false);
    assert_eq!(res.callers.len(), 1, "expected only handler.ts caller");
    assert_eq!(res.callers[0].caller_name, "handle");
    assert!(res.callers[0].uri.ends_with("handler.ts"));
    assert_eq!(res.truncated, None);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_standalone_python_and_rust_callers() {
    let py_dir = temp_dir("py_callers");
    let py_def = py_dir.join("auth.py");
    let py_use = py_dir.join("app.py");
    fs::write(&py_def, "def get_user():\n    return None\n").unwrap();
    fs::write(&py_use, "def main():\n    return get_user()\n").unwrap();

    let py_res = discover_callers(&py_dir, "get_user", &py_def, false);
    assert_eq!(py_res.callers.len(), 1);
    assert_eq!(py_res.callers[0].caller_name, "main");
    let _ = fs::remove_dir_all(&py_dir);

    let rs_dir = temp_dir("rs_callers");
    let rs_def = rs_dir.join("lib.rs");
    let rs_use = rs_dir.join("main.rs");
    fs::write(&rs_def, "pub fn load_cfg() -> Option<i32> { None }\n").unwrap();
    fs::write(&rs_use, "fn run() { let _ = load_cfg(); }\n").unwrap();

    let rs_res = discover_callers(&rs_dir, "load_cfg", &rs_def, false);
    assert_eq!(rs_res.callers.len(), 1);
    assert_eq!(rs_res.callers[0].caller_name, "run");
    let _ = fs::remove_dir_all(&rs_dir);
}

#[test]
fn test_ripgrep_cap_truncation_emits_partial() {
    let dir = temp_dir("rg_cap");
    let target = dir.join("mod.ts");
    fs::write(&target, "export function ping() { return 1; }\n").unwrap();
    for i in 0..8 {
        let p = dir.join(format!("c{i}.ts"));
        fs::write(
            &p,
            format!("import {{ ping }} from './mod';\nexport function f{i}() {{ return ping(); }}\n"),
        )
        .unwrap();
    }

    let res = discover_callers_with_limits(
        &dir,
        "ping",
        &target,
        false,
        3,
        300,
        Duration::from_secs(2),
    );
    assert_eq!(res.truncated, Some(TruncationReason::MaxNodes));
    assert!(res.walked_files <= 3);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_from_slice_cold_scan_cache_isolation_and_projections() {
    let dir = temp_dir("rg_cache_iso");
    let target = dir.join("session.ts");
    let caller = dir.join("caller.ts");
    fs::write(
        &target,
        "export function findSession(id: string) {\n  if (!id) { return null; }\n  return id;\n}\n",
    )
    .unwrap();
    fs::write(
        &caller,
        "import { findSession } from './session';\nexport function entry() { return findSession('a'); }\n",
    )
    .unwrap();

    let cache = Arc::new(FileIrCache::new(1024 * 1024));
    let dirty = Arc::new(DirtyBufferOverlay::new());
    let adapters: Vec<Arc<dyn LanguageAdapter>> = vec![Arc::new(TypeScriptAdapter::new())];
    let engine = SlicerEngine::new(
        cache.clone(),
        dirty,
        Arc::new(NullLspProvider),
        adapters,
    );

    let target_uri = format!("file://{}", target.to_string_lossy());
    let caller_uri = format!("file://{}", caller.to_string_lossy());

    let cancel = AtomicBool::new(false);
    let env = engine
        .slice_query::<fn(SliceEnvelope)>(
            &SliceParams {
                uri: target_uri.clone(),
                position: LspPosition {
                    line: 0,
                    character: 16,
                },
                offset_encoding: OffsetEncoding::Utf8,
                direction: SliceDirection::From,
                include_graph: false,
                include_focus: true,
                include_tree: true,
                include_snippets: true,
                exclude_tests: false,
                query_id: Some(1),
                lsp_attached: false,
                lsp_client_name: None,
                workspace_root: Some(dir.to_string_lossy().to_string()),
            },
            "tr-1",
            &cancel,
            None,
        )
        .unwrap();

    let rows = env.projections.tree_rows.expect("tree_rows expected");
    assert!(
        rows.iter().any(|r| r.uri == caller_uri),
        "expected caller.ts in tree_rows"
    );
    let spans = env.projections.focus_spans.expect("focus_spans expected");
    assert!(!spans.is_empty(), "expected focus_spans");

    // Cold-scan cache isolation: caller.ts must NOT be inserted into FileIrCache
    assert!(
        cache.get_skeleton(&caller_uri, None).is_none(),
        "cold-scan caller file must not pollute FileIrCache"
    );

    let _ = fs::remove_dir_all(&dir);
}
