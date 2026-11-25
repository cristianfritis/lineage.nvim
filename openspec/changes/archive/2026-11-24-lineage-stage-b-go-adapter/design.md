# Stage B Design: Go Tier-2 Adapter (`tree-sitter-go`, `go.mod`, Package Scanning, and `(val, err)` Narrowing)

## Context

`lineage.nvim` v0.1.0 ships TypeScript/TSX/JavaScript, Python, and Rust adapters over a frozen `CoreIR` (`Pruned SSI` + `GuardFact` relational guard algebra) and NDJSON JSON-RPC (`PROTOCOL_VERSION = 1`, `SCHEMA_VERSION = 1`). Ahead of Stage B, `engine/tests/ir_go_validation.rs` already verified that Go's `(val, err)` correlated multi-output calls, comma-ok map lookups, zero-value initialization, and `defer`-mutated named returns map cleanly onto `CoreIR` (`GuardFact::Implies`, `GuardFact::StateIn`, `GuardFact::Unreachable`, `SsiOp::Call`, `AbsenceValue::unknown`) with zero `CoreIR` or wire-schema changes.

## Goals / Non-Goals

**Goals:**
- Implement `GoAdapter` (`engine/src/adapters/go/{mod.rs,lower.rs,project.rs,stdlib.rs}`) using `tree-sitter-go` (`0.23`), registered in `default_adapters()`, `dogfood_gate`, and `golden_nullability`.
- Support Go-specific nullability and absence semantics:
  - Zero-value initialization (`var p *T`, `[]T`, `map[K]V`, `func(...)`, `chan T`, `interface{}` / `any` -> `Absent` (`nil`); value types `int`, `string`, `bool`, `struct` -> `Present`).
  - `(val, err)` multi-output call lowering emitting `GuardFact::Implies(StateIn(err, Nil), StateIn(val, Present))` and comma-ok (`v, ok := m[k]` / `v, ok := x.(T)`) emitting `GuardFact::Implies(StateIn(ok, Present), StateIn(v, Present))`.
  - `if err != nil { return ... }` / `if x == nil` / `if x != nil` branch narrowing and `switch` / type-switch phi merging.
  - `defer` blocks mutating named return variables downgraded conservatively to `Unknown` (`DynamicDispatch`).
  - Goroutine (`go fn()`), channel receive (`<-ch`), interface dynamic dispatch, and typed-nil interface pitfalls tagged with explicit `Unknown` boundaries rather than false-safe `NonNull`.
  - Danger sinks for pointer dereference (`*p`), selector access on nilable pointer/interface (`p.Field`), and nilable slice/map indexing.
- Support Go module & package resolution:
  - `GoModCache` (`moka::sync::Cache::new(64)`) parsing `go.mod` (`module <path>`) with `safe_join` path-traversal guards.
  - Same-package sibling scanning (`*.go` in the same directory, respecting `exclude_tests` for `*_test.go`) so unexported/exported package-level functions in sibling `.go` files resolve in Phase 1 without requiring `gopls`.
- Add `engine/tests/golden_go.rs` (`>= 40` probes), `>= 10` Go probes in `engine/tests/golden_nullability.rs`, `gopls` startup rule in `tests/dogfood_init.lua`, and Go documentation in `README.md` / `doc/lineage.txt`.

**Non-Goals:**
- Stage C graph exporters (`Mermaid`, `DOT`, offline `HTML+SVG`) and bounded ASCII DAG buffer.
- Full Go type-checker / generics monomorphization or `go/packages` subprocess invocation (Phase 2 delegates cross-module resolution to `gopls` over existing reverse-RPC).
- Breaking changes to `PROTOCOL_VERSION`, `SCHEMA_VERSION`, or `CoreIR`.

## Decisions

1. **Same-Package Resolution via `LanguageAdapter` + `SlicerEngine` Sibling Lookup:**
   - Go splits a single package across multiple `.go` files in the same directory without explicit `import` statements between sibling files. Rather than modifying `CoreIR`, `GoAdapter::resolve_static_import` resolves both:
     - `go.mod` module-prefixed imports (`<module>/pkg/sub` -> `<mod_root>/pkg/sub/*.go`)
     - Same-package sibling calls via a directory scan helper in `go/project.rs` (capped at 32 sibling files per package directory to stay within the `< 50ms` Phase 1 budget), or `SlicerEngine` querying `adapter.resolve_same_package_symbol(...)` / `CallTarget::Local` fallback when a symbol is not in the current file's `FileSkeleton.functions`.
   - *Alternative rejected:* Parsing every `*.go` file in the package during `parse_skeleton` of a single file — slows cold single-file hover and bloats `FileIrCache`.

2. **Multi-Output `(val, err)` & Comma-Ok Lowering on Frozen `CoreIR`:**
   - For `val, err := fn(...)`, lower the call once and bind `val` (`AbsenceValue::maybe_null()`) and `err` (`AbsenceValue::maybe_null()`), emitting `GuardFact::Implies { antecedent_place: "err", antecedent_mask: STATE_NULL, consequent_place: "val", consequent_mask: STATE_PRESENT }` into the enclosing block's `guards` (matching `engine/tests/ir_go_validation.rs`).
   - For `if err != nil { return ... }`, emit `GuardFact::Unreachable` on the early-return block and `GuardFact::StateIn { place: "err", mask: STATE_NULL }` on the surviving continuation block, then emit `SsiOp::Pi` on subsequent uses of `val` so `GuardEnv::apply_to` narrows `val` to `Present`.
   - *Alternative rejected:* Adding a new `SsiOp::MultiAssign` variant — unnecessary since `ir_go_validation.rs` already proved `CoreIR` handles this without new ops.

3. **Conservative `Unknown` for `defer` Named Returns & Typed-Nil Interfaces:**
   - If a function has named return parameters and contains a `defer_statement` whose body assigns to any named return identifier, downgrade `return_absence` with `UnknownReason::DynamicDispatch` (never fabricating `NonNull` or `Absent`).
   - Goroutine spawns, channel receives, and non-concrete interface method calls emit `UnknownReason::DynamicDispatch`.

## Risks / Trade-offs

- **[Risk] Package directory scan on huge Go packages (e.g., 200+ `.go` files) exceeds 50ms Phase 1 budget** → Mitigation: Cap same-package sibling scan at 32 non-test `.go` files and check `start.elapsed() < 20ms`, deferring remaining unresolved calls to Phase 2 `gopls` (`definition`).
- **[Risk] `tree-sitter-go` `0.23` API or node-kind differences** → Mitigation: Pin `tree-sitter-go = "0.23"` alongside existing `0.23` grammars (`typescript`, `python`, `rust`) and validate against `golden_go.rs` + `golden_nullability.rs`.
