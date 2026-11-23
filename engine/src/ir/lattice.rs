use crate::protocol::schema::{AbsenceSummary, UnknownReason};

pub const STATE_UNREACHABLE: u8 = 0;
pub const STATE_PRESENT: u8 = 1 << 0;
pub const STATE_NULL: u8 = 1 << 1;
pub const STATE_UNDEFINED: u8 = 1 << 2;
pub const STATE_ABSENT_ALL: u8 = STATE_NULL | STATE_UNDEFINED;

/// 2-axis product absence lattice:
/// - `known_states`: proven adapter-native states (Present, Null, Undefined)
/// - `unknown_reasons`: bitmask of `UnknownReason`s
///
/// Crucially, `Unknown` is orthogonal to `Absent`:
/// `join(Present, Unknown(r))` keeps `known_states = STATE_PRESENT` and `unknown_reasons = r`,
/// so `is_maybe_absent()` remains `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AbsenceValue {
    pub known_states: u8,
    pub unknown_reasons: u32,
}

impl AbsenceValue {
    pub const fn bottom() -> Self {
        Self {
            known_states: STATE_UNREACHABLE,
            unknown_reasons: 0,
        }
    }

    pub const fn present() -> Self {
        Self {
            known_states: STATE_PRESENT,
            unknown_reasons: 0,
        }
    }

    pub const fn null() -> Self {
        Self {
            known_states: STATE_NULL,
            unknown_reasons: 0,
        }
    }

    pub const fn undefined() -> Self {
        Self {
            known_states: STATE_UNDEFINED,
            unknown_reasons: 0,
        }
    }

    pub const fn maybe_null() -> Self {
        Self {
            known_states: STATE_PRESENT | STATE_NULL,
            unknown_reasons: 0,
        }
    }

    pub const fn maybe_undefined() -> Self {
        Self {
            known_states: STATE_PRESENT | STATE_UNDEFINED,
            unknown_reasons: 0,
        }
    }

    pub const fn maybe_absent() -> Self {
        Self {
            known_states: STATE_PRESENT | STATE_NULL | STATE_UNDEFINED,
            unknown_reasons: 0,
        }
    }

    pub fn unknown(reason: UnknownReason) -> Self {
        Self {
            known_states: STATE_UNREACHABLE,
            unknown_reasons: reason_bit(reason),
        }
    }

    pub fn with_unknown(mut self, reason: UnknownReason) -> Self {
        self.unknown_reasons |= reason_bit(reason);
        self
    }

    pub fn join(self, other: Self) -> Self {
        Self {
            known_states: self.known_states | other.known_states,
            unknown_reasons: self.unknown_reasons | other.unknown_reasons,
        }
    }

    pub fn narrow_mask(self, mask: u8) -> Self {
        Self {
            known_states: self.known_states & mask,
            unknown_reasons: self.unknown_reasons,
        }
    }

    pub fn is_present_only(self) -> bool {
        (self.known_states & STATE_PRESENT) != 0
            && (self.known_states & STATE_ABSENT_ALL) == 0
            && self.unknown_reasons == 0
    }

    pub fn is_maybe_absent(self) -> bool {
        (self.known_states & STATE_ABSENT_ALL) != 0
    }

    pub fn is_definitely_absent(self) -> bool {
        (self.known_states & STATE_PRESENT) == 0 && (self.known_states & STATE_ABSENT_ALL) != 0
    }

    pub fn has_unknown(self) -> bool {
        self.unknown_reasons != 0
    }

    pub fn reasons(&self) -> Vec<UnknownReason> {
        let mut out = Vec::new();
        for reason in ALL_REASONS {
            if (self.unknown_reasons & reason_bit(reason)) != 0 {
                out.push(reason);
            }
        }
        out
    }

    pub fn to_summary(&self) -> AbsenceSummary {
        AbsenceSummary {
            present: (self.known_states & STATE_PRESENT) != 0,
            maybe_null: (self.known_states & STATE_NULL) != 0,
            maybe_undefined: (self.known_states & STATE_UNDEFINED) != 0,
            unknown_reasons: self.reasons(),
        }
    }
}

const ALL_REASONS: [UnknownReason; 13] = [
    UnknownReason::ExternalUntyped,
    UnknownReason::DynamicDispatch,
    UnknownReason::LspTimeout,
    UnknownReason::LspUnattached,
    UnknownReason::LspEmpty,
    UnknownReason::CircuitOpen,
    UnknownReason::HopBudgetExhausted,
    UnknownReason::BarrelFanoutCap,
    UnknownReason::FileTooLarge,
    UnknownReason::NonRegularFile,
    UnknownReason::TreeSitterErrorNode,
    UnknownReason::PolymorphicCap,
    UnknownReason::MaxDepth,
];

pub const fn reason_bit(reason: UnknownReason) -> u32 {
    let idx = match reason {
        UnknownReason::ExternalUntyped => 0,
        UnknownReason::DynamicDispatch => 1,
        UnknownReason::LspTimeout => 2,
        UnknownReason::LspUnattached => 3,
        UnknownReason::LspEmpty => 4,
        UnknownReason::CircuitOpen => 5,
        UnknownReason::HopBudgetExhausted => 6,
        UnknownReason::BarrelFanoutCap => 7,
        UnknownReason::FileTooLarge => 8,
        UnknownReason::NonRegularFile => 9,
        UnknownReason::TreeSitterErrorNode => 10,
        UnknownReason::PolymorphicCap => 11,
        UnknownReason::MaxDepth => 12,
    };
    1u32 << idx
}
