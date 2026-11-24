use crate::ir::{AbsenceValue, STATE_NULL, STATE_PRESENT, STATE_UNDEFINED};

/// Built-in Rust stdlib / Option / Result combinator absence summaries.
/// - `Option::None` maps to `STATE_NULL`.
/// - `Result::Err` maps to `STATE_UNDEFINED`.
pub fn lookup_rust_stdlib(symbol: &str) -> Option<AbsenceValue> {
    let tail = symbol.rsplit("::").next().unwrap_or(symbol);
    let method = tail.rsplit('.').next().unwrap_or(tail);

    match symbol {
        "None" | "Option::None" => return Some(AbsenceValue::null()),
        "Some" | "Option::Some" | "Ok" | "Result::Ok" => return Some(AbsenceValue::present()),
        "Err" | "Result::Err" => return Some(AbsenceValue::undefined()),
        "HashMap::get" | "BTreeMap::get" | "slice::get" | "Vec::get" | "Vec::pop"
        | "Iterator::next" => {
            return Some(AbsenceValue {
                known_states: STATE_PRESENT | STATE_NULL,
                unknown_reasons: 0,
            });
        }
        "std::fs::read_to_string" | "fs::read_to_string" | "File::open" | "std::env::var" => {
            return Some(AbsenceValue {
                known_states: STATE_PRESENT | STATE_UNDEFINED,
                unknown_reasons: 0,
            });
        }
        _ => {}
    }

    match method {
        "get" | "get_mut" | "first" | "last" | "pop" | "next" | "find" | "position"
        | "strip_prefix" | "strip_suffix" | "parent" | "file_name" | "extension" | "ok" | "err"
        | "and_then" | "or_else" | "map" | "filter" => Some(AbsenceValue {
            known_states: STATE_PRESENT | STATE_NULL,
            unknown_reasons: 0,
        }),
        "ok_or" | "ok_or_else" | "parse" => Some(AbsenceValue {
            known_states: STATE_PRESENT | STATE_UNDEFINED,
            unknown_reasons: 0,
        }),
        "unwrap_or" | "unwrap_or_else" | "unwrap_or_default" | "is_some" | "is_none" | "is_ok"
        | "is_err" | "len" | "is_empty" | "clone" | "to_string" | "to_owned" | "as_str"
        | "as_ref" | "push" | "insert" | "contains" | "iter" | "collect" => {
            Some(AbsenceValue::present())
        }
        _ => None,
    }
}
