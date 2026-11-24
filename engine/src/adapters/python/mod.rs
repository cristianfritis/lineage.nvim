pub mod lower;
pub mod project;
pub mod stdlib;
pub mod stub_extract;

use std::path::Path;
use std::time::Instant;

use crate::adapters::{ImportResolution, LanguageAdapter};
use crate::ir::{AbsenceValue, FileSkeleton, FunctionIr, FunctionSkeleton};

use lower::{lower_py_function, parse_py_skeleton};
use project::{resolve_python_import, PyProjectCache};
use stdlib::lookup_python_stdlib;
use stub_extract::extract_pyi_outer_wrapper;

pub struct PythonAdapter {
    project_cache: PyProjectCache,
}

impl PythonAdapter {
    pub fn new() -> Self {
        Self {
            project_cache: PyProjectCache::new(),
        }
    }
}

impl Default for PythonAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for PythonAdapter {
    fn language_id(&self) -> &'static str {
        "python"
    }

    fn supports_extension(&self, ext: &str) -> bool {
        matches!(ext.to_ascii_lowercase().as_str(), "py" | "pyi")
    }

    fn parse_skeleton(&self, uri: &str, source: &str) -> FileSkeleton {
        parse_py_skeleton(uri, source)
    }

    fn lower_function(
        &self,
        uri: &str,
        source: &str,
        file_skeleton: &FileSkeleton,
        skeleton: &FunctionSkeleton,
    ) -> FunctionIr {
        lower_py_function(uri, source, skeleton, &file_skeleton.imports)
    }

    fn extract_declaration_signature(
        &self,
        _uri: &str,
        source: &str,
        symbol: &str,
    ) -> Option<AbsenceValue> {
        extract_pyi_outer_wrapper(source, symbol)
    }

    fn resolve_static_import(
        &self,
        from_file: &Path,
        module_specifier: &str,
        workspace_root: Option<&Path>,
        _budget_start: Instant,
    ) -> ImportResolution {
        resolve_python_import(
            &self.project_cache,
            from_file,
            module_specifier,
            workspace_root,
        )
    }

    fn stdlib_summary(&self, symbol: &str) -> Option<AbsenceValue> {
        lookup_python_stdlib(symbol)
    }

    fn classify_test_file(&self, uri: &str) -> bool {
        let fname = uri.rsplit('/').next().unwrap_or(uri);
        fname.starts_with("test_")
            || fname.ends_with("_test.py")
            || uri.contains("/tests/")
            || uri.contains("/test/")
    }
}
