use tree_sitter::{Node, Parser};

use crate::ir::{AbsenceValue, STATE_NULL, STATE_PRESENT};
use crate::protocol::schema::UnknownReason;

/// Declaration-only outer-wrapper signature extractor for `.pyi` stub files.
/// Inspects only the outermost return type (`Optional[T]`, `T | None`, `Union[T, None]`)
/// without generic instantiation.
pub fn extract_pyi_outer_wrapper(source: &str, symbol: &str) -> Option<AbsenceValue> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    find_pyi_symbol_return(tree.root_node(), source.as_bytes(), symbol)
}

fn find_pyi_symbol_return(node: Node<'_>, src: &[u8], symbol: &str) -> Option<AbsenceValue> {
    if node.kind() == "function_definition" {
        if let Some(name_node) = node.child_by_field_name("name") {
            if name_node.utf8_text(src).ok() == Some(symbol) {
                if let Some(ret_node) = node.child_by_field_name("return_type") {
                    return Some(classify_py_type_text(
                        ret_node.utf8_text(src).unwrap_or(""),
                    ));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_pyi_symbol_return(child, src, symbol) {
            return Some(found);
        }
    }
    None
}

pub fn classify_py_type_text(raw: &str) -> AbsenceValue {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return AbsenceValue::present();
    }
    if trimmed == "None" {
        return AbsenceValue::null();
    }
    // Check `Optional[...]` or `Union[..., None]`
    if trimmed.starts_with("Optional[") && trimmed.ends_with(']') {
        return AbsenceValue {
            known_states: STATE_PRESENT | STATE_NULL,
            unknown_reasons: 0,
        };
    }
    if trimmed.starts_with("Union[") && trimmed.ends_with(']') {
        let inner = &trimmed["Union[".len()..trimmed.len() - 1];
        if split_top_level(inner, ',').iter().any(|p| p.trim() == "None") {
            return AbsenceValue {
                known_states: STATE_PRESENT | STATE_NULL,
                unknown_reasons: 0,
            };
        }
    }
    // Check PEP 604 union `T | None`
    let parts = split_top_level(trimmed, '|');
    if parts.len() > 1 {
        if parts.iter().any(|p| p.trim() == "None") {
            return AbsenceValue {
                known_states: STATE_PRESENT | STATE_NULL,
                unknown_reasons: 0,
            };
        }
        if parts.iter().any(|p| is_bare_py_typevar(p.trim())) {
            return AbsenceValue::unknown(UnknownReason::ExternalUntyped);
        }
        return AbsenceValue::present();
    }

    if is_bare_py_typevar(trimmed) {
        return AbsenceValue::unknown(UnknownReason::ExternalUntyped);
    }
    AbsenceValue::present()
}

fn is_bare_py_typevar(s: &str) -> bool {
    matches!(s, "T" | "U" | "V" | "K" | "R" | "Any" | "_T")
}

fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                out.push(&s[start..idx]);
                start = idx + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}
