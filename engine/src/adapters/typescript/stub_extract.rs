use tree_sitter::{Node, Parser};

use crate::ir::{AbsenceValue, STATE_NULL, STATE_PRESENT, STATE_UNDEFINED};
use crate::protocol::schema::UnknownReason;

/// Declaration-only outer-wrapper signature extractor for `.d.ts` files.
/// Inspects only the outermost return type union (`T | null`, `T | undefined`, `T | null | undefined`)
/// without generic instantiation or full type-checking.
pub fn extract_dts_outer_wrapper(source: &str, symbol: &str) -> Option<AbsenceValue> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    find_symbol_return_type(tree.root_node(), source.as_bytes(), symbol)
}

fn find_symbol_return_type(node: Node<'_>, src: &[u8], symbol: &str) -> Option<AbsenceValue> {
    let kind = node.kind();
    if matches!(
        kind,
        "function_signature" | "method_signature" | "function_declaration" | "method_definition"
    ) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = name_node.utf8_text(src).ok()?;
            if name == symbol {
                if let Some(ret_node) = node.child_by_field_name("return_type") {
                    return Some(classify_type_annotation(ret_node, src));
                }
            }
        }
    }

    if kind == "variable_declarator" {
        if let Some(name_node) = node.child_by_field_name("name") {
            if name_node.utf8_text(src).ok() == Some(symbol) {
                if let Some(type_node) = node.child_by_field_name("type") {
                    return Some(classify_type_annotation(type_node, src));
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_symbol_return_type(child, src, symbol) {
            return Some(found);
        }
    }
    None
}

pub fn classify_type_annotation(node: Node<'_>, src: &[u8]) -> AbsenceValue {
    let mut target = node;
    if target.kind() == "type_annotation" {
        if let Some(inner) = target.named_child(0) {
            target = inner;
        }
    }

    match target.kind() {
        "union_type" => {
            let mut mask = 0u8;
            let mut has_bare_type_param = false;
            let mut cursor = target.walk();
            for child in target.named_children(&mut cursor) {
                let text = child.utf8_text(src).unwrap_or("").trim();
                match text {
                    "null" => mask |= STATE_NULL,
                    "undefined" | "void" => mask |= STATE_UNDEFINED,
                    _ => {
                        if is_bare_type_param(text) {
                            has_bare_type_param = true;
                        }
                        mask |= STATE_PRESENT;
                    }
                }
            }
            if mask == 0 {
                AbsenceValue::unknown(UnknownReason::ExternalUntyped)
            } else if (mask & (STATE_NULL | STATE_UNDEFINED)) != 0 {
                AbsenceValue {
                    known_states: mask,
                    unknown_reasons: 0,
                }
            } else if has_bare_type_param {
                AbsenceValue::unknown(UnknownReason::ExternalUntyped)
            } else {
                AbsenceValue::present()
            }
        }
        "literal_type" | "predefined_type" | "type_identifier" => {
            let text = target.utf8_text(src).unwrap_or("").trim();
            match text {
                "null" => AbsenceValue::null(),
                "undefined" | "void" => AbsenceValue::undefined(),
                _ if is_bare_type_param(text) => {
                    AbsenceValue::unknown(UnknownReason::ExternalUntyped)
                }
                _ => AbsenceValue::present(),
            }
        }
        _ => AbsenceValue::present(),
    }
}

fn is_bare_type_param(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Single uppercase letter (T, U, K, V, R) or TData/TValue style generic parameter
    let first = trimmed.chars().next().unwrap();
    (trimmed.len() == 1 && first.is_ascii_uppercase())
        || (trimmed.starts_with('T')
            && trimmed.len() <= 8
            && trimmed.chars().all(|c| c.is_ascii_alphanumeric()))
}
