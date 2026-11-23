use crate::ir::AbsenceValue;

/// Built-in JS/TS stdlib return absence summaries.
pub fn lookup_stdlib_summary(symbol: &str) -> Option<AbsenceValue> {
    let tail = symbol.rsplit('.').next().unwrap_or(symbol);
    match symbol {
        "Array.find" | "Array.prototype.find" | "Map.get" | "Map.prototype.get" | "WeakMap.get" => {
            Some(AbsenceValue::maybe_undefined())
        }
        "RegExp.exec"
        | "RegExp.prototype.exec"
        | "Document.querySelector"
        | "document.querySelector"
        | "Element.querySelector" => Some(AbsenceValue::maybe_null()),
        "Array.pop" | "Array.prototype.pop" | "Array.shift" | "Array.prototype.shift" => {
            Some(AbsenceValue::maybe_undefined())
        }
        _ => match tail {
            "find" | "get" | "pop" | "shift" => Some(AbsenceValue::maybe_undefined()),
            "exec" | "querySelector" => Some(AbsenceValue::maybe_null()),
            _ => None,
        },
    }
}
