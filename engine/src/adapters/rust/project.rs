use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::adapters::typescript::tsconfig::safe_join;
use crate::adapters::ImportResolution;

#[derive(Debug, Clone, Default)]
pub struct CargoWorkspaceMeta {
    pub root_dir: PathBuf,
    pub package_name: Option<String>,
    pub is_workspace: bool,
    pub member_crates: HashMap<String, PathBuf>,
}

#[derive(Default)]
pub struct CargoWorkspaceCache {
    by_dir: Mutex<HashMap<PathBuf, Option<CargoWorkspaceMeta>>>,
}

impl CargoWorkspaceCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn find_for_file(
        &self,
        file: &Path,
        workspace_root: Option<&Path>,
    ) -> Option<CargoWorkspaceMeta> {
        let start_dir = file.parent()?;
        let mut cur = Some(start_dir);
        let mut first_pkg: Option<CargoWorkspaceMeta> = None;

        while let Some(dir) = cur {
            {
                let cache = self.by_dir.lock().expect("cargo cache lock");
                if let Some(Some(meta)) = cache.get(dir) {
                    if meta.is_workspace {
                        return Some(meta.clone());
                    }
                    if first_pkg.is_none() {
                        first_pkg = Some(meta.clone());
                    }
                }
            }

            let cargo_toml = dir.join("Cargo.toml");
            if cargo_toml.is_file() {
                let meta = parse_cargo_toml(&cargo_toml, dir);
                let mut cache = self.by_dir.lock().expect("cargo cache lock");
                cache.insert(dir.to_path_buf(), meta.clone());
                if let Some(m) = meta {
                    if m.is_workspace {
                        return Some(m);
                    }
                    if first_pkg.is_none() {
                        first_pkg = Some(m);
                    }
                }
            }

            if let Some(root) = workspace_root {
                if dir == root {
                    break;
                }
            }
            cur = dir.parent();
        }
        first_pkg
    }
}

fn parse_cargo_toml(path: &Path, dir: &Path) -> Option<CargoWorkspaceMeta> {
    let raw = fs::read_to_string(path).ok()?;
    let val: toml::Value = toml::from_str(&raw).ok()?;
    let package_name = val
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.replace('-', "_"));

    let mut member_crates = HashMap::new();
    if let Some(name) = &package_name {
        member_crates.insert(name.clone(), dir.to_path_buf());
    }

    let mut is_workspace = false;
    if let Some(members) = val
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
    {
        is_workspace = true;
        for m in members {
            let Some(pat) = m.as_str() else { continue };
            if let Some(prefix) = pat.strip_suffix("/*") {
                if let Some(parent_dir) = safe_join(dir, prefix, Some(dir)) {
                    if let Ok(entries) = fs::read_dir(&parent_dir) {
                        for entry in entries.flatten().take(64) {
                            let p = entry.path();
                            if p.is_dir() && p.join("Cargo.toml").is_file() {
                                if let Some(pkg) = read_package_name(&p.join("Cargo.toml")) {
                                    member_crates.insert(pkg, p);
                                }
                            }
                        }
                    }
                }
            } else if let Some(member_dir) = safe_join(dir, pat, Some(dir)) {
                if member_dir.join("Cargo.toml").is_file() {
                    if let Some(pkg) = read_package_name(&member_dir.join("Cargo.toml")) {
                        member_crates.insert(pkg, member_dir);
                    }
                }
            }
        }
    }

    Some(CargoWorkspaceMeta {
        root_dir: dir.to_path_buf(),
        package_name,
        is_workspace,
        member_crates,
    })
}

fn read_package_name(cargo_toml: &Path) -> Option<String> {
    let raw = fs::read_to_string(cargo_toml).ok()?;
    let val: toml::Value = toml::from_str(&raw).ok()?;
    val.get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.replace('-', "_"))
}

pub fn resolve_rust_import(
    cache: &CargoWorkspaceCache,
    from_file: &Path,
    module_specifier: &str,
    workspace_root: Option<&Path>,
) -> ImportResolution {
    if module_specifier.is_empty() {
        return ImportResolution::Unresolved;
    }

    // `mod foo;`
    if let Some(mod_name) = module_specifier.strip_prefix("mod:") {
        if let Some(dir) = from_file.parent() {
            if let Some(found) = probe_rust_mod(dir, &[mod_name], workspace_root) {
                return ImportResolution::Resolved(found);
            }
        }
        return ImportResolution::Unresolved;
    }

    let parts: Vec<&str> = module_specifier
        .split("::")
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return ImportResolution::Unresolved;
    }

    match parts[0] {
        "self" => {
            if let Some(dir) = from_file.parent() {
                if let Some(found) = probe_rust_mod(dir, &parts[1..], workspace_root) {
                    return ImportResolution::Resolved(found);
                }
            }
        }
        "super" => {
            if let Some(dir) = from_file.parent().and_then(|p| p.parent()) {
                if let Some(found) = probe_rust_mod(dir, &parts[1..], workspace_root) {
                    return ImportResolution::Resolved(found);
                }
            }
        }
        "crate" => {
            if let Some(src_dir) = find_crate_src_dir(from_file, workspace_root) {
                if let Some(found) = probe_rust_mod(&src_dir, &parts[1..], workspace_root) {
                    return ImportResolution::Resolved(found);
                }
            }
        }
        crate_name => {
            if let Some(meta) = cache.find_for_file(from_file, workspace_root) {
                if let Some(crate_dir) = meta.member_crates.get(crate_name) {
                    let src_dir = crate_dir.join("src");
                    if let Some(found) = probe_rust_mod(&src_dir, &parts[1..], workspace_root) {
                        return ImportResolution::Resolved(found);
                    }
                }
            }
        }
    }

    ImportResolution::Unresolved
}

fn find_crate_src_dir(from_file: &Path, workspace_root: Option<&Path>) -> Option<PathBuf> {
    let mut cur = from_file.parent();
    while let Some(dir) = cur {
        if dir.join("Cargo.toml").is_file() {
            let src = dir.join("src");
            if src.is_dir() {
                return Some(src);
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

fn probe_rust_mod(
    base_dir: &Path,
    segments: &[&str],
    workspace_root: Option<&Path>,
) -> Option<PathBuf> {
    if segments.is_empty() {
        let lib = base_dir.join("lib.rs");
        if lib.is_file() {
            return Some(lib);
        }
        let main = base_dir.join("main.rs");
        if main.is_file() {
            return Some(main);
        }
        return None;
    }

    // Try full path, then dropping trailing symbol name (e.g., `crate::auth::verify` -> `auth.rs`)
    for len in [segments.len(), segments.len().saturating_sub(1)] {
        if len == 0 {
            continue;
        }
        let rel = segments[..len].join("/");
        let Some(joined) = safe_join(base_dir, &rel, workspace_root) else {
            continue;
        };
        let rs_file = joined.with_extension("rs");
        if rs_file.is_file() {
            return Some(rs_file);
        }
        let mod_rs = joined.join("mod.rs");
        if mod_rs.is_file() {
            return Some(mod_rs);
        }
    }
    None
}
