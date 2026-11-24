use std::fs;
use std::path::{Path, PathBuf};

use moka::sync::Cache;

use crate::adapters::typescript::tsconfig::safe_join;
use crate::adapters::ImportResolution;

#[derive(Debug, Clone, Default)]
pub struct PyProjectMeta {
    pub root_dir: PathBuf,
    pub source_roots: Vec<PathBuf>,
}

pub struct PyProjectCache {
    by_dir: Cache<PathBuf, Option<PyProjectMeta>>,
}

impl Default for PyProjectCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PyProjectCache {
    pub fn new() -> Self {
        Self {
            by_dir: Cache::new(64),
        }
    }

    pub fn find_for_file(
        &self,
        file: &Path,
        workspace_root: Option<&Path>,
    ) -> Option<PyProjectMeta> {
        let start_dir = file.parent()?;
        let mut cur = Some(start_dir);
        while let Some(dir) = cur {
            if let Some(cached) = self.by_dir.get(dir) {
                return cached;
            }
            let pyproject = dir.join("pyproject.toml");
            if pyproject.is_file() {
                let meta = parse_pyproject(&pyproject, dir);
                self.by_dir.insert(dir.to_path_buf(), meta.clone());
                return meta;
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

fn parse_pyproject(path: &Path, dir: &Path) -> Option<PyProjectMeta> {
    let raw = fs::read_to_string(path).ok()?;
    let val: toml::Value = toml::from_str(&raw).ok()?;
    let mut source_roots = vec![dir.to_path_buf()];
    let src_dir = dir.join("src");
    if src_dir.is_dir() {
        source_roots.push(src_dir);
    }
    if let Some(paths) = val
        .get("tool")
        .and_then(|t| t.get("pytest"))
        .and_then(|p| p.get("ini_options"))
        .and_then(|o| o.get("pythonpath"))
        .and_then(|v| v.as_array())
    {
        for p in paths {
            if let Some(s) = p.as_str() {
                if let Some(joined) = safe_join(dir, s, Some(dir)) {
                    source_roots.push(joined);
                }
            }
        }
    }
    Some(PyProjectMeta {
        root_dir: dir.to_path_buf(),
        source_roots,
    })
}

pub fn resolve_python_import(
    cache: &PyProjectCache,
    from_file: &Path,
    module_specifier: &str,
    workspace_root: Option<&Path>,
) -> ImportResolution {
    if module_specifier.is_empty() {
        return ImportResolution::Unresolved;
    }

    // Relative import: `.foo`, `..bar`, `.`
    if module_specifier.starts_with('.') {
        let dots = module_specifier.chars().take_while(|&c| c == '.').count();
        let rest = &module_specifier[dots..];
        let mut base = match from_file.parent() {
            Some(p) => p.to_path_buf(),
            None => return ImportResolution::Unresolved,
        };
        for _ in 1..dots {
            base = match base.parent() {
                Some(p) => p.to_path_buf(),
                None => return ImportResolution::Unresolved,
            };
        }
        if let Some(found) = probe_module_path(&base, rest, workspace_root) {
            return ImportResolution::Resolved(found);
        }
        return ImportResolution::Unresolved;
    }

    // Absolute package import via pyproject.toml / src/ / workspace_root
    if let Some(meta) = cache.find_for_file(from_file, workspace_root) {
        for root in &meta.source_roots {
            if let Some(found) = probe_module_path(root, module_specifier, workspace_root) {
                return ImportResolution::Resolved(found);
            }
        }
    }
    if let Some(root) = workspace_root {
        if let Some(found) = probe_module_path(root, module_specifier, Some(root)) {
            return ImportResolution::Resolved(found);
        }
        let src = root.join("src");
        if src.is_dir() {
            if let Some(found) = probe_module_path(&src, module_specifier, Some(root)) {
                return ImportResolution::Resolved(found);
            }
        }
    }
    ImportResolution::Unresolved
}

fn probe_module_path(base: &Path, dotted: &str, workspace_root: Option<&Path>) -> Option<PathBuf> {
    if dotted.is_empty() {
        let init = base.join("__init__.py");
        return init.is_file().then_some(init);
    }
    let rel = dotted.replace('.', "/");
    let joined = safe_join(base, &rel, workspace_root)?;
    let py_file = joined.with_extension("py");
    if py_file.is_file() {
        return Some(py_file);
    }
    let pyi_file = joined.with_extension("pyi");
    if pyi_file.is_file() {
        return Some(pyi_file);
    }
    let init = joined.join("__init__.py");
    if init.is_file() {
        return Some(init);
    }
    None
}
