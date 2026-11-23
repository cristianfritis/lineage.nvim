## 1. Crate Dependencies & Adapter Registration

- [x] 1.1 Add `tree-sitter-python`, `tree-sitter-rust`, and `toml` to `engine/Cargo.toml` and verify offline/release compilation.
- [x] 1.2 Register `PythonAdapter` and `RustAdapter` in `engine/src/adapters/mod.rs` and `default_adapters()` in `engine/src/main.rs`.

## 2. Python Adapter (`engine/src/adapters/python/`)

- [x] 2.1 Implement `PythonAdapter::parse_skeleton` and `LanguageAdapter` trait wiring (`.py`, `.pyi`, `classify_test_file`, native `None` terminology).
- [x] 2.2 Implement Pruned SSI lowering (`lower.rs`): `None`, truthiness asymmetry, `is None` / `is not None`, `assert x is not None`, walrus `:=`, default `= None`, comprehensions, `raise` termination, shallow 1-level `obj.field`, and `ERROR`/`MISSING` recovery.
- [x] 2.3 Implement `.pyi` outer-wrapper signature extraction (`Optional[T]`, `T | None`), curated Python stdlib summary table (`dict.get`, `re.search`, `re.match`, `next`, etc.), and cached `pyproject.toml`/`src/` + relative import static resolver.

## 3. Rust Adapter (`engine/src/adapters/rust/`)

- [x] 3.1 Implement `RustAdapter::parse_skeleton` and `LanguageAdapter` trait wiring (`.rs`, `classify_test_file`, native `Option::None`/`Result::Err` terminology).
- [x] 3.2 Implement Pruned SSI lowering (`lower.rs`): `Option<T>`/`Result<T, E>` payload extraction (`if let`, `match`, `let ... else`) via `UnwrapVariant`, `?` early-return + survivor unwrap, `.unwrap()`/`.expect()` danger sinks, `&self`/`&T` vs `&mut` shallow mutation rules, `unsafe` raw pointer `Unknown` boundaries, and `ERROR`/`MISSING` recovery.
- [x] 3.3 Implement curated Rust stdlib combinator summaries (`.map`, `.and_then`, `.unwrap_or`, `.ok_or`, `HashMap::get`, `slice::get`) and cached `Cargo.toml` workspace/`mod`/`crate::`/`super::`/`self::` static resolver.

## 4. Go `(val, err)` Core IR Validation & Multi-Language Test Matrix

- [x] 4.1 Implement `engine/tests/ir_go_validation.rs` covering zero-value pointers/slices/maps, correlated `(val, err)` and comma-ok `Implies` narrowing, and `defer`-mutated named returns with zero Core IR modifications.
- [x] 4.2 Implement `engine/tests/golden_python.rs` (40–60 inline-cursor cases) and `engine/tests/golden_rust.rs` (40–60 inline-cursor cases).
- [x] 4.3 Extend `engine/tests/fuzz_broken_syntax.rs` to `.py` and `.rs` truncated/mutated inputs.
- [x] 4.4 Run `cargo test`, `cargo clippy --all-targets -- -D warnings`, headless Neovim tests, and a `django` Python corpus latency/RSS check.
