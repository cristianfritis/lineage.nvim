## 1. Go Grammar, Project Resolution, and Stdlib Summaries

- [x] 1.1 Add `tree-sitter-go = "0.23"` to `engine/Cargo.toml`, create `engine/src/adapters/go/{mod.rs,project.rs,stdlib.rs}` and register `pub mod go;` in `engine/src/adapters/mod.rs`.
- [x] 1.2 Implement `GoModCache` (`moka::sync::Cache::new(64)`) in `engine/src/adapters/go/project.rs` to parse `go.mod` (`module <path>`), resolve internal module-prefixed imports with `safe_join` traversal guards, and scan same-package sibling `*.go` files (bounded at 32 non-test files within 20ms).
- [x] 1.3 Implement Go stdlib summary table in `engine/src/adapters/go/stdlib.rs` (`os.Getenv`, `os.LookupEnv`, `os.Open`, `os.ReadFile`, `json.Unmarshal`, `strings.Cut`, `regexp.Compile`, `errors.New`, `fmt.Errorf`, `context.WithTimeout`, etc.) and `*_test.go` / `/testdata/` test-file classification in `GoAdapter::classify_test_file`.

## 2. Go Skeleton Parsing, Pruned SSI Lowering, and Slicer Integration

- [x] 2.1 Implement `parse_go_skeleton` in `engine/src/adapters/go/lower.rs` extracting `package` name, single and grouped `import (...)` specs (with aliases), top-level `function_declaration` and `method_declaration` (`ReceiverType::MethodName` / `MethodName`), and `parse_errors`.
- [x] 2.2 Implement `lower_go_function` in `engine/src/adapters/go/lower.rs`: zero-value initialization (`*T`, `[]T`, `map[K]V`, `func`, `chan`, `interface{}`/`any` -> `Absent` vs value types -> `Present`), `:=` short var declarations, `(val, err)` multi-output calls (`GuardFact::Implies(StateIn(err, Nil), StateIn(val, Present))`), comma-ok map/type-assertion lookups (`GuardFact::Implies(StateIn(ok, Present), StateIn(v, Present))`), `if err != nil { return }` / `if x == nil` / `if x != nil` narrowing, `switch` / type-switch merges, `defer` named-return conservative `Unknown` downgrade, goroutine/channel/dynamic-interface `Unknown` boundaries, and pointer-dereference / nilable selector / index danger sinks.
- [x] 2.3 Register `GoAdapter` in `default_adapters()` (`engine/src/main.rs`), `engine/examples/dogfood_gate.rs`, and `engine/tests/golden_nullability.rs`; wire same-package sibling lookup in `engine/src/slicer/mod.rs` so `CallTarget::Local` falls back to same-package `.go` files when a symbol is not in the current file.

## 3. Golden Tests, Dogfooding Harness, and Documentation

- [x] 3.1 Create `engine/tests/golden_go.rs` with `>= 40` idiomatic Go probes (zero-value pointers/slices/maps vs structs, `(val, err)` `Implies` narrowing, comma-ok map/type-assertion, `defer` named-return `Unverified`, `go.mod` + same-package sibling resolution, stdlib summaries, danger sinks, broken-syntax recovery) and extend `engine/tests/golden_nullability.rs` with `>= 10` Go probes (`0 / N` false-safe `NonNull`).
- [x] 3.2 Add `gopls` startup rule to `tests/dogfood_init.lua` and update `README.md` and `doc/lineage.txt` to document Go Tier-2 support and `gopls`.
- [x] 3.3 Run full verification: `cd engine && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo check --features mimalloc && cd .. && stylua --check plugin lua tests && for t in tests/test_*.lua; do nvim --headless -u tests/minimal_init.lua -l "$t" || exit 1; done`.
