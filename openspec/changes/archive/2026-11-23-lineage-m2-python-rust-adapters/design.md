## Context

M0+M1 delivered the Pruned SSI + bounded relational guard Core IR, 2-axis product absence lattice (`Unknown != Absent`), NDJSON stdio transport with reverse-RPC LSP batching, TypeScript/TSX/JS adapter, and Interactive Two-Phase Hover. The M1 real-repo measurement gate on `zod`, `trpc`, and `vite` (713 targets) validated `< 50ms` Phase 1 p95 (`3–14ms`), `<= 250ms` Phase 2 p95 (`17–27ms`), `24.05%` aggregate external generic `Unknown` rate (`<= 25%`), and `27.93 MB` peak OS RSS.

M2 extends `lineage-engine` to Python and Rust and validates Go `(val, err)` semantics against the existing Core IR without modifying the IR, slicer, or Neovim Lua UI.

## Goals / Non-Goals

**Goals:**
- Implement `PythonAdapter` (`.py`, `.pyi`) and `RustAdapter` (`.rs`) behind the existing `LanguageAdapter` trait.
- Statically link `tree-sitter-python` and `tree-sitter-rust` with zero dynamic `.so`/`.dylib` loading.
- Reuse the existing `AbsenceValue` bitmask (`STATE_PRESENT`, `STATE_NULL`/`None`/`Option::None`, `STATE_UNDEFINED`/`Result::Err`) and `UnwrapVariant` / `Implies` / `Pi` / `Phi` SSI primitives.
- Cache parsed `pyproject.toml` / `Cargo.toml` workspace metadata once per workspace root (failures included) and guard all path joins against `..` escape.
- Validate Go `(val, err)`, comma-ok, zero-value initialization, and `defer`-mutated named returns via `engine/tests/ir_go_validation.rs` with zero Core IR changes.

**Non-Goals:**
- No Go `LanguageAdapter` implementation in M2 (only Core IR specification tests; full Go adapter is post-v1).
- No Inline Focus Mode, Interactive Tree Panel, or standalone ripgrep caller discovery (deferred to M3).
- No cross-platform prebuilt release CI/packaging (deferred to M4).
- No Python metaclass/decorator magic, Rust procedural macro expansion, or `unsafe` pointer aliasing.

## Decisions

1. **Map Python `None` and Rust `Option::None` / `Result::Err` onto the existing `AbsenceValue` bitmask without changing `CoreIR`:**
   - Python `None` maps to `STATE_NULL` (rendered by the adapter's native terminology as `None`); `is None` / `is not None` and `assert x is not None` emit `Pi` narrowing guards; truthiness `if x:` narrows `then` to `Present` while `else` remains `MaybeAbsent` (`0`, `""`, `[]`, `False`).
   - Rust `Option::None` maps to `STATE_NULL` and `Result::Err` maps to `STATE_UNDEFINED` (or both checked via `is_maybe_absent()`), while `if let Some(u) = opt`, `match`, `let Some(u) = opt else { ... }`, and `expr?` emit `UnwrapVariant` + `ExitEdge::EarlyExit` so `u` traces back to `Some`/`Ok` producers and `.unwrap()`/`.expect()` emit `DangerSink`.
   - *Alternative rejected:* Adding language-specific enum variants to `CoreIR` or `AbsenceSummary` — rejected to keep the wire schema (`schema_version: 1`) and 5-stage slicer language-blind.

2. **Parse `pyproject.toml` and `Cargo.toml` with the `toml` crate and cache once per workspace root:**
   - Python resolves relative imports (`.`, `..`), `pyproject.toml` `[tool.pytest]`/`[project]` source roots, and `src/` layouts without invoking a Python interpreter.
   - Rust resolves `mod foo;` (`foo.rs` / `foo/mod.rs`), `crate::`, `super::`, `self::`, and `Cargo.toml` workspace member crates without shelling out to `cargo metadata` (which can stall for seconds on large workspaces).
   - *Alternative rejected:* Invoking `cargo metadata` or `python -c` subprocesses — rejected due to `< 50ms` Phase 1 latency budget and cold-start stalls.

3. **Keep `Rust` `&self` / `&T` vs `&mut` shallow mutation rules conservative:**
   - Passing `&self` or `&T` to a call preserves narrowed `obj.field` state; passing `&mut self` or `&mut T` downgrades narrowed `obj.field` to `Unknown` (never `Absent`). `unsafe` raw pointer dereferences emit `Unknown` (`dynamic_dispatch` / `external_untyped`).

## Risks / Trade-offs

- **[Risk] `Cargo.toml` workspace glob patterns (`members = ["crates/*"]`) without `cargo metadata`** → Mitigation: Support direct member paths and single-level `/*` globs via `std::fs::read_dir`, bounded and cached once per workspace root.
- **[Risk] Python dynamic imports / `importlib` or Rust `macro_rules!` / proc-macros** → Mitigation: Lower unresolvable dynamic constructs deterministically to `Unknown` (`dynamic_dispatch` / `external_untyped`), keeping sibling valid statements sliceable.
- **[Risk] Binary size growth from two additional statically linked Tree-sitter grammars** → Mitigation: OS demand-pages unused parse tables lazily; verify release binary size and steady-state RSS on `django` remain well within the `128–300 MB` ceiling.
