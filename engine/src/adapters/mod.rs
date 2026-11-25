use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::ir::{
    AbsenceValue, BasicBlock, BlockId, FileSkeleton, FnSummary, FunctionIr, FunctionSkeleton,
    SsiInst, SsiOp, ValueId,
};
use crate::protocol::schema::{CanonicalPos, CanonicalRange, UnknownReason};

pub mod go;
pub mod python;
pub mod rust;
pub mod typescript;

/// Pluggable language adapter contract for parsing, lowering, stdlib summaries,
/// `.d.ts`/stub extraction, static import resolution, and test-file classification.
pub trait LanguageAdapter: Send + Sync {
    fn language_id(&self) -> &'static str;

    fn supports_extension(&self, ext: &str) -> bool;

    fn parse_skeleton(&self, uri: &str, source: &str) -> FileSkeleton;

    fn lower_function(
        &self,
        uri: &str,
        source: &str,
        file_skeleton: &FileSkeleton,
        skeleton: &FunctionSkeleton,
    ) -> FunctionIr;

    fn extract_declaration_signature(
        &self,
        uri: &str,
        source: &str,
        symbol: &str,
    ) -> Option<AbsenceValue>;

    fn resolve_static_import(
        &self,
        from_file: &Path,
        module_specifier: &str,
        workspace_root: Option<&Path>,
        budget_start: Instant,
    ) -> ImportResolution;

    fn resolve_same_package_siblings(
        &self,
        _from_file: &Path,
        _exclude_tests: bool,
        _budget_start: Instant,
    ) -> Vec<PathBuf> {
        Vec::new()
    }

    fn stdlib_summary(&self, symbol: &str) -> Option<AbsenceValue>;

    fn classify_test_file(&self, uri: &str) -> bool;

    fn format_unknown_badge(&self, _reason: UnknownReason) -> &'static str {
        "Unverified"
    }
}

#[derive(Debug, Clone)]
pub enum ImportResolution {
    Resolved(PathBuf),
    BarrelCapReached,
    Unresolved,
}

/// In-memory StubAdapter for M0 unit and CLI tests.
pub struct StubAdapter;

impl LanguageAdapter for StubAdapter {
    fn language_id(&self) -> &'static str {
        "stub"
    }

    fn supports_extension(&self, ext: &str) -> bool {
        ext == "stub"
    }

    fn parse_skeleton(&self, uri: &str, source: &str) -> FileSkeleton {
        let hash = xxhash_rust::xxh3::xxh3_64(source.as_bytes());
        FileSkeleton {
            uri: uri.to_string(),
            content_hash: hash,
            imports: Vec::new(),
            exports: Vec::new(),
            functions: vec![FunctionSkeleton {
                symbol_id: "<module>".to_string(),
                name: "<module>".to_string(),
                range: CanonicalRange {
                    start: CanonicalPos { line: 0, col: 0 },
                    end: CanonicalPos { line: 0, col: 0 },
                },
                start_byte: 0,
                end_byte: source.len(),
                content_hash: hash,
                is_exported: false,
            }],
            parse_errors: Vec::new(),
        }
    }

    fn lower_function(
        &self,
        _uri: &str,
        _source: &str,
        _file_skeleton: &FileSkeleton,
        skeleton: &FunctionSkeleton,
    ) -> FunctionIr {
        let v0 = ValueId(0);
        FunctionIr {
            symbol_id: skeleton.symbol_id.clone(),
            name: skeleton.name.clone(),
            range: skeleton.range,
            params: Vec::new(),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                preds: Vec::new(),
                succs: Vec::new(),
                insts: vec![v0],
                guards: Vec::new(),
                terminated: true,
            }],
            insts: vec![SsiInst {
                id: v0,
                block: BlockId(0),
                name: Some("stub".to_string()),
                range: skeleton.range,
                op: SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: "stub".to_string(),
                },
                snippet: Some("stub".to_string()),
            }],
            returns: vec![v0],
            danger_sinks: Vec::new(),
            span_index: vec![(skeleton.range, v0)],
            local_summary: FnSummary {
                return_absence: AbsenceValue::present(),
                param_flow_to_return: Vec::new(),
                symbolic_callees: Vec::new(),
            },
        }
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
        _from_file: &Path,
        _module_specifier: &str,
        _workspace_root: Option<&Path>,
        _budget_start: Instant,
    ) -> ImportResolution {
        ImportResolution::Unresolved
    }

    fn stdlib_summary(&self, _symbol: &str) -> Option<AbsenceValue> {
        None
    }

    fn classify_test_file(&self, uri: &str) -> bool {
        uri.contains(".test.") || uri.contains(".spec.")
    }
}
