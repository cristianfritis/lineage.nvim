## ADDED Requirements

### Requirement: Statically Linked Rust Grammar and Algebraic Sum-Type Lowering
`lineage-engine` SHALL statically link `tree-sitter-rust` and register a `RustAdapter` supporting `.rs` files with two-granularity lazy parsing, `Option<T>` / `Result<T, E>` payload extraction (`if let`, `match`, `let ... else`), `?` early-return + survivor unwrap, `.unwrap()`/`.expect()` danger sinks, and deterministic `ERROR`/`MISSING` recovery.

#### Scenario: `if let Some(u)` and `let ... else` trace unwrapped payload provenance
- **WHEN** a Rust function extracts `u` via `if let Some(u) = find_user()` or `let Some(u) = find_user() else { return; }`
- **THEN** `u` links via `UnwrapVariant` to `find_user()` and narrows to `Present` inside the surviving scope

#### Scenario: `?` operator models early return and survivor unwrap
- **WHEN** a Rust function evaluates `let user = find_user()?;`
- **THEN** the `None`/`Err` path models an `EarlyExit` edge and `user` continues as `Present`

#### Scenario: `.unwrap()` and `.expect()` emit intraprocedural danger sinks
- **WHEN** `.unwrap()` or `.expect(...)` is invoked on a potentially `None`/`Err` value
- **THEN** `RustAdapter` records an intraprocedural `DangerSink` on that call site

### Requirement: Rust Mutability Preservation, Combinators, `unsafe` Boundaries, and `Cargo.toml` Static Resolution
`RustAdapter` SHALL preserve narrowed `obj.field` state across `&self`/`&T` read-only calls while downgrading `&mut` calls to `Unknown`, mark `unsafe` raw pointer dereferences `Unknown`, provide curated stdlib combinator summaries (`.map`, `.and_then`, `.unwrap_or`, `.ok_or`, `HashMap::get`, `slice::get`), and resolve `Cargo.toml` workspace crates, `mod`, `crate::`, `super::`, and `self::` statically.

#### Scenario: Read-only `&self` preserves field narrowing while `&mut` downgrades to `Unknown`
- **WHEN** `self.user` is narrowed to `Present` and followed by `self.inspect()` (`&self`) vs `self.reset()` (`&mut self`)
- **THEN** `self.inspect()` preserves `Present` while `self.reset()` downgrades `self.user` to `Unknown` (never `Absent`)

#### Scenario: Static `Cargo.toml` and `mod` resolution without `cargo metadata`
- **WHEN** a Rust file calls a function via `mod utils;` or `crate::auth::verify` inside a `Cargo.toml` workspace with no LSP attached
- **THEN** `RustAdapter` resolves the target `.rs` file statically in Phase 1 and caches `Cargo.toml` workspace metadata once per root
