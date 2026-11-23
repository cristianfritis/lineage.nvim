use std::collections::HashSet;

use crate::ir::AbsenceValue;
use crate::protocol::schema::{
    CanonicalRange, FileEntry, HoverCardProjection, HoverItem, HoverRisk, TruncationReason,
    UnknownBoundary,
};

#[allow(clippy::too_many_arguments)]
pub fn project_hover_card(
    symbol: &str,
    root_absence: AbsenceValue,
    files: &[FileEntry],
    phase1_candidates: Vec<HoverItem>,
    phase2_candidates: Vec<HoverItem>,
    guards: Vec<String>,
    local_risks: Vec<HoverRisk>,
    unknown_boundaries: Vec<UnknownBoundary>,
    truncation: &[TruncationReason],
) -> HoverCardProjection {
    let _ = files;
    let unverified_count = unknown_boundaries.len();
    let has_truncation = !truncation.is_empty();
    let is_partial = unverified_count > 0 || has_truncation;

    let status_badge = if root_absence.is_definitely_absent() {
        "Absent".to_string()
    } else if root_absence.is_maybe_absent() {
        if root_absence.to_summary().maybe_null && root_absence.to_summary().maybe_undefined {
            "Nullable (null | undefined)".to_string()
        } else if root_absence.to_summary().maybe_null {
            "Nullable (null)".to_string()
        } else {
            "Nullable (undefined)".to_string()
        }
    } else if root_absence.is_present_only() {
        "NonNull".to_string()
    } else if is_partial {
        "Unverified".to_string()
    } else {
        "NonNull".to_string()
    };

    let verdict_line = if is_partial && unverified_count > 0 {
        format!(
            "{}: {} (partial — {} unverified [gu])",
            symbol, status_badge, unverified_count
        )
    } else if is_partial {
        format!("{}: {} (partial — truncated)", symbol, status_badge)
    } else {
        format!("{}: {}", symbol, status_badge)
    };

    let absent_origin_count = phase1_candidates
        .iter()
        .chain(phase2_candidates.iter())
        .filter(|i| i.is_absent_origin)
        .count();

    let is_trivial_peek = root_absence.is_present_only()
        && absent_origin_count == 0
        && local_risks.is_empty()
        && unknown_boundaries.is_empty()
        && !has_truncation;

    let peek_lines = if is_trivial_peek {
        let mut lines = vec![verdict_line.clone()];
        if let Some(first) = phase1_candidates.first() {
            lines.push(format!("  origin: {}", first.label));
        }
        lines
    } else {
        Vec::new()
    };

    // Phase 1 items: capped at 5 when open cross-file boundaries exist, up to 9 otherwise.
    let phase1_cap = if unverified_count > 0 { 5 } else { 9 };
    let mut seen: HashSet<(u32, CanonicalRange)> = HashSet::new();
    let mut phase1_items = Vec::new();
    for mut item in phase1_candidates {
        if phase1_items.len() >= phase1_cap {
            break;
        }
        if seen.insert((item.file_id, item.range)) {
            item.index = phase1_items.len() + 1;
            phase1_items.push(item);
        }
    }

    let mut phase2_items = Vec::new();
    let mut next_idx = phase1_items.len() + 1;
    for mut item in phase2_candidates {
        if next_idx > 9 {
            break;
        }
        if seen.insert((item.file_id, item.range)) {
            item.index = next_idx;
            next_idx += 1;
            phase2_items.push(item);
        }
    }

    HoverCardProjection {
        symbol: symbol.to_string(),
        verdict_line,
        status_badge,
        is_trivial_peek,
        peek_lines,
        phase1_items,
        phase2_items,
        guards,
        local_risks,
        unknown_boundaries,
        unverified_count,
    }
}
