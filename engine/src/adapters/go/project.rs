use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use moka::sync::Cache;

use crate::adapters::typescript::tsconfig::safe_join;
use crate::adapters::ImportResolution;

pub const MAX_GO_PKG_SIBLINGS: usize = 32;
pub const GO_PKG_SCAN_BUDGET: Duration = Duration::from_millis(20);

#[derive(Debug, Clone)]
pub struct GoModMeta {
    pub root_dir: PathBuf,
    pub module_path: String,
}

pub struct GoModCache {
    by_dir: Cache<PathBuf, Option<GoModMeta>>,
}

impl Default for GoModCache {
    fn default() -> Self {
        Self::new()
    }
}

impl GoModCache {
    pub fn new() -> Self {
        Self {
            by_dir: Cache::new(64),
        }
    }

    pub fn find_for_file(&self, file: &Path, workspace_root: Option<&Path>) -> Option<GoModMeta> {
        let start_dir = file.parent()?;
        let mut cur = Some(start_dir);
        while let Some(dir) = cur {
            if let Some(cached) = self.by_dir.get(dir) {
                if cached.is_some() {
                    return cached;
                }
            }
            let go_mod = dir.join("go.mod");
            if go_mod.is_file() {
                let meta = parse_go_mod(&go_mod, dir);
                self.by_dir.insert(dir.to_path_buf(), meta.clone());
                if meta.is_some() {
                    return meta;
                }
            }
            if let Some(root) = workspace_root {
                if dir == root {
                    break;
                }
            }
            cur = dir.parent();
        }
        None
    }
}

fn parse_go_mod(path: &Path, root_dir: &Path) -> Option<GoModMeta> {
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let mod_path = rest.split_whitespace().next()?.trim_matches('"');
            if !mod_path.is_empty() {
                return Some(GoModMeta {
                    root_dir: root_dir.to_path_buf(),
                    module_path: mod_path.to_string(),
                });
            }
        }
    }
    None
}

pub fn resolve_go_import(
    cache: &GoModCache,
    from_file: &Path,
    module_specifier: &str,
    workspace_root: Option<&Path>,
) -> ImportResolution {
    let spec = module_specifier.trim_matches('"');
    if spec.is_empty() {
        return ImportResolution::Unresolved;
    }

    let Some(meta) = cache.find_for_file(from_file, workspace_root) else {
        return ImportResolution::Unresolved;
    };

    let rel = if spec == meta.module_path {
        ""
    } else if let Some(rest) = spec.strip_prefix(&format!("{}/", meta.module_path)) {
        rest
    } else {
        return ImportResolution::Unresolved;
    };

    let pkg_dir = if rel.is_empty() {
        meta.root_dir.clone()
    } else if let Some(joined) = safe_join(&meta.root_dir, rel, workspace_root) {
        joined
    } else {
        return ImportResolution::Unresolved;
    };

    if !pkg_dir.is_dir() {
        return ImportResolution::Unresolved;
    }

    if let Some(first_go) = first_non_test_go_file(&pkg_dir) {
        return ImportResolution::Resolved(first_go);
    }

    ImportResolution::Unresolved
}

fn first_non_test_go_file(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().and_then(|e| e.to_str()) == Some("go")
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_test.go"))
        })
        .collect();
    files.sort();
    files.into_iter().next()
}

pub fn scan_package_siblings(
    from_file: &Path,
    exclude_tests: bool,
    budget_start: Instant,
) -> Vec<PathBuf> {
    let Some(dir) = from_file.parent() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut siblings = Vec::new();
    for entry in entries.flatten() {
        if budget_start.elapsed() > GO_PKG_SCAN_BUDGET || siblings.len() >= MAX_GO_PKG_SIBLINGS {
            break;
        }
        let path = entry.path();
        if path == from_file || !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("go") {
            continue;
        }
        if exclude_tests
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("_test.go"))
        {
            continue;
        }
        siblings.push(path);
    }
    siblings.sort();
    siblings
}
