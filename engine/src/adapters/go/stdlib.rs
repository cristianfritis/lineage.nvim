use crate::ir::{AbsenceValue, STATE_NULL, STATE_PRESENT};

/// Built-in Go standard library absence summaries.
pub fn lookup_go_stdlib(symbol: &str) -> Option<AbsenceValue> {
    match symbol {
        "nil" => Some(AbsenceValue::null()),
        "len" | "cap" | "make" | "new" | "append" | "copy" | "close" | "panic" | "recover"
        | "os.Getenv" | "strings.TrimSpace" | "strings.Split" | "strings.Join"
        | "strings.HasPrefix" | "strings.HasSuffix" | "strings.ReplaceAll" | "fmt.Sprintf"
        | "fmt.Errorf" | "errors.New" | "context.Background" | "context.TODO" | "json.Marshal" => {
            Some(AbsenceValue::present())
        }
        "os.LookupEnv"
        | "os.Open"
        | "os.ReadFile"
        | "os.Stat"
        | "os.Create"
        | "json.Unmarshal"
        | "strings.Cut"
        | "regexp.Compile"
        | "strconv.Atoi"
        | "strconv.ParseInt"
        | "context.WithTimeout"
        | "context.WithCancel"
        | "http.Get"
        | "http.NewRequest"
        | "errors.Is"
        | "errors.As" => Some(AbsenceValue {
            known_states: STATE_PRESENT | STATE_NULL,
            unknown_reasons: 0,
        }),
        _ => None,
    }
}
