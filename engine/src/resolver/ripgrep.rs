use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ignore::{WalkBuilder, WalkState};
use memchr::memmem;
use tree_sitter::{Node, Parser};

use crate::protocol::schema::{CanonicalPos, CanonicalRange, TruncationReason};

pub const MAX_WALKED_FILES: usize = 5_000;
pub const MAX_PARSED_FILES: usize = 300;
pub const SCAN_DEADLINE_MS: u64 = 2_000;

#[derive(Debug, Clone)]
pub struct CallerSite {
    pub path: PathBuf,
    pub uri: String,
    pub range: CanonicalRange,
    pub snippet: Option<String>,
    pub caller_name: String,
    pub is_test_file: bool,
}

#[derive(Debug, Clone)]
pub struct RipgrepScanResult {
    pub callers: Vec<CallerSite>,
    pub walked_files: usize,
    pub parsed_files: usize,
    pub truncated: Option<TruncationReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LangFamily {
    TypeScriptJs,
    Python,
    Rust,
}

impl LangFamily {
    fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs" => Some(Self::TypeScriptJs),
            "py" | "pyi" => Some(Self::Python),
            "rs" => Some(Self::Rust),
            _ => None,
        }
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

pub fn contains_word_boundary(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    for pos in memmem::find_iter(haystack, needle) {
        let left_ok = pos == 0 || !is_ident_byte(haystack[pos - 1]);
        let right_idx = pos + needle.len();
        let right_ok = right_idx == haystack.len() || !is_ident_byte(haystack[right_idx]);
        if left_ok && right_ok {
            return true;
        }
    }
    false
}

fn is_test_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains(".test.")
        || s.contains(".spec.")
        || s.contains("__tests__")
        || s.contains("/tests/")
        || s.contains("\\tests\\")
        || s.ends_with("_test.py")
        || s.ends_with("_test.rs")
        || path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("test_"))
}

fn target_module_stem(target_file: &Path) -> Option<String> {
    let stem = target_file.file_stem()?.to_str()?;
    if stem == "index" || stem == "mod" || stem == "__init__" {
        target_file
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    } else {
        Some(stem.to_string())
    }
}

/// TS/JS quoted-specifier prefilter: checks whether `haystack` contains a quoted
/// module specifier referencing `stem` (e.g. `'./session'` or `"../lib/session"`).
pub fn has_quoted_module_specifier(haystack: &[u8], stem: &str) -> bool {
    if stem.is_empty() {
        return true;
    }
    let needle = stem.as_bytes();
    for pos in memmem::find_iter(haystack, needle) {
        // Scan backwards on the same line for an opening quote (' or ")
        let line_start = haystack[..pos]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let before = &haystack[line_start..pos];
        let has_open_quote = before.iter().any(|&b| b == b'\'' || b == b'"');
        if !has_open_quote {
            continue;
        }
        let line_end = haystack[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(haystack.len(), |i| pos + i);
        let after = &haystack[pos + needle.len()..line_end];
        let has_close_quote = after.iter().any(|&b| b == b'\'' || b == b'"');
        if has_close_quote {
            return true;
        }
    }
    false
}

fn ts_language_for_ext(ext: &str) -> Option<tree_sitter::Language> {
    match ext {
        "ts" | "mts" | "cts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "js" | "jsx" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "py" | "pyi" => Some(tree_sitter_python::LANGUAGE.into()),
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        _ => None,
    }
}

fn verify_call_sites_with_treesitter(
    ext: &str,
    source: &str,
    target_symbol: &str,
    path: &Path,
    is_test_file: bool,
) -> Vec<CallerSite> {
    let Some(lang) = ts_language_for_ext(ext) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let bytes = source.as_bytes();
    let uri = format!("file://{}", path.to_string_lossy());
    let mut sites = Vec::new();
    collect_call_sites(
        tree.root_node(),
        bytes,
        source,
        target_symbol,
        path,
        &uri,
        is_test_file,
        "<module>",
        &mut sites,
    );
    sites
}

#[allow(clippy::too_many_arguments)]
fn collect_call_sites(
    node: Node,
    bytes: &[u8],
    source: &str,
    target_symbol: &str,
    path: &Path,
    uri: &str,
    is_test_file: bool,
    enclosing_fn: &str,
    out: &mut Vec<CallerSite>,
) {
    let current_fn_owned: Option<String> = match node.kind() {
        "function_declaration"
        | "method_definition"
        | "function_definition"
        | "function_item" => node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(bytes).ok())
            .map(|s| s.to_string()),
        _ => None,
    };
    let active_fn = current_fn_owned.as_deref().unwrap_or(enclosing_fn);

    let is_call_node = matches!(
        node.kind(),
        "call_expression" | "new_expression" | "call" | "macro_invocation"
    );

    if is_call_node {
        let callee_node = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("constructor"))
            .or_else(|| node.child_by_field_name("macro"))
            .or_else(|| node.named_child(0));

        if let Some(callee) = callee_node {
            let matched = match callee.kind() {
                "identifier" | "property_identifier" | "field_identifier" => {
                    callee.utf8_text(bytes).ok() == Some(target_symbol)
                }
                "member_expression" | "attribute" | "field_expression" | "scoped_identifier" => {
                    let prop = callee
                        .child_by_field_name("property")
                        .or_else(|| callee.child_by_field_name("attribute"))
                        .or_else(|| callee.child_by_field_name("field"))
                        .or_else(|| callee.child_by_field_name("name"));
                    prop.and_then(|p| p.utf8_text(bytes).ok()) == Some(target_symbol)
                }
                _ => false,
            };

            if matched {
                let start = node.start_position();
                let end = node.end_position();
                let line_snippet = source
                    .lines()
                    .nth(start.row)
                    .map(|l| l.trim().to_string());
                out.push(CallerSite {
                    path: path.to_path_buf(),
                    uri: uri.to_string(),
                    range: CanonicalRange {
                        start: CanonicalPos {
                            line: start.row as u32,
                            col: start.column as u32,
                        },
                        end: CanonicalPos {
                            line: end.row as u32,
                            col: end.column as u32,
                        },
                    },
                    snippet: line_snippet,
                    caller_name: active_fn.to_string(),
                    is_test_file,
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_call_sites(
            child,
            bytes,
            source,
            target_symbol,
            path,
            uri,
            is_test_file,
            active_fn,
            out,
        );
    }
}

pub fn discover_callers(
    workspace_root: &Path,
    target_symbol: &str,
    target_file: &Path,
    exclude_tests: bool,
) -> RipgrepScanResult {
    discover_callers_with_limits(
        workspace_root,
        target_symbol,
        target_file,
        exclude_tests,
        MAX_WALKED_FILES,
        MAX_PARSED_FILES,
        Duration::from_millis(SCAN_DEADLINE_MS),
    )
}

pub fn discover_callers_with_limits(
    workspace_root: &Path,
    target_symbol: &str,
    target_file: &Path,
    exclude_tests: bool,
    max_walked: usize,
    max_parsed: usize,
    deadline: Duration,
) -> RipgrepScanResult {
    if target_symbol.is_empty() || !workspace_root.exists() {
        return RipgrepScanResult {
            callers: Vec::new(),
            walked_files: 0,
            parsed_files: 0,
            truncated: None,
        };
    }

    let start = Instant::now();
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let threads = (cpus / 2).clamp(1, 4);

    let target_ext = target_file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let target_family = LangFamily::from_ext(target_ext);
    let module_stem = target_module_stem(target_file);
    let canonical_target = target_file
        .canonicalize()
        .unwrap_or_else(|_| target_file.to_path_buf());

    let walked_files = Arc::new(AtomicUsize::new(0));
    let parsed_files = Arc::new(AtomicUsize::new(0));
    let aborted = Arc::new(AtomicBool::new(false));
    let trunc_reason: Arc<Mutex<Option<TruncationReason>>> = Arc::new(Mutex::new(None));
    let results: Arc<Mutex<Vec<CallerSite>>> = Arc::new(Mutex::new(Vec::new()));

    let walker = WalkBuilder::new(workspace_root)
        .threads(threads)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build_parallel();

    walker.run(|| {
        let walked_files = Arc::clone(&walked_files);
        let parsed_files = Arc::clone(&parsed_files);
        let aborted = Arc::clone(&aborted);
        let trunc_reason = Arc::clone(&trunc_reason);
        let results = Arc::clone(&results);
        let target_symbol = target_symbol.to_string();
        let module_stem = module_stem.clone();
        let canonical_target = canonical_target.clone();

        Box::new(move |entry_res| {
            if aborted.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            if start.elapsed() >= deadline {
                aborted.store(true, Ordering::Relaxed);
                *trunc_reason.lock().expect("trunc lock") = Some(TruncationReason::WallClockBudget);
                return WalkState::Quit;
            }

            let Ok(entry) = entry_res else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return WalkState::Continue;
            }

            let walked = walked_files.fetch_add(1, Ordering::Relaxed) + 1;
            if walked > max_walked {
                aborted.store(true, Ordering::Relaxed);
                *trunc_reason.lock().expect("trunc lock") = Some(TruncationReason::MaxNodes);
                return WalkState::Quit;
            }

            let path = entry.path();
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                return WalkState::Continue;
            };
            let Some(family) = LangFamily::from_ext(ext) else {
                return WalkState::Continue;
            };
            if let Some(tf) = target_family {
                if tf != family {
                    return WalkState::Continue;
                }
            }

            let test_file = is_test_path(path);
            if exclude_tests && test_file {
                return WalkState::Continue;
            }

            let Ok(bytes) = std::fs::read(path) else {
                return WalkState::Continue;
            };

            if !contains_word_boundary(&bytes, target_symbol.as_bytes()) {
                return WalkState::Continue;
            }

            // TS/JS quoted-specifier prefilter for cross-file candidates
            if family == LangFamily::TypeScriptJs {
                let is_same_file = path
                    .canonicalize()
                    .map(|c| c == canonical_target)
                    .unwrap_or(false);
                if !is_same_file {
                    if let Some(ref stem) = module_stem {
                        if !has_quoted_module_specifier(&bytes, stem) {
                            return WalkState::Continue;
                        }
                    }
                }
            }

            let parsed = parsed_files.fetch_add(1, Ordering::Relaxed) + 1;
            if parsed > max_parsed {
                aborted.store(true, Ordering::Relaxed);
                *trunc_reason.lock().expect("trunc lock") = Some(TruncationReason::MaxNodes);
                return WalkState::Quit;
            }

            let Ok(source) = std::str::from_utf8(&bytes) else {
                return WalkState::Continue;
            };

            let sites = verify_call_sites_with_treesitter(
                ext,
                source,
                &target_symbol,
                path,
                test_file,
            );
            if !sites.is_empty() {
                results.lock().expect("results lock").extend(sites);
            }

            WalkState::Continue
        })
    });

    let mut callers = results.lock().expect("results lock").clone();
    callers.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.range.start.cmp(&b.range.start))
    });

    let final_walked = walked_files.load(Ordering::Relaxed).min(max_walked);
    let final_parsed = parsed_files.load(Ordering::Relaxed).min(max_parsed);
    let truncated = *trunc_reason.lock().expect("trunc lock");

    RipgrepScanResult {
        callers,
        walked_files: final_walked,
        parsed_files: final_parsed,
        truncated,
    }
}
