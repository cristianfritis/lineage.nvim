## MODIFIED Requirements

### Requirement: Committed Golden Nullability Soundness Suite (`0 / N` False-Safe `NonNull`)
`engine/` SHALL include a committed CI regression suite (`engine/tests/golden_nullability.rs`) containing `>= 50` idiomatic multi-pattern probes (`>= 10` each across TypeScript, TSX/JavaScript, Python, Rust, and Go) that asserts both `0 / N` unqualified false-safe `NonNull` verdicts (`status_badge == "NonNull"` with `unverified_count == 0` over any true `Nullable` or `Absent` origin) AND explicit positive expected verdicts (`Nullable`/`Absent`, `NonNull`, and `Unverified` controls) so returning `Unverified` for every probe cannot vacuously pass.

#### Scenario: Golden soundness suite verifies zero false-safe NonNull and positive controls across TS, JS, Python, Rust, and Go
- **WHEN** `cargo test` runs `engine/tests/golden_nullability.rs`
- **THEN** all `>= 50` probes pass their expected `Nullable`/`Absent`/`NonNull`/`Unverified` assertions with zero unqualified false-safe `NonNull` verdicts across TS, TSX/JS, Python, Rust, and Go

### Requirement: Headless Mock-LSP Two-Phase CI Test and Layer 2 Live LSP Sign-Off
The repository SHALL include a headless Neovim mock-LSP integration test (`tests/test_two_phase_lsp.lua`) verifying Phase 1-before-Phase 2 wire ordering, immutable `[1..k]` hover item numbering with appended `[k+1]`, preserved unicode symbol headers with updated `status_badge`, UTF-16 column conversion on multi-byte lines, `:Lineage from` caller delta delivery, non-hanging concurrent `ensure_started`, and cancellation of in-flight Phase 2 reverse-RPC queries without stale delta mutation or EWMA timeout pollution; alongside a minimal-config Layer 2 live LSP runner (`tests/dogfood_init.lua`) and sign-off checklist (`docs/verification/stage-a-checklist.md`) targeting `vtsls` or `ts_ls`, `pyright` or `ty`, `rust-analyzer`, and `gopls` (`Phase 2 p95 <= 250ms` over `>= 20` probes/LSP via client telemetry ring).

#### Scenario: Headless mock-LSP test guards two-phase hover, tree delta, and cancellation regressions in CI
- **WHEN** headless Neovim executes `tests/test_two_phase_lsp.lua`
- **THEN** Phase 1 renders before `lineage/sliceDelta` arrives, Phase 2 appends `[k+1]` without renumbering `[1..k]` and updates `status_badge`, UTF-16 positions align on multi-byte lines, `:Lineage from` receives callers via delta, concurrent `ensure_started` calls complete cleanly, and cancelling an in-flight query aborts its Phase 2 delta without mutating active windows or tripping the circuit breaker
