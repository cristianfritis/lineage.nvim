## Purpose
Provide a Go Tier-2 `LanguageAdapter` (`GoAdapter` for `.go` files) backed by statically linked `tree-sitter-go`, `go.mod` module import resolution, same-package sibling scanning, `(val, err)` / comma-ok `GuardFact::Implies` narrowing, conservative `defer` named-return semantics, and built-in Go stdlib summaries.

## Requirements

### Requirement: Go `LanguageAdapter` with `tree-sitter-go` and Package-Level Resolution
`lineage-engine` SHALL provide a Go `LanguageAdapter` (`GoAdapter` supporting `.go` files) backed by statically linked `tree-sitter-go` (`0.23`), SHALL parse Go file skeletons (`package`, single/grouped `import` specs with aliases, top-level `function_declaration` and `method_declaration`, and `parse_errors`), SHALL resolve `go.mod` module imports via a bounded `moka::sync::Cache::new(64)` (`GoModCache`) with `safe_join` traversal guards, SHALL resolve same-package sibling symbols across `*.go` files in the same directory within Phase 1 caps, and SHALL classify `*_test.go` and `/testdata/` paths as test files.

#### Scenario: Same-package sibling function resolves without explicit import
- **WHEN** `handler.go` calls `findUser(id)` defined in sibling `store.go` in the same Go package directory with no LSP attached
- **THEN** `GoAdapter` locates `findUser` in `store.go`, lowers it, and propagates its `FnSummary` in Phase 1

#### Scenario: `go.mod` module import resolves internal package without LSP
- **WHEN** `cmd/server/main.go` imports `"example.com/app/internal/auth"` and `go.mod` declares `module example.com/app`
- **THEN** `GoAdapter` resolves the import to `<mod_root>/internal/auth` within workspace bounds

### Requirement: Go Nullability, `(val, err)` Relational Narrowing, and Conservative `defer` Semantics
`GoAdapter` SHALL lower Go zero-value declarations (`*T`, `[]T`, `map[K]V`, `func`, `chan`, `interface{}`/`any` as `Absent`, value types as `Present`), SHALL emit `GuardFact::Implies` for `(val, err)` multi-output calls and comma-ok map/type-assertion lookups so `if err != nil { return }` and `if ok { ... }` narrow `val` to `Present` in surviving continuations, SHALL downgrade `defer`-mutated named returns and goroutine/channel/dynamic-interface operations to explicit `Unknown` (`DynamicDispatch`), SHALL record pointer-dereference and nilable selector/index danger sinks, and SHALL provide built-in Go stdlib summaries (`os.Getenv`, `os.LookupEnv`, `os.Open`, `os.ReadFile`, `json.Unmarshal`, `strings.Cut`, `regexp.Compile`, `errors.New`, `fmt.Errorf`).

#### Scenario: `(val, err)` guard narrows `val` to `NonNull` after early return
- **WHEN** `val, err := fetchUser(id)` is followed by `if err != nil { return nil, err }` and `return val, nil`
- **THEN** `val` at the return site narrows to `NonNull` via `GuardFact::Implies` and `StateIn`

#### Scenario: `defer` mutating named return downgrades to `Unverified`
- **WHEN** a function with named return `(res *User)` mutates `res` inside a `defer func() { ... }()` block
- **THEN** `GoAdapter` emits an `Unknown` boundary (`dynamic_dispatch`) rather than reporting unqualified `NonNull`
