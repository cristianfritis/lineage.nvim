## Why

`v0.1.0` shipped `lineage.nvim` with full support for TypeScript/TSX/JavaScript, Python, and Rust, and already validated Go's `(val, err)` relational narrowing and `defer` semantics against the frozen `CoreIR` in `engine/tests/ir_go_validation.rs`. However, `lineage-engine` does not yet include a Go `LanguageAdapter`, leaving `.go` files unsupported in the editor, standalone CLI, and `dogfood_gate`.

## What Changes

- **Go Tier-2 Adapter (`engine/src/adapters/go/`):**
  - Add `tree-sitter-go` (`0.23`) to `engine/Cargo.toml` and register `GoAdapter` in `default_adapters()` (`engine/src/main.rs`), `dogfood_gate.rs`, and `tests/golden_nullability.rs`.
  - Implement `GoAdapter` (`mod.rs`, `lower.rs`, `project.rs`, `stdlib.rs`):
    - Skeleton parsing (`parse_skeleton`) for Go `package` declarations, `import` specs (single and grouped `import (...)` with aliases), top-level `function_declaration` and `method_declaration` (keyed as `ReceiverType::MethodName` / `MethodName`), and `parse_errors`.
    - Flow-sensitive Pruned SSI lowering (`lower_function(..., &FileSkeleton, ...)`) for Go zero-value initialization (`var p *T`, `var s []T`, `var m map[K]V`, `var fn func(...)`, `var ch chan T`, `var i interface{}` -> `Nil`/`Absent` vs value types -> `Present`), `:=` short var declarations, `(val, err)` multi-output call `Implies(StateIn(err, Nil), StateIn(val, Present))`, comma-ok map lookup/type-assertion `Implies(StateIn(ok, Present), StateIn(v, Present))`, `if err != nil { return ... }` / `if x == nil` / `if x != nil` narrowing, `switch` / type-switch branch merging, `defer` named-return conservative `Unknown` downgrade, goroutine/channel/interface-typed-nil explicit `Unknown` boundaries (`DynamicDispatch` / `ExternalUntyped`), and pointer-dereference / slice-index / method-call danger sinks.
    - Same-package sibling scanning (`*.go` in the same directory, excluding `*_test.go` when `exclude_tests` is true) and `go.mod` module-root resolution (`GoModCache` backed by `moka::sync::Cache::new(64)`).
    - Built-in Go stdlib summary table (`os.Getenv`, `os.LookupEnv`, `os.Open`, `os.ReadFile`, `json.Unmarshal`, `errors.New`, `fmt.Errorf`, `strings.Cut`, `regexp.Compile`, `context.WithTimeout`, etc.).
    - Test-file classification (`*_test.go` and `/testdata/`).
- **Multi-Output Call & Same-Package Resolution in `SlicerEngine` (`engine/src/slicer/mod.rs`):**
  - Support same-package symbol lookup when `CallTarget::Local` or `CallTarget::Imported` targets a Go sibling file in the same package directory (and wire `gopls` in `tests/dogfood_init.lua` for Layer 2 live LSP).
- **Golden Tests & Documentation:**
  - Add `engine/tests/golden_go.rs` (`>= 40` Go probes), extend `engine/tests/golden_nullability.rs` with `>= 10` Go probes (`0 / N` false-safe `NonNull`), update `README.md` / `doc/lineage.txt` to list Go support, and update `benchmarks/corpus_manifest.json` if appropriate.

## Capabilities

### New Capabilities
- `go-adapter`: Go Tier-2 `LanguageAdapter` with `tree-sitter-go`, `go.mod` + same-package `*.go` sibling resolution, `(val, err)` and comma-ok `Implies` narrowing, zero-value initialization, `defer` named-return downgrade, stdlib summaries, and `*_test.go` classification.

### Modified Capabilities
- `dogfooding-verification-harness`: Extend `golden_nullability.rs` (`>= 10` Go probes) and `dogfood_init.lua` (`gopls` startup rule) to cover Go alongside TS, Python, and Rust.
- `release-pipeline-and-docs`: Document Go language support and `gopls` in `README.md` and `doc/lineage.txt`.

## Impact

- **Rust Engine (`engine/`):** Modifies `Cargo.toml`, `Cargo.lock`, `src/adapters/mod.rs`, `src/main.rs`, `src/slicer/mod.rs`, `examples/dogfood_gate.rs`, `tests/golden_nullability.rs`; creates `src/adapters/go/{mod.rs,lower.rs,project.rs,stdlib.rs}` and `tests/golden_go.rs`.
- **Neovim & Docs:** Modifies `tests/dogfood_init.lua`, `README.md`, `doc/lineage.txt`.
