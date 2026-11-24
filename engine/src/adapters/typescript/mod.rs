pub mod lower;
pub mod stdlib;
pub mod stub_extract;
pub mod tsconfig;

use std::path::Path;
use std::time::Instant;

use crate::adapters::{ImportResolution, LanguageAdapter};
use crate::ir::{AbsenceValue, FileSkeleton, FunctionIr, FunctionSkeleton};

use lower::{lower_ts_function, parse_ts_skeleton};
use stdlib::lookup_stdlib_summary;
use stub_extract::extract_dts_outer_wrapper;
use tsconfig::{resolve_import_with_tsconfig, TsConfigCache};

pub struct TypeScriptAdapter {
    tsconfig_cache: TsConfigCache,
}

impl TypeScriptAdapter {
    pub fn new() -> Self {
        Self {
            tsconfig_cache: TsConfigCache::new(),
        }
    }
}

impl Default for TypeScriptAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for TypeScriptAdapter {
    fn language_id(&self) -> &'static str {
        "typescript"
    }

    fn supports_extension(&self, ext: &str) -> bool {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs"
        )
    }

    fn parse_skeleton(&self, uri: &str, source: &str) -> FileSkeleton {
        parse_ts_skeleton(uri, source)
    }

    fn lower_function(
        &self,
        uri: &str,
        source: &str,
        file_skeleton: &FileSkeleton,
        skeleton: &FunctionSkeleton,
    ) -> FunctionIr {
        lower_ts_function(uri, source, skeleton, &file_skeleton.imports)
    }

    fn extract_declaration_signature(
        &self,
        _uri: &str,
        source: &str,
        symbol: &str,
    ) -> Option<AbsenceValue> {
        extract_dts_outer_wrapper(source, symbol)
    }

    fn resolve_static_import(
        &self,
        from_file: &Path,
        module_specifier: &str,
        workspace_root: Option<&Path>,
        budget_start: Instant,
    ) -> ImportResolution {
        resolve_import_with_tsconfig(
            &self.tsconfig_cache,
            from_file,
            module_specifier,
            workspace_root,
            budget_start,
        )
    }

    fn stdlib_summary(&self, symbol: &str) -> Option<AbsenceValue> {
        lookup_stdlib_summary(symbol)
    }

    fn classify_test_file(&self, uri: &str) -> bool {
        uri.contains(".test.")
            || uri.contains(".spec.")
            || uri.contains("__tests__")
            || uri.contains("/test/")
            || uri.contains("/tests/")
    }
}
