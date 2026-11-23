## Why

With M0+M1 complete and the TypeScript real-repo measurement gate validated (`<= 25%` aggregate external `Unknown` rate, `< 50ms` Phase 1 p95, `<= 250ms` Phase 2 p95, `< 30 MB` peak RSS), `lineage-engine` needs multi-paradigm language coverage before expanding the Neovim UI in M3. Adding Python and Rust adapters plus a Go `(val, err)` Core IR validation suite proves that the frozen Pruned SSI + bounded relational `Implies` IR generalizes across dynamic, nullable, and algebraic-sum-type languages without core rewrites.

## What Changes

- Statically link `tree-sitter-python` and `tree-sitter-rust` in `engine/Cargo.toml` and register both adapters in `default_adapters()`.
- Implement the **Python `LanguageAdapter`** (`engine/src/adapters/python/`): `None` provenance, truthiness asymmetry, `is None` / `is not None`, `assert x is not None`, walrus `:=`, default `= None`, comprehensions, `raise` termination, shallow 1-level `obj.field`, `.pyi` outer-wrapper signature extraction, curated stdlib summaries (`dict.get`, `re.search`, `re.match`, `next`, etc.), and `pyproject.toml`/`src` + relative import resolution.
- Implement the **Rust `LanguageAdapter`** (`engine/src/adapters/rust/`): `Option<T>` / `Result<T, E>` payload extraction (`if let`, `match`, `let ... else`) via `UnwrapVariant` edges, `?` early-return + survivor unwrap, `.unwrap()`/`.expect()` danger sinks, `&self`/`&T` read-only call preservation vs. `&mut` invalidation, stdlib combinators (`.map`, `.and_then`, `.unwrap_or`, `.ok_or`, `HashMap::get`, `slice::get`), `unsafe` raw pointers (`Unknown`), and `Cargo.toml` workspace/`mod`/`crate::` static resolution (caching metadata once per workspace root).
- Add **Go `(val, err)` Core IR specification tests** (`engine/tests/ir_go_validation.rs`) verifying zero-value pointers, `Implies`-correlated `(val, err)` and comma-ok guards, and `defer`-mutated named returns on the existing Core IR with zero core changes.
- Add golden test suites (`engine/tests/golden_python.rs`, `engine/tests/golden_rust.rs`, 40–60 cases each), extend `engine/tests/fuzz_broken_syntax.rs` to `.py` and `.rs`, and benchmark Python on the cloned `django` corpus.

## Capabilities

### New Capabilities
- `python-adapter`: Python `LanguageAdapter` with statically linked `tree-sitter-python`, `None`/truthiness/`is None`/`assert`/walrus/comprehension lowering, `.pyi` outer-wrapper extraction, stdlib summaries, and `pyproject.toml`/relative static import resolution.
- `rust-adapter`: Rust `LanguageAdapter` with statically linked `tree-sitter-rust`, `Option`/`Result` `UnwrapVariant` + `?` + `.unwrap()`/`.expect()` lowering, `&self`/`&T` vs `&mut` shallow mutation rules, stdlib combinators, `unsafe` raw pointer `Unknown` boundaries, and `Cargo.toml`/`mod`/`crate::` static resolution.
- `go-ir-validation`: Core IR validation test suite proving Go zero-value pointers, correlated `(val, err)` / comma-ok `Implies` guards, and `defer` named-return downgrades require zero Core IR changes.

### Modified Capabilities
<!-- None: existing Core IR, protocol, and TypeScript adapter requirements remain unchanged. -->

## Impact

- **Engine Dependencies:** Adds `tree-sitter-python`, `tree-sitter-rust`, and `toml` (for `pyproject.toml` / `Cargo.toml` parsing) to `engine/Cargo.toml`.
- **Engine Modules:** Adds `engine/src/adapters/python/` and `engine/src/adapters/rust/`; updates `engine/src/adapters/mod.rs` and `engine/src/main.rs` (`default_adapters()`).
- **Tests:** Adds `engine/tests/golden_python.rs`, `engine/tests/golden_rust.rs`, `engine/tests/ir_go_validation.rs`, and extends `engine/tests/fuzz_broken_syntax.rs`.
- **Neovim Lua UI:** No Lua changes required; `:Lineage hover` automatically supports `.py`, `.pyi`, and `.rs` buffers once `lineage-engine` registers the adapters.
