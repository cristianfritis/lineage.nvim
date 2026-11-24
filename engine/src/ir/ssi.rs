use crate::ir::guards::GuardFact;
use crate::ir::lattice::AbsenceValue;
use crate::protocol::schema::{CanonicalRange, UnknownReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ValueId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitEdge {
    Normal,
    EarlyExit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallTarget {
    Local(String),
    Imported {
        module_specifier: String,
        symbol: String,
    },
    Stdlib(String),
    Dynamic(String),
}

#[derive(Debug, Clone)]
pub enum SsiOp {
    Const {
        value: AbsenceValue,
        label: String,
    },
    Param {
        index: usize,
        name: String,
        default_absence: AbsenceValue,
    },
    Assign {
        src: ValueId,
    },
    Phi {
        incoming: Vec<(BlockId, ValueId)>,
    },
    Pi {
        src: ValueId,
        guard: GuardFact,
    },
    ProjectField {
        base: ValueId,
        field: String,
    },
    ProjectIndex {
        base: ValueId,
        index: Option<ValueId>,
    },
    UnwrapVariant {
        base: ValueId,
        variant: String,
    },
    Call {
        callee: CallTarget,
        inputs: Vec<ValueId>,
        outputs: Vec<ValueId>,
        exit_edge: ExitEdge,
    },
    UnknownOp {
        reason: UnknownReason,
        symbol: String,
    },
}

#[derive(Debug, Clone)]
pub struct SsiInst {
    pub id: ValueId,
    pub block: BlockId,
    pub name: Option<String>,
    pub range: CanonicalRange,
    pub op: SsiOp,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub preds: Vec<BlockId>,
    pub succs: Vec<BlockId>,
    pub insts: Vec<ValueId>,
    pub guards: Vec<GuardFact>,
    pub terminated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerSinkKind {
    MemberAccess,
    Call,
    ElementIndex,
}

#[derive(Debug, Clone)]
pub struct DangerSink {
    pub kind: DangerSinkKind,
    pub target: ValueId,
    pub range: CanonicalRange,
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct FnSummary {
    pub return_absence: AbsenceValue,
    pub param_flow_to_return: Vec<usize>,
    pub symbolic_callees: Vec<CallTarget>,
}

#[derive(Debug, Clone)]
pub struct FunctionIr {
    pub symbol_id: String,
    pub name: String,
    pub range: CanonicalRange,
    pub params: Vec<(String, ValueId)>,
    pub blocks: Vec<BasicBlock>,
    pub insts: Vec<SsiInst>,
    pub returns: Vec<ValueId>,
    pub danger_sinks: Vec<DangerSink>,
    pub span_index: Vec<(CanonicalRange, ValueId)>,
    pub local_summary: FnSummary,
}

impl FunctionIr {
    pub fn inst(&self, id: ValueId) -> Option<&SsiInst> {
        self.insts.get(id.0 as usize)
    }

    pub fn lookup_at_pos(&self, pos: crate::protocol::schema::CanonicalPos) -> Option<ValueId> {
        // Find innermost span containing `pos`.
        let mut best: Option<(u32, ValueId)> = None;
        for (range, vid) in &self.span_index {
            if range.start <= pos && pos <= range.end {
                let width = range
                    .end
                    .line
                    .saturating_sub(range.start.line)
                    .saturating_mul(10_000)
                    .saturating_add(range.end.col.saturating_sub(range.start.col));
                if best.map(|(w, _)| width <= w).unwrap_or(true) {
                    best = Some((width, *vid));
                }
            }
        }
        best.map(|(_, vid)| vid)
    }
}

/// Compute 1-hop parameter indices that flow directly to any return value.
pub fn compute_param_flow_to_return(insts: &[SsiInst], returns: &[ValueId]) -> Vec<usize> {
    use std::collections::{HashSet, VecDeque};

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut params = Vec::new();

    for &ret in returns {
        if visited.insert(ret) {
            queue.push_back(ret);
        }
    }

    while let Some(vid) = queue.pop_front() {
        let Some(inst) = insts.get(vid.0 as usize) else {
            continue;
        };
        match &inst.op {
            SsiOp::Param { index, .. } => {
                if !params.contains(index) {
                    params.push(*index);
                }
            }
            SsiOp::Assign { src } | SsiOp::Pi { src, .. } => {
                if visited.insert(*src) {
                    queue.push_back(*src);
                }
            }
            SsiOp::Phi { incoming } => {
                for &(_b, src) in incoming {
                    if visited.insert(src) {
                        queue.push_back(src);
                    }
                }
            }
            SsiOp::ProjectField { base, .. }
            | SsiOp::ProjectIndex { base, .. }
            | SsiOp::UnwrapVariant { base, .. } => {
                if visited.insert(*base) {
                    queue.push_back(*base);
                }
            }
            SsiOp::Const { .. } | SsiOp::Call { .. } | SsiOp::UnknownOp { .. } => {}
        }
    }

    params.sort_unstable();
    params
}

#[derive(Debug, Clone)]
pub struct ImportSpec {
    pub local_name: String,
    pub imported_name: String,
    pub module_specifier: String,
    pub is_type_only: bool,
    pub range: CanonicalRange,
}

#[derive(Debug, Clone)]
pub struct ExportSpec {
    pub exported_name: String,
    pub local_name: Option<String>,
    pub module_specifier: Option<String>,
    pub is_wildcard: bool,
    pub range: CanonicalRange,
}

#[derive(Debug, Clone)]
pub struct FunctionSkeleton {
    pub symbol_id: String,
    pub name: String,
    pub range: CanonicalRange,
    pub start_byte: usize,
    pub end_byte: usize,
    pub content_hash: u64,
    pub is_exported: bool,
}

#[derive(Debug, Clone)]
pub struct FileSkeleton {
    pub uri: String,
    pub content_hash: u64,
    pub imports: Vec<ImportSpec>,
    pub exports: Vec<ExportSpec>,
    pub functions: Vec<FunctionSkeleton>,
    pub parse_errors: Vec<CanonicalRange>,
}
