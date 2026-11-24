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
        // Known-present JS/TS built-in globals and collection/string/promise methods
        "Array.from"
        | "Array.isArray"
        | "Array.of"
        | "Object.keys"
        | "Object.values"
        | "Object.entries"
        | "Object.assign"
        | "Object.freeze"
        | "Object.create"
        | "Object.hasOwn"
        | "Object.fromEntries"
        | "Object.getPrototypeOf"
        | "Object.defineProperty"
        | "Promise.resolve"
        | "Promise.reject"
        | "Promise.all"
        | "Promise.allSettled"
        | "Promise.race"
        | "Promise.any"
        | "JSON.stringify"
        | "JSON.parse"
        | "String.fromCharCode"
        | "String.raw"
        | "Number.isNaN"
        | "Number.isFinite"
        | "Number.isInteger"
        | "Number.parseInt"
        | "Number.parseFloat"
        | "setTimeout"
        | "setInterval"
        | "clearTimeout"
        | "clearInterval"
        | "queueMicrotask"
        | "parseInt"
        | "parseFloat"
        | "isNaN"
        | "isFinite"
        | "encodeURIComponent"
        | "decodeURIComponent"
        | "Error"
        | "TypeError"
        | "RangeError" => Some(AbsenceValue::present()),
        _ if symbol.starts_with("Math.")
            || symbol.starts_with("console.")
            || symbol.starts_with("Reflect.") =>
        {
            Some(AbsenceValue::present())
        }
        _ => match tail {
            "find" | "get" | "pop" | "shift" => Some(AbsenceValue::maybe_undefined()),
            "exec" | "querySelector" => Some(AbsenceValue::maybe_null()),
            "map" | "filter" | "reduce" | "forEach" | "some" | "every" | "includes" | "indexOf"
            | "lastIndexOf" | "join" | "slice" | "concat" | "flat" | "flatMap" | "push"
            | "unshift" | "splice" | "reverse" | "sort" | "keys" | "values" | "entries" | "has"
            | "set" | "add" | "delete" | "clear" | "trim" | "trimStart" | "trimEnd" | "split"
            | "replace" | "replaceAll" | "startsWith" | "endsWith" | "toLowerCase"
            | "toUpperCase" | "substring" | "padStart" | "padEnd" | "repeat" | "test"
            | "toString" | "valueOf" | "then" | "catch" | "finally" | "bind" | "call" | "apply"
            | "hasOwnProperty" => Some(AbsenceValue::present()),
            _ => None,
        },
    }
}
