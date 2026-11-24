# Stage A Design: v0.1.0 Closure, Two-Layer Verification, and Decoupled Release Pipeline

## Context

`lineage.nvim` is a Neovim (`>= 0.10`) plugin backed by a persistent Rust sidecar (`lineage-engine serve`) over NDJSON JSON-RPC (`PROTOCOL_VERSION = 1`, `SCHEMA_VERSION = 1`). It performs demand-driven, flow-sensitive nullability and absence slicing across TypeScript/TSX/JavaScript, Python, and Rust using a two-phase model: Phase 1 computes local and disk-backed static origins immediately, while Phase 2 queries attached Neovim LSP clients over reverse-RPC (`lineage/lspResolveBatch`) and streams refinements back via `lineage/sliceDelta`. Milestones M0–M4 merged the initial v1 feature set (including Go `(val, err)` IR validation tests ahead of a future Go adapter), but `v0.1.0` has not yet been verified on real multi-file repositories under live LSPs or published to a public GitHub repository.

A pre-release audit ([lineage-stage-a-v0-1-0-closure.md](../../../docs/designs/lineage-stage-a-v0-1-0-closure.md)) uncovered localized defects on `main` across four areas: (1) synchronous Phase 1/Phase 2 inversion dropping hover deltas and broken multi-client LSP circuit breaking, (2) silent false-safe `NonNull` fallbacks and Focus/Tree `Unknown -> [NULL]` mislabeling, (3) double-parsing in `lower_function` and an oversized 300 MB logical cache cap leaving zero OS RSS headroom, and (4) a single-tag `checksums.json` release deadlock plus a CI test coupled to live `"TBD"` digests. Stage A resolves these defects, enforces release gates `G1`–`G6` through a two-layer verification harness, and publishes verified `engine-v0.1.0` binaries on `cristianfritis/lineage.nvim` before tagging `v0.1.0`.

## Goals / Non-Goals

**Goals:**
- Remediate all four pre-dogfooding defect groups (async two-phase transport & LSP bridge, explicit `Unknown` soundness, memory/parsing calibration & observability, and decoupled release/installer contract) while preserving frozen `CoreIR` semantics and `PROTOCOL_VERSION = 1` / `SCHEMA_VERSION = 1`.
- Enforce six release gates (`G1`–`G6`) via a self-hosted Layer 1 Rust harness over 4 external commit-SHA-pinned repos (`zod`, `preactjs/preact`, `fastapi`, `ripgrep`) + in-tree `lineage-engine`, a committed `>= 40` probe golden soundness suite, a headless mock-LSP CI regression test (`tests/test_two_phase_lsp.lua`), and a minimal-config Layer 2 live LSP sign-off runner/checklist (`tests/dogfood_init.lua`, `docs/verification/stage-a-checklist.md`):

| Gate | Scope & Metric | Release-Blocking Threshold |
| :--- | :--- | :--- |
| **G1 (Stability)** | Engine panics, malformed JSON-RPC frames, unhandled Lua errors, stuck `ensure_started` waiters, leaked extmarks, or stale Neovim diagnostics | `0` |
| **G2 (Phase 1 Latency)** | Client-observed NDJSON round-trip and end-to-end engine `stats.phase1_ms` (including reduction and projections) on warm pass | `p95 < 50ms` (cold logged separately) |
| **G3 (Initial Tree / `FromHere` Latency)** | Client-observed NDJSON round-trip for `direction = "from"` / `"to"` with `include_tree = true` on warm pass | `p95 < 350ms` |
| **G4 (OS Memory Ceiling)** | External OS process peak (`VmHWM`) and post-sweep sustained (`VmRSS`) measured via `/proc/<pid>/status` on Linux (`sysinfo` on macOS) | Peak and sustained `<= 300 MB` |
| **G5a (Soundness)** | Committed `>= 40` probe golden suite (`>= 10`/language across TS, TSX/JS, Python, Rust) with positive `Nullable`/`Absent`/`NonNull` assertions | `0 / N` unqualified false-safe `NonNull` (`status_badge == "NonNull"` with `unverified_count == 0` over a nullable/absent origin) |
| **G5b (Workspace+Stdlib Coverage)** | `Unknown` rate on workspace-internal + stdlib calls (`>= 100` calls/language floor); uninstalled third-party (`ExternalUninstalled`) and raw totals always printed | `<= 25%` |
| **G6 (Layer 2 Live LSP)** | Minimal-config Neovim against `vtsls` or `ts_ls`, `pyright` or `ty`, and `rust-analyzer` after indexing warmup (`>= 20` probes/LSP via client telemetry ring) | `Phase 2 p95 <= 250ms` + UI/keymap checklist pass |

- Publish verified prebuilt binaries for all 4 release targets (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `aarch64-apple-darwin`, `x86_64-apple-darwin`) via decoupled `engine-v*` tags on public `cristianfritis/lineage.nvim`, synced into `checksums.json` via `scripts/sync_checksums.sh` before tagging `v0.1.0`.

**Non-Goals:**
- Stage B Go adapter (`tree-sitter-go`, package directory scan, `go.mod`) and Stage C graph exporters (`Mermaid`, `DOT`, offline `HTML+SVG`) / bounded ASCII DAG buffer.
- Inline terminal image previews (Kitty/Ghostty/tmux) or bundled rasterizers.
- Windows prebuilt binaries, Mason registry publishing, live LSP containers in GitHub Actions PR CI, or a GitHub Actions auto-commit bot for `checksums.json`.

## Decisions

1. **Dedicated Async Phase 2 Worker Pool & Inline Cancellation Wake-Up:**
   - Split slicing into Phase 1 (returns immediately and flushes its response onto stdout) and Phase 2 (dispatched via non-blocking `try_send` onto a dedicated bounded 2–4 thread worker pool isolated from Phase 1 workers, draining cleanly on shutdown before closing stdout, with an RAII drop guard releasing active-query cancellation state).
   - Store `query_id` inline in the pending reverse-RPC map (`(query_id, oneshot::Sender)`) so cancellation or shutdown drops matching senders directly (`Disconnected` is distinguished from `Timeout` so user cursor movement never trips EWMA/circuit breakers). Correlate reverse-RPC results back to `LspResolveKind::CallHierarchyIncoming` by index/key before projecting Phase 2 callers into `tree_rows`.
   - *Alternatives rejected:* (a) Client-side delta buffering in `hover.lua` blocks Phase 1 until Phase 2 LSP round-trips complete (up to the `250ms` Phase 2 budget); (b) sharing the Phase 1 request pool lets slow reverse-RPC waits starve `< 50ms` hovers; (c) a secondary `query_id -> Vec<RpcId>` index adds lock-hierarchy complexity unnecessary for a handful of concurrent reverse-RPCs.

2. **Explicit `Unknown` Soundness, Mode-Aware Budgets & Calibrated Memory Target:**
   - Replace silent `Present`/`NonNull` fallbacks (`bottom()`, missed cursor lookup, missing SSA values, unresolved local calls) with explicit `Unknown`/`Unverified` boundaries; fix Focus and Tree `is_absent` checks so pure `Unknown` nodes render `[?]` instead of `[NULL]`; populate 1-hop `param_flow_to_return` across all three lowerers; capture `phase1_ms` after reduction and projections; enforce mode-aware pipeline budgets (`50ms` hover / `300ms` tree and `from`); pass `&FileSkeleton` into `lower_function` across all 3 adapters; parse nested braced and glob Rust `use` declarations; add `UnknownReason::ExternalUninstalled` to `schema.rs` and `lattice.rs` and a single additive `SliceStats.uninstalled_external_calls` counter.
   - Calibrate Moka's logical weight cap to `clamp(3% RAM, 64 MB, 140 MB)` as an empirical target verified against external OS `VmHWM`/`VmRSS` in Layer 1 (including a `--features mimalloc` run), replace the three `Mutex<HashMap>` workspace config caches with `moka::sync::Cache::new(64)`, remove unused `SliceArena`, and expose current-PID `StatusResult.rss_bytes` via `sysinfo`.
   - *Alternatives rejected:* (a) A closed-loop `sysinfo` RSS eviction governor is premature before measuring static calibration and risks cache thrashing when allocators retain freed pages; (b) adding three narrow Gate 5b counters to `SliceStats` bloats every production hover payload; (c) bumping `SCHEMA_VERSION` to `2` breaks backward compatibility unnecessarily when additive defaulted fields suffice.

3. **Neovim Client Concurrency, Multi-Client LSP Isolation & Observability Boundary:**
   - Pass `(workspace_root, lsp_client_name)` as Rust-internal context into `SymbolResolver::resolve_batch` rather than serializing them on `LspResolveBatchParams`. Add only `circuit_open_clients` and `max_timeout_ms` to `LspResolveBatchParams` plus `lsp_timeout_ms` to `SliceParams`, forwarding `circuit_open_clients` into `lsp_compat.resolve_batch`, picking capability-supporting clients via `lsp_compat.pick_client`, and populating `client_name` on timed-out items so healthy secondary LSPs keep serving Phase 2.
   - Branch `vim.str_utfindex` on Neovim `0.10` vs `0.11+` and read unloaded file lines directly from disk without `bufload`. Serialize concurrent startup callers in `state.init_waiters` (drained on `initialize`, `stop()`, and early process exit), centralize `client.next_query_id()`, add a 100-entry bounded telemetry ring, preserve symbol prefixes on unicode hover headers and update `status_badge`/`is_trivial_peek` in `apply_delta`, clear prior buffer extmarks on re-focus/re-pin and cancel active queries in `focus.lua`, default engine `EnvFilter` to `info`, and glob `<log_path>*` in `:Lineage log` to tail `tracing_appender::rolling::daily`'s dated files.
   - *Alternatives rejected:* Keeping `workspace_root`/`lsp_client_name` on `LspResolveBatchParams` pollutes the Engine->Neovim wire with Rust-internal keys that Lua never reads; using `bufload` for UTF-16 conversion violates the zero-background-buffer invariant.

4. **Self-Hosted Layer 1 Harness & Permanent CI Regression Suites:**
   - Implement `engine/examples/dogfood_gate.rs` reusing `LanguageAdapter::parse_skeleton` and `xxhash_rust::xxh3::xxh3_64` against persistent `lineage-engine serve` over NDJSON stdio, parsing `/proc/<pid>/status` (`VmHWM`/`VmRSS`) on Linux (`sysinfo` on macOS), and emitting one JSON report + concise stdout summary (`benchmarks/m1_gate.sh` becomes a thin wrapper delegating to `benchmarks/stage_a_gate.sh`).
   - Enforce `G5a` via `engine/tests/golden_nullability.rs` (`>= 40` probes, `>= 10`/language with explicit `Nullable`/`Absent`/`NonNull`/`Unverified` assertions) and guard two-phase ordering, UTF-16 conversion, `:Lineage from` tree deltas, concurrent `ensure_started`, and cancellation races in `tests/test_two_phase_lsp.lua`.
   - *Alternatives rejected:* An external Python + `py-tree-sitter` driver risks grammar version drift against `engine/Cargo.toml`; a separate Markdown report generator duplicates the JSON + stdout output unnecessarily.

5. **Decoupled `engine-v*` / `v*` Public Release Pipeline & Single-Source `checksums.json`:**
   - Trigger `.github/workflows/release.yml` on `engine-v*` tags plus `workflow_dispatch` (`dry_run: boolean`, skipping tag-prefix parity on `workflow_dispatch`), build all 4 targets (`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` with `--features mimalloc`, plus `aarch64-apple-darwin` and `x86_64-apple-darwin`), run binary smoke checks (static-musl-safe `ldd`, arch inspection, and CLI checks on native runners), emit deterministic `checksums.json` (`version`, `engine_tag`, `repo: "cristianfritis/lineage.nvim"`, `targets`), and publish with `make_latest: false`.
   - Both `install.lua` and `client.lua` resolve binary paths from `checksums.json`'s `engine_tag` (`lineage-engine-<engine_tag>[.exe]`, legacy `v<version>` fallback). Add explicit connect/total timeouts to `curl`/`wget`, verify SHA-256 on `already_installed` release binaries (accepting an existing binary with an empty `<dest>.source_build` marker when `!opts.force`, and removing `<dest>.source_build` whenever a verified prebuilt binary is installed), match `:Lineage install!` (`sub == "install!"` plus `:Lineage! install` bang alias), and fall back to `cargo build --release` when `cargo` is executable on network/HTTP download failure (writing `<dest>.source_build`, while always aborting with an error on SHA-256 mismatch). The maintainer runs `scripts/sync_checksums.sh <engine-tag>` locally to validate and write `checksums.json` before committing to `main` and tagging `v0.1.0`; `ci.yml` triggers on `tags: ["v*"]` to enforce non-`"TBD"` digests.
   - *Alternatives rejected:* Single `v*` tags create a circular `checksums.json` deadlock for tag-pinned `lazy.nvim` users; custom `engine_source_hash` + JSON provenance sidecars spread hashing drift across 6 files and cause false `v*` failures when `engine/tests/` or `engine/examples/` change; a GitHub Actions auto-commit bot adds PAT/protected-branch complexity for a single-maintainer `v0.1.0` release; hard-failing on network errors blocks offline/airgapped users who have `cargo` installed.

## Risks / Trade-offs

- **[Risk] Async Phase 2 worker saturation or cancellation races** → Mitigation: Dedicated bounded Phase 2 pool with non-blocking `try_send`, RAII `finish_query` guard, inline `(query_id, tx)` abort in `pending_reverse`, and cancellation-race assertions in `tests/test_two_phase_lsp.lua`.
- **[Risk] Tightening `Unknown` fallbacks raises raw `Unknown` rate on bare clones** → Mitigation: Gate `G5b` enforces `<= 25%` on workspace+stdlib calls while reporting `UnknownReason::ExternalUninstalled` and raw totals separately, and Stage A fixes nested/glob Rust `use` parsing.
- **[Risk] Static 140 MB logical cache cap still approaches 300 MB real OS RSS on large sweeps** → Mitigation: Validated empirically via external `VmHWM`/`VmRSS` in Layer 1 (including `--features mimalloc`); if `G4` still fails, tune weigher constants or escalate per the structural triage rule.
- **[Risk] Structural defect discovered during dogfooding** → Mitigation: Any defect requiring `CoreIR` redesign, breaking `SliceEnvelope`/`PROTOCOL_VERSION` changes, or a new allocator/index architecture pauses Stage A into a separate blocking OpenSpec change; all gates `G1`–`G6` re-run before tagging `v0.1.0`.
- **[Risk] Cross-platform release failure on `ubuntu-24.04-arm` or macOS cross-compile** → Mitigation: Run `workflow_dispatch` (`dry_run = true`) on `cristianfritis/lineage.nvim` before pushing the immutable `engine-v0.1.0` tag; never force-move published `engine-v*` tags.

## Migration Plan

1. Implement sidecar hardening, Neovim client/LSP/installer updates, golden soundness suite, Layer 1 harness, and decoupled release workflows locally; pass all local Rust, Stylua, headless Neovim, and Layer 1/Layer 2 gates (`G1`–`G6`).
2. Push `main` to public repository `cristianfritis/lineage.nvim` and verify `ci.yml` passes across Neovim `v0.10.4` and `v0.11.0`.
3. Trigger `release.yml` via `workflow_dispatch` (`dry_run = true`) to verify all 4 target builds and smoke checks.
4. Tag and push `engine-v0.1.0`, run `scripts/sync_checksums.sh engine-v0.1.0`, commit updated `checksums.json` to `main`, confirm CI passes (including fixture-isolated `tests/test_install.lua`), and tag `v0.1.0`.

### Rollback

- **Initial `v0.1.0` launch:** Because no prior published `engine_tag` exists, rolling back a defective `v0.1.0` requires marking the release pre-release/yanking it and cutting a forward patch (`engine-v0.1.1` -> `v0.1.1`) for tag-pinned users (reverting `checksums.json` on `main` restores `"TBD"` placeholders and falls back to Cargo source builds).
- **Post-`v0.1.0` releases on `main`:** Revert the `checksums.json` commit on `main` to point back to the prior verified `engine_tag`; `:Lineage install` verifies SHA-256 and swaps atomically back after stopping the sidecar (tag-pinned users upgrade to a forward patch tag).
