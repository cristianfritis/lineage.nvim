use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::adapters::ImportResolution;

pub const MAX_BARREL_HOPS: usize = 2;
pub const MAX_BARREL_SIBLINGS: usize = 10;
pub const BARREL_BUDGET: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Default)]
pub struct TsConfig {
    pub base_dir: PathBuf,
    pub base_url: Option<PathBuf>,
    pub paths: HashMap<String, Vec<String>>,
}

#[derive(Default)]
pub struct TsConfigCache {
    by_dir: Mutex<HashMap<PathBuf, Option<TsConfig>>>,
}

impl TsConfigCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn find_for_file(&self, file: &Path, workspace_root: Option<&Path>) -> Option<TsConfig> {
        let start_dir = file.parent()?;
        let mut cur = Some(start_dir);
        while let Some(dir) = cur {
            {
                let cache = self.by_dir.lock().expect("tsconfig cache lock");
                if let Some(cached) = cache.get(dir) {
                    if cached.is_some() {
                        return cached.clone();
                    }
                }
            }
            let candidate = dir.join("tsconfig.json");
            if candidate.is_file() {
                let parsed = parse_tsconfig(&candidate, workspace_root);
                let mut cache = self.by_dir.lock().expect("tsconfig cache lock");
                cache.insert(dir.to_path_buf(), parsed.clone());
                return parsed;
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

fn parse_tsconfig(path: &Path, workspace_root: Option<&Path>) -> Option<TsConfig> {
    let raw = fs::read_to_string(path).ok()?;
    let val = jsonc_parser::parse_to_serde_value(&raw, &Default::default()).ok()??;
    let dir = path.parent()?.to_path_buf();

    let mut cfg = TsConfig {
        base_dir: dir.clone(),
        base_url: None,
        paths: HashMap::new(),
    };

    // Single-level relative "extends"
    if let Some(extends_str) = val.get("extends").and_then(|v| v.as_str()) {
        if extends_str.starts_with('.') {
            if let Some(parent_path) = safe_join(&dir, extends_str, workspace_root) {
                let parent_file = if parent_path.extension().is_some() {
                    parent_path
                } else {
                    parent_path.with_extension("json")
                };
                if let Ok(p_raw) = fs::read_to_string(&parent_file) {
                    if let Ok(Some(p_val)) =
                        jsonc_parser::parse_to_serde_value(&p_raw, &Default::default())
                    {
                        apply_compiler_options(
                            &mut cfg,
                            &p_val,
                            parent_file.parent().unwrap_or(&dir),
                            workspace_root,
                        );
                    }
                }
            }
        }
    }

    apply_compiler_options(&mut cfg, &val, &dir, workspace_root);
    Some(cfg)
}

fn apply_compiler_options(
    cfg: &mut TsConfig,
    val: &serde_json::Value,
    dir: &Path,
    workspace_root: Option<&Path>,
) {
    let Some(co) = val.get("compilerOptions") else {
        return;
    };
    if let Some(base_url) = co.get("baseUrl").and_then(|v| v.as_str()) {
        if let Some(joined) = safe_join(dir, base_url, workspace_root) {
            cfg.base_url = Some(joined);
        }
    }
    if let Some(paths_obj) = co.get("paths").and_then(|v| v.as_object()) {
        for (pattern, targets_val) in paths_obj {
            if let Some(arr) = targets_val.as_array() {
                let targets: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                cfg.paths.insert(pattern.clone(), targets);
            }
        }
    }
}

/// Normalizes a relative join and rejects `..` escape outside `workspace_root`.
pub fn safe_join(base: &Path, rel: &str, workspace_root: Option<&Path>) -> Option<PathBuf> {
    let joined = normalize_path(&base.join(rel));
    if let Some(root) = workspace_root {
        let norm_root = normalize_path(root);
        if !joined.starts_with(&norm_root) {
            return None;
        }
    }
    Some(joined)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                } else {
                    stack.push(comp);
                }
            }
            _ => stack.push(comp),
        }
    }
    stack.iter().collect()
}

pub fn resolve_import_with_tsconfig(
    cache: &TsConfigCache,
    from_file: &Path,
    specifier: &str,
    workspace_root: Option<&Path>,
    budget_start: Instant,
) -> ImportResolution {
    if budget_start.elapsed() > BARREL_BUDGET {
        return ImportResolution::BarrelCapReached;
    }

    // 1. Relative import
    if specifier.starts_with('.') {
        if let Some(dir) = from_file.parent() {
            if let Some(base_target) = safe_join(dir, specifier, workspace_root) {
                if let Some(found) = probe_ts_extensions(&base_target) {
                    return check_barrel_cap(
                        &found,
                        workspace_root,
                        budget_start,
                        0,
                        &mut HashSet::new(),
                    );
                }
            }
        }
        return ImportResolution::Unresolved;
    }

    // 2. tsconfig.json paths / baseUrl
    if let Some(cfg) = cache.find_for_file(from_file, workspace_root) {
        for (pattern, targets) in &cfg.paths {
            if let Some(matched) = match_path_pattern(pattern, specifier) {
                let root = cfg.base_url.as_deref().unwrap_or(&cfg.base_dir);
                for target_pat in targets {
                    let expanded = target_pat.replace('*', matched);
                    if let Some(candidate) = safe_join(root, &expanded, workspace_root) {
                        if let Some(found) = probe_ts_extensions(&candidate) {
                            return check_barrel_cap(
                                &found,
                                workspace_root,
                                budget_start,
                                0,
                                &mut HashSet::new(),
                            );
                        }
                    }
                }
            }
        }
        if let Some(base_url) = &cfg.base_url {
            if let Some(candidate) = safe_join(base_url, specifier, workspace_root) {
                if let Some(found) = probe_ts_extensions(&candidate) {
                    return check_barrel_cap(
                        &found,
                        workspace_root,
                        budget_start,
                        0,
                        &mut HashSet::new(),
                    );
                }
            }
        }
    }

    ImportResolution::Unresolved
}

fn match_path_pattern<'a>(pattern: &str, specifier: &'a str) -> Option<&'a str> {
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        if specifier.starts_with(prefix) && specifier.ends_with(suffix) {
            let end = specifier.len().saturating_sub(suffix.len());
            if end >= prefix.len() {
                return Some(&specifier[prefix.len()..end]);
            }
        }
        None
    } else if pattern == specifier {
        Some("")
    } else {
        None
    }
}

fn probe_ts_extensions(base: &Path) -> Option<PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    let exts = ["ts", "tsx", "d.ts", "js", "jsx", "mts", "cts"];
    for ext in exts {
        let mut p = base.to_path_buf();
        let file_name = base.file_name()?.to_string_lossy();
        p.set_file_name(format!("{file_name}.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    for ext in exts {
        let p = base.join(format!("index.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn check_barrel_cap(
    path: &Path,
    workspace_root: Option<&Path>,
    budget_start: Instant,
    hop: usize,
    visited: &mut HashSet<PathBuf>,
) -> ImportResolution {
    if budget_start.elapsed() > BARREL_BUDGET || hop > MAX_BARREL_HOPS {
        return ImportResolution::BarrelCapReached;
    }
    if !visited.insert(path.to_path_buf()) {
        return ImportResolution::Resolved(path.to_path_buf());
    }
    let Ok(text) = fs::read_to_string(path) else {
        return ImportResolution::Resolved(path.to_path_buf());
    };
    let wildcard_count = text
        .lines()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("export *") || t.starts_with("export*")
        })
        .count();
    if wildcard_count > MAX_BARREL_SIBLINGS {
        return ImportResolution::BarrelCapReached;
    }
    let _ = workspace_root;
    ImportResolution::Resolved(path.to_path_buf())
}
