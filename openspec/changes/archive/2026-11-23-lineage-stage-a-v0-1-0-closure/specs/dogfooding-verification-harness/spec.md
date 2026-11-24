## ADDED Requirements

### Requirement: Self-Hosted Layer 1 Persistent Dogfooding Harness and Release Gates
The repository SHALL provide a self-hosted Rust Layer 1 verification harness (`engine/examples/dogfood_gate.rs` invoked by `benchmarks/stage_a_gate.sh`, with `benchmarks/m1_gate.sh` delegating to it, over `benchmarks/corpus_manifest.json`) that reuses `LanguageAdapter::parse_skeleton` and `xxhash_rust::xxh3::xxh3_64` to extract seeded deterministic variable and call-site probes across 4 external commit-SHA-pinned real repositories (`zod`, `preactjs/preact`, `fastapi`, and `ripgrep`) plus in-tree `lineage-engine` (external clones cached in gitignored `.dogfood-cache/`), drives a persistent `lineage-engine serve` process over NDJSON stdio, samples external OS peak `VmHWM` and sustained `VmRSS` via `/proc/<pid>/status` on Linux (`sysinfo` on macOS), emits one JSON report plus a concise stdout summary, and enforces release gates `G1`–`G5b` (`0` panics/malformed frames, warm Phase 1 `p95 < 50ms`, warm initial Tree/`FromHere` `p95 < 350ms`, peak and sustained OS RSS `<= 300 MB` including a `--features mimalloc` check, and workspace-internal + stdlib `Unknown <= 25%` with `>= 100` calls/language floor while always printing the full `UnknownReason` breakdown including `ExternalUninstalled` third-party calls and raw total `Unknown`).

#### Scenario: Harness enforces latency, memory, and workspace/stdlib coverage gates
- **WHEN** the maintainer runs `./benchmarks/stage_a_gate.sh` against the commit-SHA-pinned corpus
- **THEN** `dogfood_gate` writes a JSON report and concise stdout summary with warm/cold Phase 1 p95, Tree/`FromHere` p95, external OS peak (`VmHWM`) and sustained (`VmRSS`) memory, workspace+stdlib `Unknown` rate, `ExternalUninstalled` third-party count/percentage, raw total `Unknown`, and full `UnknownReason` histogram, exiting non-zero if any release gate fails

### Requirement: Committed Golden Nullability Soundness Suite (`0 / N` False-Safe `NonNull`)
`engine/` SHALL include a committed CI regression suite (`engine/tests/golden_nullability.rs`) containing `>= 40` idiomatic multi-pattern probes (`>= 10` each across TypeScript, TSX/JavaScript, Python, and Rust) that asserts both `0 / N` unqualified false-safe `NonNull` verdicts (`status_badge == "NonNull"` with `unverified_count == 0` over any true `Nullable` or `Absent` origin) AND explicit positive expected verdicts (`Nullable`/`Absent`, `NonNull`, and `Unverified` controls) so returning `Unverified` for every probe cannot vacuously pass.

#### Scenario: Golden soundness suite verifies zero false-safe NonNull and positive controls in CI
- **WHEN** `cargo test` runs `engine/tests/golden_nullability.rs`
- **THEN** all `>= 40` probes pass their expected `Nullable`/`Absent`/`NonNull`/`Unverified` assertions with zero unqualified false-safe `NonNull` verdicts across TS, TSX/JS, Python, and Rust

### Requirement: Headless Mock-LSP Two-Phase CI Test and Layer 2 Live LSP Sign-Off
The repository SHALL include a headless Neovim mock-LSP integration test (`tests/test_two_phase_lsp.lua`) verifying Phase 1-before-Phase 2 wire ordering, immutable `[1..k]` hover item numbering with appended `[k+1]`, preserved unicode symbol headers with updated `status_badge`, UTF-16 column conversion on multi-byte lines, `:Lineage from` caller delta delivery, non-hanging concurrent `ensure_started`, and cancellation of in-flight Phase 2 reverse-RPC queries without stale delta mutation or EWMA timeout pollution; alongside a minimal-config Layer 2 live LSP runner (`tests/dogfood_init.lua`) and sign-off checklist (`docs/verification/stage-a-checklist.md`) targeting `vtsls` or `ts_ls`, `pyright` or `ty`, and `rust-analyzer` (`Phase 2 p95 <= 250ms` over `>= 20` probes/LSP via client telemetry ring).

#### Scenario: Headless mock-LSP test guards two-phase hover, tree delta, and cancellation regressions in CI
- **WHEN** headless Neovim executes `tests/test_two_phase_lsp.lua`
- **THEN** Phase 1 renders before `lineage/sliceDelta` arrives, Phase 2 appends `[k+1]` without renumbering `[1..k]` and updates `status_badge`, UTF-16 positions align on multi-byte lines, `:Lineage from` receives callers via delta, concurrent `ensure_started` calls complete cleanly, and cancelling an in-flight query aborts its Phase 2 delta without mutating active windows or tripping the circuit breaker
