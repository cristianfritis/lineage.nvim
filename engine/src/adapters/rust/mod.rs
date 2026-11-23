pub mod lower;
pub mod project;
pub mod stdlib;

use std::path::Path;
use std::time::Instant;

use crate::adapters::{ImportResolution, LanguageAdapter};
use crate::ir::{AbsenceValue, FileSkeleton, FunctionIr, FunctionSkeleton};

use lower::{lower_rust_function, parse_rust_skeleton};
use project::{resolve_rust_import, CargoWorkspaceCache};
use stdlib::lookup_rust_stdlib;

pub struct RustAdapter {
    workspace_cache: CargoWorkspaceCache,
}

impl RustAdapter {
    pub fn new() -> Self {
        Self {
            workspace_cache: CargoWorkspaceCache::new(),
        }
    }
}

impl Default for RustAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for RustAdapter {
    fn language_id(&self) -> &'static str {
        "rust"
    }

    fn supports_extension(&self, ext: &str) -> bool {
        ext.eq_ignore_ascii_case("rs")
    }

    fn parse_skeleton(&self, uri: &str, source: &str) -> FileSkeleton {
        parse_rust_skeleton(uri, source)
    }

    fn lower_function(&self, uri: &str, source: &str, skeleton: &FunctionSkeleton) -> FunctionIr {
        let skel = parse_rust_skeleton(uri, source);
        lower_rust_function(uri, source, skeleton, &skel.imports)
    }

    fn extract_declaration_signature(
        &self,
        _uri: &str,
        source: &str,
        symbol: &str,
    ) -> Option<AbsenceValue> {
        lower::extract_rust_fn_signature(source, symbol)
    }

    fn resolve_static_import(
        &self,
        from_file: &Path,
        module_specifier: &str,
        workspace_root: Option<&Path>,
        _budget_start: Instant,
    ) -> ImportResolution {
        resolve_rust_import(
            &self.workspace_cache,
            from_file,
            module_specifier,
            workspace_root,
        )
    }

    fn stdlib_summary(&self, symbol: &str) -> Option<AbsenceValue> {
        lookup_rust_stdlib(symbol)
    }

    fn classify_test_file(&self, uri: &str) -> bool {
        let fname = uri.rsplit('/').next().unwrap_or(uri);
        uri.contains("/tests/")
            || uri.contains("/test/")
            || fname.starts_with("test_")
            || fname.ends_with("_test.rs")
    }
}
