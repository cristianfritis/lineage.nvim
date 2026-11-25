pub mod lower;
pub mod project;
pub mod stdlib;

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::adapters::{ImportResolution, LanguageAdapter};
use crate::ir::{AbsenceValue, FileSkeleton, FunctionIr, FunctionSkeleton};

use lower::{lower_go_function, parse_go_skeleton};
use project::{resolve_go_import, scan_package_siblings, GoModCache};
use stdlib::lookup_go_stdlib;

pub struct GoAdapter {
    mod_cache: GoModCache,
}

impl GoAdapter {
    pub fn new() -> Self {
        Self {
            mod_cache: GoModCache::new(),
        }
    }
}

impl Default for GoAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for GoAdapter {
    fn language_id(&self) -> &'static str {
        "go"
    }

    fn supports_extension(&self, ext: &str) -> bool {
        ext.eq_ignore_ascii_case("go")
    }

    fn parse_skeleton(&self, uri: &str, source: &str) -> FileSkeleton {
        parse_go_skeleton(uri, source)
    }

    fn lower_function(
        &self,
        uri: &str,
        source: &str,
        file_skeleton: &FileSkeleton,
        skeleton: &FunctionSkeleton,
    ) -> FunctionIr {
        lower_go_function(uri, source, skeleton, &file_skeleton.imports)
    }

    fn extract_declaration_signature(
        &self,
        _uri: &str,
        _source: &str,
        _symbol: &str,
    ) -> Option<AbsenceValue> {
        None
    }

    fn resolve_static_import(
        &self,
        from_file: &Path,
        module_specifier: &str,
        workspace_root: Option<&Path>,
        _budget_start: Instant,
    ) -> ImportResolution {
        resolve_go_import(&self.mod_cache, from_file, module_specifier, workspace_root)
    }

    fn resolve_same_package_siblings(
        &self,
        from_file: &Path,
        exclude_tests: bool,
        budget_start: Instant,
    ) -> Vec<PathBuf> {
        scan_package_siblings(from_file, exclude_tests, budget_start)
    }

    fn stdlib_summary(&self, symbol: &str) -> Option<AbsenceValue> {
        lookup_go_stdlib(symbol)
    }

    fn classify_test_file(&self, uri: &str) -> bool {
        uri.ends_with("_test.go") || uri.contains("/testdata/")
    }
}
