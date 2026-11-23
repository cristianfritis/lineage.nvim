use crate::ir::AbsenceValue;

/// Built-in Python stdlib return absence summaries.
/// Note: Python `None` maps to `STATE_NULL` (`AbsenceValue::maybe_null()`).
pub fn lookup_python_stdlib(symbol: &str) -> Option<AbsenceValue> {
    let tail = symbol.rsplit('.').next().unwrap_or(symbol);
    match symbol {
        "dict.get"
        | "os.environ.get"
        | "os.getenv"
        | "re.search"
        | "re.match"
        | "re.fullmatch"
        | "shutil.which"
        | "next"
        | "getattr" => Some(AbsenceValue::maybe_null()),
        "len"
        | "str"
        | "int"
        | "float"
        | "bool"
        | "list"
        | "dict"
        | "set"
        | "tuple"
        | "sorted"
        | "enumerate"
        | "zip"
        | "range"
        | "isinstance"
        | "issubclass"
        | "hasattr"
        | "print"
        | "open"
        | "repr"
        | "id"
        | "hash"
        | "any"
        | "all"
        | "sum"
        | "min"
        | "max"
        | "abs"
        | "round"
        | "json.loads"
        | "json.dumps"
        | "os.path.join"
        | "os.path.exists"
        | "os.path.dirname"
        | "os.path.basename"
        | "Path"
        | "re.compile"
        | "re.sub"
        | "re.findall" => Some(AbsenceValue::present()),
        _ => match tail {
            "get" | "search" | "match" | "fullmatch" | "which" => {
                Some(AbsenceValue::maybe_null())
            }
            "strip" | "lstrip" | "rstrip" | "split" | "rsplit" | "join" | "replace"
            | "lower" | "upper" | "startswith" | "endswith" | "encode" | "decode"
            | "format" | "append" | "extend" | "copy" | "keys" | "values" | "items"
            | "setdefault" | "add" | "update" | "clear" | "count" | "index" => {
                Some(AbsenceValue::present())
            }
            _ => None,
        },
    }
}
