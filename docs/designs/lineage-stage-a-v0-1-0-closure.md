# lineage.nvim Stage A: v0.1.0 Closure, Two-Layer Dogfooding Harness, and Decoupled Release Pipeline

*   **Author:** cristianfritis
*   **Status:** Proposed
*   **Date:** 2026-11-23

--------------------------------------------------------------------------------

## Overview

Milestones M0 through M4 delivered the v1 code surface of `lineage.nvim` across TypeScript/TSX/JavaScript, Python, and Rust, including the `lineage-engine` Rust sidecar, hybrid LSP bridge, bounded ripgrep caller discovery, Two-Phase Hover, Inline Focus Mode, Interactive Tree Panel, binary installer, and documentation. However, `v0.1.0` has not yet been validated against real multi-file repositories under live LSP servers, nor published to a public GitHub repository with verified prebuilt binaries.

Stage A closes `v0.1.0` as an isolated release-hardening and verification milestone before opening Stage B (Go Tier-2 adapter) or Stage C (graph exporters and bounded ASCII DAG views). It fixes concrete pre-dogfooding defects already present on `main`, introduces a reproducible two-layer verification harness enforcing hard latency, memory, and soundness gates, and establishes a decoupled `engine-v*` / `v*` public release pipeline on `cristianfritis/lineage.nvim`.

## The Problem

Shipping `v0.1.0` directly from the current `main` branch would fail both end users and maintainers in four ways:

1. **Broken Two-Phase Behavior and False-Safe Defaults on `main`:**
   * `SlicerEngine::slice_query` executes Phase 2 LSP reverse-RPC synchronously and emits `lineage/sliceDelta` *before* returning the Phase 1 `lineage/slice` RPC response. Because `lua/lineage/ui/hover.lua` ignores deltas while `active.winid`/`active.card` are `nil`, Phase 2 updates are dropped while Phase 1 blocks behind LSP latency.
   * Several code paths silently fall back to `NonNull` (`AbsenceValue::bottom()` rendering as `"NonNull"`, unresolved `CallTarget::Local` returning `AbsenceValue::present()`, and missed `lookup_at_pos` falling back to `ValueId(0)`), violating the plugin's coverage-honesty invariant.
   * Multiple Lua/LSP bugs remain on `main`: unicode hover headers strip the symbol name, `apply_delta` leaves `status_badge` stale, `:Lineage from` with an attached LSP returns an empty tree, Phase 2 resolution never clears resolved `Unknown` boundaries, reverse-RPC sends UTF-8 columns verbatim to UTF-16 LSP clients, concurrent `ensure_started` calls hang on mismatched table keys, UI modules collide on global `query_id` counters, and the per-LSP circuit breaker checks `"neovim-lsp"` while recording under actual client names (`"vtsls"`, `"pyright"`, `"rust_analyzer"`).
2. **Unmeasured Real-World Latency, Memory, and Accuracy:**
   * `benchmarks/m1_gate.sh` only slices a 15-line synthetic TypeScript fixture, probes `:1:0` on external repos, spawns a cold process per slice instead of measuring persistent `lineage-engine serve`, and never measures OS RSS or p95 latency.
   * `stats.phase1_ms` is captured before the reduction pipeline and UI projections run, `lower_function` re-parses the full file in all three adapters, and `FileIrCache` allows up to `300 MB` of estimated logical weight alone—leaving zero headroom inside a `300 MB` total OS RSS ceiling.
   * Engine tracing defaults to `ERROR` when `RUST_LOG` is unset, while `:Lineage log` reads only `stderr` instead of the rotating log file, leaving operators without per-query diagnostics during dogfooding.
3. **Release Tag Deadlock and Split-Brain Binary Versioning:**
   * `.github/workflows/release.yml` builds binaries on `v*` tags and attaches `checksums.json` to the GitHub Release asset list rather than the tagged git commit. Because `lua/lineage/install.lua` reads `checksums.json` from the plugin git checkout, tagging `v0.1.0` with `"TBD"` hashes causes every tag-pinned `lazy.nvim` user to miss prebuilt binaries and fall back to `cargo build --release`.
   * Repository references still point to `lineage-nvim/lineage.nvim` instead of `cristianfritis/lineage.nvim`. `client.lua` resolves binary filenames from hardcoded `config.lua` (`version = "0.1.0"`) while `install.lua` reads `checksums.json` and skips SHA-256 verification whenever `dest` is already executable.
4. **CI Breakage When Real Checksums Land:**
   * `tests/test_install.lua` currently reads the live repository `checksums.json` expecting `"TBD"` to test the Cargo fallback branch. Committing real SHA-256 digests to `main` will immediately break `ci.yml`.

Stage A resolves these four gaps in strict sequence: remediate the localized blockers, pass both automated (Layer 1) and live-LSP (Layer 2) stop-the-line gates, and publish verified `engine-v0.1.0` binaries before cutting plugin tag `v0.1.0`.

## Goals

* **Pre-Dogfooding Remediation (Block 1):** Fix the localized sidecar, Lua UI/LSP, telemetry, memory-cap, and release-contract defects (`B1`–`B8`, `S1`–`S4`, `P1`–`P4`, `R1`–`R4`) without modifying `CoreIR` semantics or breaking `PROTOCOL_VERSION = 1` / `SCHEMA_VERSION = 1`.
* **Automated Layer 1 Verification Harness:** Replace `benchmarks/m1_gate.sh` with a self-hosted Rust harness inside `engine/` (reusing the compiled Tree-sitter grammars) driving a persistent `lineage-engine serve` NDJSON stdio process across 3–4 commit-SHA-pinned real repositories (`zod`, a non-trivial `.tsx`/`.js` codebase, `fastapi`, and `lineage-engine` + `ripgrep`) plus an untracked local clone cache, emitting machine-readable JSON and Markdown gate reports.
* **Hard Stop-the-Line Release Gates:** Pass all gates before tagging `v0.1.0`:
  * **G1 (Stability):** `0` engine panics, malformed JSON-RPC frames, unhandled Lua errors, stuck `ensure_started` waiters, leaked extmarks, or ghost `vim.diagnostic` entries.
  * **G2 (Phase 1 Latency):** Warm `p95 < 50ms` for both client-observed NDJSON round-trip and end-to-end engine `stats.phase1_ms` (including reduction and projections) over seeded 300–500 probes/repo (cold pass reported separately).
  * **G3 (Initial Tree / `FromHere` Latency):** Warm `p95 < 350ms` client-observed NDJSON round-trip for `direction = "from"` / `"to"` with `include_tree = true` over seeded 50–100 probes/repo.
  * **G4 (OS Memory Ceiling):** Peak (`VmHWM`) and post-sweep sustained (`VmRSS`) OS RSS `<= 300 MB` measured externally via `/proc/<pid>/status` or `ps` across single-workspace sessions and multi-repo sweeps.
  * **G5a (Zero False-Safe `NonNull`):** `0 / N` unqualified false-safe `NonNull` verdicts (`status_badge == "NonNull"` with `unverified_count == 0` on a true `Nullable`/`Absent` origin) on a committed in-repo golden suite (`36–48` real-idiom probes across TS/TSX/JS, Python, and Rust running in `cargo test`) plus pinned external-corpus spot checks.
  * **G5b (External-Call `Unknown` Rate):** `<= 25%` `Unknown` rate scoped to workspace-internal + stdlib calls (`>= 100` calls/language denominator floor), with the harness always printing the full `UnknownReason` breakdown (including uninstalled third-party calls and raw total `Unknown` count/percentage).
  * **G6 (Layer 2 Live Neovim + Real LSP):** Minimal-config (`nvim -u ...`) live verification against `vtsls` OR `ts_ls`, `pyright` OR `ty`, and `rust-analyzer` after indexing-idle warmup achieving Phase 2 `p95 <= 250ms` (`>= 20` probes/LSP via client telemetry ring) plus passing the append-only hover, keymap, Focus, and Tree checklist, backed permanently in CI by a headless mock-LSP integration test (`tests/test_two_phase_lsp.lua`).
* **Deterministic Triage Rule:** Localized fixes land in Stage A; any defect requiring `CoreIR` redesign, breaking `SliceEnvelope` / `PROTOCOL_VERSION` changes, or a new allocator/persistent-index architecture pauses Stage A into a separate blocking change, after which all gates re-run in full.
* **Decoupled Public Release Pipeline:** Migrate all references to public `cristianfritis/lineage.nvim`, make `checksums.json` (`engine_tag`, `version`, `repo`, `targets`, optional `engine_source_hash`) the single source of truth for both `install.lua` and `client.lua`, verify SHA-256 on `already_installed` release binaries while recording local provenance markers for Cargo source builds, harden `release.yml` on `engine-v*` (`make_latest: false`, `workflow_dispatch` `dry_run`, post-build smoke tests), decouple `tests/test_install.lua` from live `"TBD"` hashes, sync real digests via `scripts/sync_checksums.sh`, and cut verified tag `v0.1.0`.

## Non-Goals

* **Stage B (Go Tier-2 Adapter):** `tree-sitter-go`, package directory scanning, `go.mod` resolution, and Go lowering remain deferred until `v0.1.0` is tagged.
* **Stage C (Visualizations & Exporters):** Mermaid, DOT, offline interactive HTML+SVG exporters, and bounded ASCII/Unicode DAG buffers remain deferred.
* **Inline Terminal Image Previews:** Kitty/Ghostty/tmux image protocols and bundled rasterizers (`resvg`, mandatory `graphviz`) are excluded.
* **Windows Prebuilt Binaries & Mason Registry:** Windows continues using the `cargo build --release` fallback; Mason registry submission is deferred post-`v0.1.0`.
* **Live LSP Containers in GitHub Actions PR CI:** PR CI runs the headless mock-LSP two-phase test; real LSP server checks run during Stage A Layer 2 release sign-off.
* **Automated GitHub Actions PR Bot for `checksums.json`:** Checksum synchronization uses an explicit local maintainer script (`scripts/sync_checksums.sh`) rather than an auto-commit CI bot.

## Open Questions

* **Exact Pinned Commit SHAs & TSX/JS Corpus Repository:** Which exact non-trivial `.tsx`/`.js` repository (and exact commit SHAs for `zod`, that TSX/JS repo, `fastapi`, and `ripgrep`) should be locked into the Layer 1 corpus manifest during implementation?

## Critical User Journeys (CUJs)

### CUJ 1: First-Time `lazy.nvim` Installation on `tag = "v0.1.0"` or `main`

* **Given**: A developer adds `cristianfritis/lineage.nvim` to `lazy.nvim` (either tracking `main` or pinned to `tag = "v0.1.0"`) with `build = function() require("lineage.install").install() end` on Linux (`x86_64`/`aarch64`) or macOS (`arm64`/`x86_64`).
* **When**: `lazy.nvim` invokes `require("lineage.install").install()` (or the user runs `:Lineage install`).
* **Then**: `install.lua` reads `checksums.json`, constructs `https://github.com/cristianfritis/lineage.nvim/releases/download/engine-v0.1.0/<asset>`, verifies the downloaded file's SHA-256 against git-pinned `checksums.json`, stops any active sidecar, sets `0755`, and atomically renames the binary into `stdpath("data")/lineage/bin/lineage-engine-engine-v0.1.0`. `:checkhealth lineage` reports `OK` with the installed binary path, `engine_tag`, and passing sidecar ping.

### CUJ 2: Lua-Only Patch Upgrade (`v0.1.1`) Without Engine Re-Download or Stale Shadowing

* **Given**: A user already has `lineage-engine-engine-v0.1.0` installed (or previously built a local binary via Cargo fallback) and upgrades the plugin to a Lua-only patch `v0.1.1` whose `checksums.json` still pins `engine_tag = "engine-v0.1.0"`.
* **When**: The user runs `:Lineage install` or triggers a `:Lineage hover` query.
* **Then**: If the installed release binary matches the pinned SHA-256 in `checksums.json`, `install.lua` returns `already_installed` without network traffic and `client.find_binary()` resolves the exact `engine_tag` binary directly from `checksums.json`. If the file at the destination fails SHA-256 verification (for example, a stale or corrupted binary) or the user runs `:Lineage install!`, `install.lua` re-downloads and atomically replaces it (unless a local `source_build` provenance marker and `build_from_source = true` opt-in are configured).

### CUJ 3: Live Two-Phase Hover and Tree (`FromHere`) Under Real LSP in Neovim

* **Given**: A developer opens a TypeScript (`vtsls`/`ts_ls`), Python (`pyright`/`ty`), or Rust (`rust-analyzer`) project in Neovim after LSP indexing finishes and invokes `:Lineage hover` or `:Lineage from` on a symbol with cross-file calls.
* **When**: `lineage-engine serve` executes Phase 1 and then runs Phase 2 LSP resolution asynchronously.
* **Then**: Phase 1 renders the hover card in `< 50ms` (p95) with the symbol name preserved in the unicode verdict line and Phase 1 numbered items `[1]..[k]`. Within `<= 250ms` (p95), `lineage/sliceDelta` arrives *after* Phase 1 opened, appends Phase 2 items `[k+1]..[9]` without renumbering `[1]..[k]`, updates `status_badge` and `verdict_line`, prunes resolved `unknown_boundaries`, and never emits ghost diagnostics or hangs concurrent requests. `:Lineage from` displays callers in the Tree Panel even when an LSP is attached.

### CUJ 4: Maintainer Pre-Release Gate Sign-Off (Layer 1 + Layer 2)

* **Given**: Block 1 remediation tasks are merged locally and `cargo test` (including the 36–48 probe golden nullability suite and headless mock-LSP Neovim tests) is green.
* **When**: The maintainer runs the Layer 1 Rust harness against the commit-SHA-pinned corpus (`zod`, TSX/JS repo, `fastapi`, `lineage-engine` + `ripgrep`) and executes the Layer 2 minimal-config Neovim live LSP checklist.
* **Then**: The harness outputs a machine-readable JSON + Markdown report proving Gates G1–G5b pass (`0` panics, Phase 1 `p95 < 50ms`, Tree/FromHere `p95 < 350ms`, peak & sustained OS RSS `<= 300 MB`, `0 / N` false-safe `NonNull`, workspace+stdlib `Unknown <= 25%` with full `UnknownReason` breakdown), and the Layer 2 telemetry summary confirms Phase 2 `p95 <= 250ms` and UI/LSP sign-off. Any structural failure pauses Stage A into a separate blocking change.

### CUJ 5: Decoupled Maintainer Release (`engine-v0.1.0` -> `checksums.json` -> `v0.1.0`)

* **Given**: The repository is pushed publicly to `cristianfritis/lineage.nvim`, `ci.yml` is green, and a `workflow_dispatch` `dry_run` of `release.yml` passes all 4 target builds and smoke checks.
* **When**: The maintainer tags and pushes `engine-v0.1.0`, runs `scripts/sync_checksums.sh engine-v0.1.0` to update `checksums.json` with real SHA-256 digests and `engine_tag`, commits to `main`, waits for CI (including `test_install.lua` and pre-tag manifest validation) to pass, and tags `v0.1.0`.
* **Then**: GitHub displays `v0.1.0` as the user-facing release (with `engine-v0.1.0` marked `make_latest: false`), all 4 prebuilt binaries are downloadable without authentication, and `checksums.json` on both `main` and `tag = "v0.1.0"` matches the published artifacts byte-for-byte.

## Technical Plan (Architecture)

Stage A is structured into three tightly scoped subsystems that preserve the frozen v1 `CoreIR` semantics and wire contract (`PROTOCOL_VERSION = 1`, `SCHEMA_VERSION = 1`): (1) Block 1 pre-dogfooding hardening in `engine/` and `lua/lineage/`, (2) the two-layer verification harness and regression suites, and (3) the decoupled `engine-v*` / `v*` public release pipeline on `cristianfritis/lineage.nvim`.

### 1. Block 1 Pre-Dogfooding Hardening (Sidecar & Neovim Client)

#### A. Async Two-Phase Slice Execution (`B1`, `B3`, `B4`, `B8`)

Today `SlicerEngine::slice_query` runs Phase 2 LSP reverse-RPC synchronously and emits `lineage/sliceDelta` before `transport.rs` writes the Phase 1 `RpcResponse`. Stage A splits query execution into two distinct functions without changing any wire message schema:

* `slice_phase1(&self, params, trace_id, cancel) -> Result<(SliceEnvelope, Option<Phase2Plan>), RpcError>`
* `execute_phase2(&self, plan: Phase2Plan, cancel: &AtomicBool) -> Option<SliceEnvelope>`

```text
Neovim (hover/tree)                transport.rs (worker pool)           Phase 2 worker
       |                                      |                                |
       |-- lineage/slice (id=c-7, qid=12) --->|                                |
       |                                      |-- slice_phase1() (< 50ms)      |
       |<-- RpcResponse::ok(phase1_env) ------|                                |
       |  [opens float, renders [1..k]]       |-- spawn(Phase2Plan, cancel) -->|
       |                                      |                                |-- reverse-RPC: lineage/lspResolveBatch
       |<-- RpcRequest (id=e-3) -----------------------------------------------|
       |-- RpcResponse (id=e-3, locations) ----------------------------------->|
       |                                                                       |-- clear resolved Unknown boundaries
       |                                                                       |-- project hover_card + tree_rows
       |<-- RpcNotification: lineage/sliceDelta (qid=12) ----------------------|
       |  [appends [k+1..9], updates badge]                                    |-- finish_query(12)
```

Key behavioral fixes inside this flow:

1. **Ordering & Cancellation (`B1`):** `transport.rs` writes `RpcResponse::ok(id, phase1_val)` onto `out_tx` *before* handing `Phase2Plan` to a bounded Phase 2 worker thread/pool. `state.finish_query(qid)` moves to Phase 2 completion (or immediately if `Phase2Plan` is `None`) so `$/cancelRequest` cancels in-flight Phase 2 batches cleanly.
2. **Boundary Pruning (`B4`):** For each `LspResolveItem` resolved with concrete `.d.ts`/`.pyi`/source targets, Phase 2 recomputes `resolved_absence` without retaining the stale `UnknownReason::ExternalUntyped` bit for that call and removes resolved symbols from `updated_boundaries`.
3. **LSP-Attached `FromHere` Tree Rows (`B3`):** When `direction` is `From` or `Both` and `lsp_attached = true`, `execute_phase2` projects `CallHierarchyIncoming` results into `tree_rows` inside `lineage/sliceDelta` (and falls back to bounded `discover_callers` if the LSP returns empty/unsupported). `lua/lineage/ui/tree.lua` registers a `client.on_delta` listener, cancels prior `query_id`s on refresh/close, and forwards real `lsp_attached` state in `expand_row`.
4. **Per-LSP Circuit Breaker & Adaptive Timeout (`B8`):** `LspResolveBatchParams` (or `StdioLspProvider::resolve_batch`) keys EWMA and circuit-breaker checks by `(params.lsp_client_name, params.workspace_root)` instead of `("neovim-lsp", startup_cwd)`. `slicer/mod.rs` passes `timeout_ms: 0` (capped by configured `lsp.timeout_ms`) so `EwmaTracker::suggested_timeout_ms()` is active.

#### B. Soundness Defaults, Mode-Aware Budgets & Structured Logs (`S1`–`S4`, `P3`–`P4`)

* **Eliminate Silent `NonNull` Fallbacks (`S1`–`S3`):**
  * `projectors.rs`: `AbsenceValue::bottom()` (`known_states == 0 && unknown_reasons == 0`) maps to `"Unverified"`, never `"NonNull"`. `hover_card` explicitly preserves `unverified_count` and `truncation` so Gate 5a tests unqualified `NonNull` (`status_badge == "NonNull" && unverified_count == 0`).
  * `slicer/mod.rs`: If `lookup_at_pos` misses, emit `AbsenceValue::unknown(UnknownReason::ExternalUntyped)` (or an explicit unresolved-cursor boundary) and log a warning instead of defaulting to `ValueId(0)` / `AbsenceValue::present()`. Missing SSA values and unresolved `CallTarget::Local` (closures, unindexed methods) resolve to `UnknownReason::DynamicDispatch` rather than `AbsenceValue::present()`.
  * Audit 1-hop `param_flow_to_return` substitution (`S4`) for local callees so passing a `Nullable` argument into an identity/pass-through callee propagates `Nullable` instead of returning a static `Present` summary (downgrading unresolved multi-hop chains to `Unknown`).
* **Accurate Telemetry & Mode-Aware Pipeline Budget (`P3`–`P4`):**
  * Capture `stats.phase1_ms` immediately *after* `run_reduction_pipeline` and all projections (`hover_card`, `focus_spans`, `tree_rows`) finish.
  * Parameterize the reduction pipeline wall-clock budget by query mode: `50ms` for Phase 1 Hover (`include_tree == false`), `300ms` for initial `Tree`/`FromHere` queries (`include_tree == true` or `direction != To`).
  * Reconcile external-call counters across `Stdlib`, `Imported`, and `Dynamic`, and distinguish workspace-internal/stdlib `Unknown` calls from uninstalled third-party imports (`UnknownReason` breakdown) so Layer 1 reports both the gated ratio and raw totals.
  * Fix Rust `extract_use_decl` in `adapters/rust/lower.rs` to parse nested braced imports (`use std::{collections::{HashMap, HashSet}, io};`) and glob imports cleanly.
  * Default `EnvFilter` in `engine/src/main.rs` to `info` when `RUST_LOG` is unset, emit structured `tracing` events (`trace_id`, `query_id`, `method`, `uri`, `phase1_ms`, `phase2_ms`, `truncation`, `unknown_count`) in `transport.rs` and `slicer/mod.rs`, thread `trace_id` into `expand_node`, and update `:Lineage log` (`lua/lineage/debug.lua`) to tail the rotating `log_path` alongside the `stderr` ring buffer.

#### C. Memory Calibration & Single-Parse Lowering (`P1`–`P2`)

* **Single-Parse Lowering (`P1`):** Pass `&FileSkeleton` into `LanguageAdapter::lower_function(uri, source, fn_skel, file_skel)` across TypeScript, Python, and Rust adapters so `lower_function` reuses `file_skel.imports` directly instead of re-parsing the full file on every uncached function.
* **Bounded RSS Footprint (`P2`):**
  * Recalibrate `compute_cache_cap_bytes()` in `engine/src/cache/memory.rs` to `clamp(3% host RAM, 64 MB, 140 MB)` (roughly 45–50% of the 300 MB OS RSS ceiling) so logical IR weights plus Rust/Moka heap overhead, `DirtyBufferOverlay` (16 MB), and Tree-sitter parse buffers stay under `300 MB` real OS RSS.
  * Bound adapter config caches (`TsConfigCache`, `PyProjectCache`, `CargoWorkspaceCache`) to a fixed maximum entry cap (e.g., 64 workspace entries) and remove the dead `_arena = SliceArena::new()` binding in `slicer/mod.rs`.
  * Expose live process `rss_bytes: Option<u64>` (sampled via `sysinfo`, already in `Cargo.toml`) as an additive optional field on `StatusResult` for `:Lineage status` and `:checkhealth lineage`.

#### D. Neovim Client & UI Hardening (`B2`, `B5`, `B6`, `B7`)

* **`lua/lineage/ui/hover.lua` (`B2`):** Preserve `card.symbol .. ": "` when prepending unicode badges; update `status_badge` and `is_trivial_peek` inside `apply_delta`; clear delta listeners on error and after delta receipt.
* **`lua/lineage/lsp_compat.lua` (`B5`):** Convert engine UTF-8 byte columns to each client's `offset_encoding` (`utf-16` / `utf-32` / `utf-8`) using `vim.str_utfindex` (reading the target buffer/file line) before issuing `definition` / `typeDefinition` / `implementation` / `prepareCallHierarchy` requests.
* **`lua/lineage/client.lua` (`B6`, `B7`):** Replace `table.insert(state.pending, cb)` with a dedicated `state.init_waiters` array drained once `initialize` finishes; centralize `M.next_query_id()` used by `hover.lua`, `focus.lua`, and `tree.lua`; add a bounded in-memory telemetry ring (last 100 queries: `trace_id`, `query_id`, `method`, `phase1_ms`, `phase2_ms`, `rtt_ms`, `delta_received`, `parse_errors`, `restarts`) exposed via `M.get_telemetry()` and `:Lineage status`.

---

### 2. Two-Layer Verification Harness & Regression Suites

#### A. Layer 1 Self-Hosted Rust Harness (`engine/examples/dogfood_gate.rs` + `benchmarks/stage_a_gate.sh`)

Instead of adding an external Python/Tree-sitter runtime, the Layer 1 driver is a Rust example binary inside `engine/` that links the exact `tree-sitter-*`, `serde_json`, `ignore`, and `sysinfo` crates already in `engine/Cargo.toml` and communicates with `lineage-engine serve` strictly over its public NDJSON stdio wire protocol.

```text
benchmarks/corpus_manifest.json (pinned repo URLs + commit SHAs + quotas)
       |
       v
benchmarks/stage_a_gate.sh (shallow --depth 1 fetch into untracked .dogfood-cache/)
       |
       v
cargo run --release --example dogfood_gate -- --manifest ... --engine <bin>
       |
       +---> Tree-sitter probe extractor (TS / TSX / JS / Python / Rust)
       |        - deterministic seeded selection: 300-500 Phase 1 + 50-100 Tree/From probes/repo
       |        - records (uri, line, col, expected_symbol, probe_kind)
       |
       +---> Persistent child: `lineage-engine serve` (one session per workspace + multi-repo sweep)
       |        - warmup pass -> measured warm pass (cold pass logged separately)
       |        - measures client-observed RTT + engine stats.phase1_ms + symbol alignment
       |        - polls external OS RSS (`VmHWM` peak & sustained `VmRSS` via /proc/<pid>/status or sysinfo)
       |
       v
Gate Report (JSON + Markdown): G1..G5b PASS/FAIL + full UnknownReason histogram
```

* **Pinned Primary Corpus (3–4 targets):** `zod` (TS), one non-trivial `.tsx`/`.js` repository, `fastapi` (Python), and `lineage-engine` + `ripgrep` (Rust). Clones live in gitignored `.dogfood-cache/` so the plugin git repo never bloats.
* **Committed In-Repo Golden Soundness Suite (`engine/tests/golden_nullability.rs`):** `36–48` real-world-derived probes (9–12 each across TS, TSX/JS, Python, and Rust) running on every `cargo test` in CI, enforcing **0 unqualified false-safe `NonNull` verdicts** (`0 / N`). Supplemented by pinned spot probes in `benchmarks/corpus_manifest.json`.
* **External-Call `Unknown` Breakdown:** Gate 5b enforces `<= 25%` on workspace-internal + stdlib calls (requiring `>= 100` calls per language to avoid vacuous passes), while the report always prints (1) workspace+stdlib `Unknown` %, (2) uninstalled third-party `Unknown` count/%, (3) raw total `Unknown` count/%, and (4) full `UnknownReason` histogram.

#### B. Layer 2 Live Neovim + Real LSP Verification & Headless Mock-LSP CI Test

* **Permanent CI Protection (`tests/test_two_phase_lsp.lua`):** Headless Neovim integration test registering a mock LSP client (`offset_encoding = "utf-16"`) and verifying: (1) Phase 1 opens hover *before* `lineage/sliceDelta` arrives, (2) `[1..k]` indices stay unchanged and `[k+1]` appends cleanly, (3) unicode verdict preserves symbol name and updates `status_badge`, (4) UTF-16 conversion works on multi-byte lines, (5) `:Lineage from` receives callers via delta, and (6) concurrent `ensure_started` calls do not hang.
* **Live LSP Interactive Sign-Off (`tests/dogfood_init.lua` + `docs/verification/stage-a-checklist.md`):** Minimal-config Neovim runner isolating the session from personal plugins. After an explicit indexing-idle warmup per workspace (`vtsls` OR `ts_ls`, `pyright` OR `ty`, `rust-analyzer`), the maintainer runs `>= 20` hover/tree/focus queries per LSP and dumps `require("lineage.client").get_telemetry()` to verify Phase 2 `p95 <= 250ms`, zero panics, zero leaked extmarks, and zero ghost `vim.diagnostic` entries.

---

### 3. Decoupled Public Release Pipeline (`engine-v*` -> `checksums.json` -> `v*`)

#### A. Single-Source `checksums.json` Schema & Binary Resolution

```json
{
  "version": "0.1.0",
  "engine_tag": "engine-v0.1.0",
  "repo": "cristianfritis/lineage.nvim",
  "engine_source_hash": "<sha256 over engine/{Cargo.toml,Cargo.lock,src/**}>",
  "targets": {
    "x86_64-unknown-linux-musl": {
      "asset": "lineage-engine-x86_64-unknown-linux-musl",
      "sha256": "<64-hex>"
    }
  }
}
```

```text
[1] Tag & push `engine-v0.1.0`
        |
        v
[.github/workflows/release.yml]
   - asserts engine/Cargo.toml == 0.1.0
   - builds 4 targets (linux musl x86_64/aarch64 + mimalloc, macOS arm64/x86_64)
   - runs smoke tests: `file`, static `ldd`, `lipo -archs`, `--version`, CLI `dump-ir`/`slice`
   - emits deterministic checksums.json (version, engine_tag, repo, engine_source_hash, targets)
   - publishes GitHub Release `engine-v0.1.0` with `make_latest: false` (skipped on dry_run)
        |
        v
[2] Maintainer runs `scripts/sync_checksums.sh engine-v0.1.0`
   - fetches release checksums.json, validates 4 64-hex hashes + repo + engine_source_hash
   - commits updated checksums.json to `main` -> PR/push CI passes (incl. test_install.lua)
        |
        v
[3] Tag & push plugin `v0.1.0` on `main`
   - pre-tag / v*-tag CI check confirms non-TBD hashes, matching repo & engine_source_hash
   - creates user-facing `v0.1.0` release (Latest)
```

* **Unified Lua Resolution (`install.lua` & `client.lua`):** Both modules call one shared helper (`install.resolve_pinned_binary()`) reading `engine_tag` (falling back to `"v" .. version` only for legacy manifests) from `checksums.json`, installing/discovering `stdpath("data")/lineage/bin/lineage-engine-<engine_tag>[.exe]`. `config.lua` demotes `version` to an explicit override (`nil` default) and adds `build_from_source = false`.
* **Verified `already_installed`, Network Cargo Fallback & Provenance Marker:**
  * Before returning `already_installed`, `install.lua` verifies the existing binary's SHA-256 against `checksums.json` (unless `build_from_source = true` and a matching sidecar `<dest>.provenance.json` with `{ "mode": "source_build", "engine_source_hash": "..." }` exists). A corrupted or mismatched binary triggers re-installation.
  * When a release download fails due to a **network/HTTP error** (`curl`/`wget` non-zero exit) and `cargo` is available, `install.lua` notifies the user, automatically falls back to `cargo build --release`, atomically installs the binary, and writes `<dest>.provenance.json` (`mode = "source_build"`).
  * When a downloaded release binary fails **SHA-256 verification**, `install.lua` always deletes the temp file, leaves `dest` untouched, and returns a hard error (never falls back on checksum mismatch).
  * `:Lineage install!` passes `{ force = true }` to bypass `already_installed`.
* **CI Isolation (`tests/test_install.lua` & `ci.yml`):**
  * Refactor `tests/test_install.lua` test #7 to use a synthetic temporary `checksums.json` fixture with `"TBD"` (never relying on live repo-root `checksums.json`). Add tests for `engine_tag` URL construction, SHA-verified `already_installed`, network-error Cargo fallback + `source_build` provenance marker, and `:Lineage install!`.
  * Add `cargo check --features mimalloc` and a `checksums.json` structural/pre-tag integrity check to `.github/workflows/ci.yml`.

## Alternatives Considered

### Alternative A: Combining Stage A (`v0.1.0` Closure), Stage B (Go Adapter), and Stage C (Exporters + ASCII DAG) in One Milestone

* **Pros**: Single planning cycle and earlier delivery of Go support and graph export commands.
* **Cons**: Mixes field validation of the existing v1 engine with two large new subsystems. Any defect discovered during real-repo dogfooding in `CoreIR`, `SliceEnvelope`, or the LSP bridge causes cascading rework across the new Go lowering and graph renderers.
* **Reason for Rejection**: Rejected by author: sequencing Stage A first validates and freezes v1 in real repositories before building Stages B and C on top of it.

### Alternative B: Inline Terminal Image Preview (Kitty / Ghostty / tmux) via Bundled Rasterizer or Mandatory Graphviz

* **Pros**: Displays rendered DAG graphs directly inside Neovim terminal windows.
* **Cons**: Requires either bundling heavy C/Rust rasterizers (`resvg`, `fontdb`) into `lineage-engine` or requiring external `graphviz`/`dot` binaries, plus handling terminal-specific graphics protocols, `tmux` passthrough quirks, and image cleanup on window scroll/close.
* **Reason for Rejection**: Rejected by author: poor complexity-to-value ratio compared to offline self-contained HTML+SVG opened via `vim.ui.open` (planned for Stage C).

### Alternative C: Single `v*` Release Tag Publishing Both Plugin Version and Engine Binaries

* **Pros**: Only one tag namespace (`v0.1.0`) in GitHub Releases.
* **Cons**: Creates a circular dependency: `release.yml` produces real SHA-256 hashes only after `v0.1.0` is pushed, so `checksums.json` inside the `v0.1.0` git commit remains `"TBD"`. Any `lazy.nvim` user pinning `tag = "v0.1.0"` misses prebuilt binaries and falls back to `cargo build --release`. Lua-only patches (`v0.1.1`) also force needless Rust rebuilds.
* **Reason for Rejection**: Rejected by author: decoupling `engine-v*` binary releases from plugin `v*` tags keeps verified SHA-256 hashes pinned inside git at `tag = "v0.1.0"` and avoids rebuilding Rust on Lua-only releases.

### Alternative D: External Python + `py-tree-sitter` Script for Layer 1 Probe Extraction

* **Pros**: Quick prototyping of probe selection and JSON reporting in Python.
* **Cons**: Introduces a second Tree-sitter toolchain and four external grammar wheels that can drift from the `tree-sitter 0.24` / grammar `0.23` crates linked in `engine/Cargo.toml`, adding setup friction on developer machines.
* **Reason for Rejection**: Rejected by author: a Rust example binary (`engine/examples/dogfood_gate.rs`) reuses the already-compiled Tree-sitter grammars, `ignore`, `serde_json`, and `sysinfo` with zero extra runtime dependencies and guaranteed AST parity.

### Alternative E: Client-Side Delta Buffering in `hover.lua` Instead of Async Phase 2 Transport Split (`B1`)

* **Pros**: Smaller Lua-only patch without touching `engine/src/protocol/transport.rs`.
* **Cons**: Leaves `SlicerEngine::slice_query` blocking Phase 1 behind the full `150–250ms` LSP reverse-RPC round-trip, violating the `< 50ms` Phase 1 latency gate whenever an LSP is attached.
* **Reason for Rejection**: Rejected by author: splitting `slice_phase1` from async `execute_phase2` solves both Phase 1 latency and dropped deltas at the source without breaking `SliceEnvelope`.

### Alternative F: Hard-Fail on Prebuilt Download Network Errors (Opt-In Cargo Build Only)

* **Pros**: Predictable failure mode on transient GitHub/DNS errors without triggering an unexpected 60–90s Rust compile.
* **Cons**: Users in offline or airgapped environments with `cargo` installed cannot recover via `:Lineage install` without manually configuring `build_from_source = true`.
* **Reason for Rejection**: Rejected by author: automatic fallback to `cargo build --release` on HTTP/network errors (recording a `source_build` provenance marker while still hard-failing on SHA-256 mismatch) preserves offline usability without weakening checksum integrity.

### Alternative G: Automated GitHub Actions Bot Committing `checksums.json` Back to `main`

* **Pros**: Eliminates one manual command (`scripts/sync_checksums.sh`) between pushing `engine-v*` and cutting `v*`.
* **Cons**: Requires configuring bot PAT tokens or custom workflow permissions to satisfy protected-branch rules and PR CI triggers for a single-maintainer `v0.1.0` release.
* **Reason for Rejection**: Rejected by author: a local `scripts/sync_checksums.sh <engine-tag>` script pulling and validating the `engine-v*` release manifest is simpler, explicit, and auditable before committing to `main`.

--------------------------------------------------------------------------------

## Non-Functional Requirements

* **Supply-Chain & Integrity:** Every downloaded prebuilt binary must match the 64-hex SHA-256 digest committed in `checksums.json` before `chmod 0755` and atomic `uv.fs_rename`. Checksum mismatches always delete `<dest>.tmp.<pid>`, leave `dest` untouched, and abort without falling back to Cargo. Existing release binaries are SHA-256 verified before `already_installed` returns; local Cargo builds carry an explicit `<dest>.provenance.json` (`source_build` + `engine_source_hash`) marker.
* **Performance & Memory Budgets:** Enforced by Layer 1 and Layer 2 gates: Phase 1 warm `p95 < 50ms`, initial Tree/`FromHere` warm `p95 < 350ms`, Phase 2 LSP-enriched `p95 <= 250ms`, and peak/sustained OS RSS `<= 300 MB` with Moka logical cap calibrated to `64–140 MB` and bounded adapter config caches.
* **Observability & Privacy:** Rotating logs remain source-free (URIs, symbol names, `trace_id`, `query_id`, timings, truncation/unknown counts—never raw source code). Default log level is `info` (overridable via `RUST_LOG`), and `:Lineage log` exposes both the rotating log tail and `stderr` ring buffer.
* **Rollout & Rollback Safety:**
  * Never force-move an immutable `engine-v*` tag once published; if a target fails or a post-tag defect is found before cutting `v0.1.0`, fix on `main` and cut a new tag (`engine-v0.1.0-rc.2` or `engine-v0.1.1`) while keeping `v0.1.0` blocked.
  * To roll back a broken `checksums.json` update on `main`, revert the `checksums.json` commit pointing back to the prior `engine_tag`; `:Lineage install` will verify SHA-256 and swap atomically back to the previous binary after stopping the sidecar.
  * Repository clone footprint stays small because `.dogfood-cache/` is gitignored and never committed to `main`.

--------------------------------------------------------------------------------

### Additional Resources

* [concepts_brief.md](../../concepts_brief.md) — Stage A 7-lens Goldfish brainstorm synthesis and locked decisions.
* [lineage-engine-v1.md](./lineage-engine-v1.md) — Frozen v1 architecture, CoreIR, and `SliceEnvelope` specification.
* [binary-installer spec](../../openspec/specs/binary-installer/spec.md), [observability-and-cli spec](../../openspec/specs/observability-and-cli/spec.md), and [release-pipeline-and-docs spec](../../openspec/specs/release-pipeline-and-docs/spec.md) — Current v1 capability specs.

### Future Improvements

* **Stage B (`v1.1` — Go Tier-2 Adapter):** Add `tree-sitter-go`, package-directory sibling scanning (`*.go`), `go.mod` resolution, `(val, err)` relational narrowing, and explicit `Unverified` downgrades for typed-nil interfaces, `defer`-mutated named returns, goroutines/channels, and deep struct embedding.
* **Stage C (`v1.2` — Graph Exporters & Bounded ASCII DAG):** Add offline Mermaid, DOT, and self-contained interactive HTML+SVG exporters (`vim.ui.open`) plus a bounded `<= 15–20` node ASCII/Unicode DAG buffer over `SliceEnvelope`.
* **Post-v1.2 Distribution & Power Workflows:** Windows `x86_64-pc-windows-msvc` prebuilt binaries, Mason registry publishing, Multi-Location Queue, and opt-in workspace null-risk diagnostics.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

### Target Files to Create/Modify

#### 1. `engine/src/protocol/schema.rs` (Modify)
* **Type / Scope**: Rust Wire Protocol Schema (`SCHEMA_VERSION = 1`, backward-compatible additive fields)
* **Proposed Changes**: Add optional `rss_bytes: Option<u64>` to `StatusResult`; add optional `workspace_root: Option<String>` and `lsp_client_name: Option<String>` to `LspResolveBatchParams`; add `UnknownReason::ExternalUninstalled` and additive optional counters on `SliceStats` (`workspace_stdlib_unknown_calls`, `workspace_stdlib_total_calls`, `uninstalled_external_calls`) so Layer 1 can report gated workspace+stdlib ratios alongside full raw totals without breaking existing JSON consumers.

#### 2. `engine/src/protocol/transport.rs` (Modify)
* **Type / Scope**: Rust NDJSON Stdio Transport & Concurrency Scheduler
* **Proposed Changes**: Update `handle_request("lineage/slice")` to call `slice_phase1(...)`, immediately emit `RpcResponse::ok(id, phase1_envelope)` on `out_tx`, and—if `Some(phase2_plan)` is returned and not cancelled—dispatch `execute_phase2` onto a bounded Phase 2 worker channel/pool that sends `lineage/sliceDelta` and calls `state.finish_query(qid)` on completion. Instrument `handle_request`, malformed JSON-RPC frame drops, and reverse-RPC timeouts with structured `tracing::info!` / `tracing::warn!` fields (`trace_id`, `query_id`, `method`, `uri`, `phase1_ms`, `phase2_ms`, `truncation`, `unknown_count`). Thread `trace_id` into `expand_node` and populate `StatusResult.rss_bytes` via `sysinfo`.

#### 3. `engine/src/slicer/mod.rs` (Modify)
* **Type / Scope**: Rust Slicing Engine Core
* **Proposed Changes**: Split `slice_query` into `slice_phase1` + `execute_phase2` (retaining a thin `slice_query` wrapper for CLI and tests). Move `phase1_ms` capture *after* `run_reduction_pipeline` and all projections. Make the pipeline wall-clock budget mode-aware (`50ms` for hover, `300ms` for initial `Tree`/`FromHere`). Remove dead `_arena = SliceArena::new()`. Replace silent `AbsenceValue::present()` fallbacks (`lookup_at_pos` miss, missing SSA value, unresolved `CallTarget::Local`) with honest `AbsenceValue::unknown(...)` + `UnknownBoundary`. Implement 1-hop `param_flow_to_return` argument substitution for local callees (downgrading unresolved multi-hop chains to `Unknown`). In `execute_phase2`, clear resolved `UnknownReason::ExternalUntyped` bits, prune resolved symbols from `updated_boundaries`, and project `CallHierarchyIncoming` callers into Phase 2 `tree_rows` (falling back to bounded `discover_callers` when LSP returns empty). Reconcile external-call counters across `Stdlib`, `Imported`, and `Dynamic`. Pass `params.lsp_client_name`, `params.workspace_root`, and `timeout_ms: 0` into `LspResolveBatchParams`. Accept `trace_id` in `expand_node`.

#### 4. `engine/src/slicer/projectors.rs` (Modify)
* **Type / Scope**: Rust UI Projection Layer
* **Proposed Changes**: Change the `AbsenceValue::bottom()` fallthrough in `project_hover_card` from `"NonNull"` to `"Unverified"` so empty or unmapped SSA states cannot produce false-safe `NonNull` badges.

#### 5. `engine/src/resolver/lsp_provider.rs` (Modify)
* **Type / Scope**: Rust Reverse-RPC LSP Provider & EWMA Circuit Breaker
* **Proposed Changes**: Key `EwmaTracker::check_circuit` and metric recording on `(params.lsp_client_name, params.workspace_root)` (falling back to defaults only if absent) so per-LSP circuit breakers actually trip and recover for real servers (`vtsls`, `pyright`, `ty`, `rust_analyzer`). Honor `timeout_ms == 0` via `ewma.suggested_timeout_ms()` clamped to the configured ceiling.

#### 6. `engine/src/cache/memory.rs` (Modify)
* **Type / Scope**: Rust Memory Budget & Cache Configuration
* **Proposed Changes**: Recalibrate `compute_cache_cap_bytes()` to `clamp(3% host RAM, 64 MB, 140 MB)` so Moka logical weights plus Rust/Tree-sitter/dirty-buffer overhead remain within the `300 MB` OS RSS ceiling. Remove dead `SliceArena` scaffolding if unused.

#### 7. `engine/src/adapters/mod.rs`, `engine/src/adapters/typescript/mod.rs`, `engine/src/adapters/python/mod.rs`, `engine/src/adapters/rust/mod.rs` (Modify)
* **Type / Scope**: Rust Language Adapter Trait & Implementations
* **Proposed Changes**: Add `&FileSkeleton` parameter to `LanguageAdapter::lower_function(...)` and update all three adapters to reuse `file_skel.imports` directly, eliminating the redundant full-file Tree-sitter re-parse on every uncached function.

#### 8. `engine/src/adapters/rust/lower.rs` (Modify)
* **Type / Scope**: Rust Adapter Import Lowering
* **Proposed Changes**: Upgrade `extract_use_decl` to parse nested braced imports (`use std::{collections::{HashMap, HashSet}, io};`) and glob imports without truncating on the first `{` or `,`.

#### 9. `engine/src/adapters/typescript/tsconfig.rs`, `engine/src/adapters/python/project.rs`, `engine/src/adapters/rust/project.rs` (Modify)
* **Type / Scope**: Rust Workspace Project Config Caches
* **Proposed Changes**: Bound each adapter's `Mutex<HashMap>` workspace cache to a maximum entry count (64 workspaces with eviction) so multi-repo sessions cannot grow memory unboundedly.

#### 10. `engine/src/main.rs` (Modify)
* **Type / Scope**: Rust Sidecar Entrypoint & Logging
* **Proposed Changes**: Initialize `EnvFilter` defaulting to `info` when `RUST_LOG` is unset so rotating logs capture per-query tracing events during dogfooding and troubleshooting.

#### 11. `lua/lineage/client.lua` (Modify)
* **Type / Scope**: Neovim Sidecar Transport & Telemetry Client
* **Proposed Changes**: Add dedicated `state.init_waiters` array drained when `initialize` succeeds/fails (fixing concurrent `ensure_started` hangs). Add centralized monotonic `M.next_query_id()`. Add a bounded 100-entry telemetry ring (`M.get_telemetry()`) recording `(trace_id, query_id, method, phase1_ms, phase2_ms, rtt_ms, delta_received, parse_errors, restarts)` and log JSON decode failures instead of swallowing them. Route `M.find_binary()` through `require("lineage.install").resolve_pinned_binary()` so binary discovery matches `checksums.json`'s `engine_tag` before falling back to local dev `target/release` and `PATH`.

#### 12. `lua/lineage/lsp_compat.lua` (Modify)
* **Type / Scope**: Neovim 0.10/0.11+ LSP Compatibility Bridge
* **Proposed Changes**: Convert UTF-8 byte columns received in `LspResolveItem` to each attached client's `offset_encoding` (`utf-16`, `utf-32`, `utf-8`) using `vim.str_utfindex` before dispatching `definition`, `typeDefinition`, `implementation`, and `prepareCallHierarchy` requests.

#### 13. `lua/lineage/ui/hover.lua` (Modify)
* **Type / Scope**: Neovim Two-Phase Hover UI
* **Proposed Changes**: Allocate `query_id` via `client.next_query_id()`. Preserve `card.symbol .. ": "` when rendering unicode badges. Update `status_badge` and `is_trivial_peek` inside `apply_delta`. Clear delta listeners on RPC error and after delta application.

#### 14. `lua/lineage/ui/focus.lua` (Modify)
* **Type / Scope**: Neovim Inline Focus Mode UI
* **Proposed Changes**: Allocate `query_id` via `client.next_query_id()` and ensure `clear()` removes `indicator_ns` extmarks from every visited buffer cleanly when pinned mode ends.

#### 15. `lua/lineage/ui/tree.lua` (Modify)
* **Type / Scope**: Neovim Interactive Tree Panel UI
* **Proposed Changes**: Allocate `query_id` via `client.next_query_id()`. Cancel active `state.query_id` on `refresh()` and `close()`. Register `client.on_delta` to update `tree_rows` when Phase 2 callers arrive for `:Lineage from`/`both`. Propagate actual attached LSP status instead of hardcoded `lsp_attached = false` in `expand_row`.

#### 16. `lua/lineage/debug.lua` & `lua/lineage/health.lua` (Modify)
* **Type / Scope**: Neovim Diagnostics & Health Checks
* **Proposed Changes**: Update `:Lineage log` to tail the rotating `log_path` returned by `lineage/status` alongside the `stderr` ring buffer. Display `rss_bytes`, `ewma_clients`, and client telemetry summary in `:Lineage status`. Update `:checkhealth lineage` to display resolved `engine_tag`, binary provenance (`release` vs `source_build`), and live `rss_bytes`.

#### 17. `lua/lineage/config.lua` (Modify)
* **Type / Scope**: Neovim Plugin Configuration
* **Proposed Changes**: Change default `version` to `nil` (explicit override only, deferring to `checksums.json`), add `build_from_source = false`, and ensure `lsp.timeout_ms` is forwarded on slice requests.

#### 18. `lua/lineage/install.lua` & `plugin/lineage.lua` (Modify)
* **Type / Scope**: Neovim Binary Installer & User Command Entrypoint
* **Proposed Changes**: Read and validate `engine_tag` (fallback `"v" .. version`) and optional `engine_source_hash` in `read_checksums()`. Expose shared `M.resolve_pinned_binary()` keying installed paths on `stdpath("data")/lineage/bin/lineage-engine-<engine_tag>[.exe]`. Build release URLs from `engine_tag`. Verify SHA-256 of existing release binaries before returning `already_installed`. Write `<dest>.provenance.json` (`{ "mode": "source_build", "engine_source_hash": "..." }`) on local Cargo builds and honor it when `build_from_source = true`. On HTTP/network download failure when `cargo` exists, notify the user and fall back automatically to `cargo build --release` (still hard-failing and deleting temp files on any SHA-256 mismatch). Wire `bang = true` in `plugin/lineage.lua` so `:Lineage install!` passes `{ force = true }`.

#### 19. `engine/examples/dogfood_gate.rs` (Create)
* **Type / Scope**: Layer 1 Self-Hosted Rust Verification Harness
* **Proposed Changes**: Implement an example binary linking the workspace's `tree-sitter-*`, `ignore`, `serde_json`, and `sysinfo` crates. Reads `benchmarks/corpus_manifest.json`, extracts seeded deterministic variable/call-site probes per language (TS, TSX, JS, Python, Rust), spawns persistent `lineage-engine serve` per workspace (plus a multi-repo sweep), runs warmup + measured passes, checks returned `hover_card.symbol` alignment, samples external OS `VmHWM`/`VmRSS`, enforces Gates `G1`–`G5b` (including workspace+stdlib `Unknown <= 25%` with `>= 100` calls/language floor and full `UnknownReason` + uninstalled third-party + raw total breakdown), and emits JSON + Markdown reports.

#### 20. `benchmarks/corpus_manifest.json` & `benchmarks/stage_a_gate.sh` (Create; replaces `benchmarks/m1_gate.sh`)
* **Type / Scope**: Layer 1 Corpus Pinning & Runner Wrapper
* **Proposed Changes**: Pin exact repository URLs, commit SHAs, probe budgets (`300–500` Phase 1 and `50–100` Tree/`FromHere` probes/repo), and external spot-check probes for `zod`, one real non-trivial `.tsx`/`.js` repo, `fastapi`, and `lineage-engine` + `ripgrep`. Provide `benchmarks/stage_a_gate.sh` to shallow-clone pinned commits into untracked `.dogfood-cache/` and execute `dogfood_gate`. Remove or delegate legacy `benchmarks/m1_gate.sh`.

#### 21. `engine/tests/golden_nullability.rs` (Create)
* **Type / Scope**: Rust Integration Test (Gate 5a Permanent CI Regression Suite)
* **Proposed Changes**: Add `36–48` real-idiom probes spanning TS, TSX/JS, Python, and Rust asserting `0 / N` unqualified false-safe `NonNull` verdicts (`status_badge == "NonNull" && unverified_count == 0` over any true `Nullable`/`Absent` origin).

#### 22. `tests/test_two_phase_lsp.lua` (Create)
* **Type / Scope**: Headless Neovim Mock-LSP Integration Test
* **Proposed Changes**: Register a mock LSP client (`offset_encoding = "utf-16"`) in headless Neovim and verify Phase 1 response ordering before `lineage/sliceDelta`, immutable `[1..k]` numbering with appended `[k+1]`, preserved unicode symbol header + updated `status_badge`, UTF-16 column translation on multi-byte lines, `:Lineage from` caller delta delivery into Tree Panel, and non-hanging concurrent `ensure_started`.

#### 23. `tests/test_install.lua` (Modify)
* **Type / Scope**: Headless Neovim Installer Unit Tests
* **Proposed Changes**: Update expected repo slug to `cristianfritis/lineage.nvim`; refactor test #7 (`"TBD"` Cargo fallback) to use a temporary `checksums.json` fixture decoupled from repo root; add tests for `engine_tag` URL routing, SHA-256 verification inside `already_installed`, automatic Cargo fallback on network failure with `<dest>.provenance.json`, hard-fail on SHA-256 mismatch, and `:Lineage install!`.

#### 24. `tests/dogfood_init.lua` & `docs/verification/stage-a-checklist.md` (Create)
* **Type / Scope**: Layer 2 Minimal-Config Live Neovim Runner & Sign-Off Template
* **Proposed Changes**: Provide a clean `nvim -u tests/dogfood_init.lua` bootstrap excluding personal plugins and a structured checklist recording Neovim/LSP versions (`vtsls`/`ts_ls`, `pyright`/`ty`, `rust-analyzer`), indexing-idle warmup verification, `>= 20`-probe telemetry ring `Phase 2 p95 <= 250ms` metrics, hover/focus/tree UX checks, and zero-ghost-diagnostic verification.

#### 25. `checksums.json` (Modify)
* **Type / Scope**: Pinned Release Manifest
* **Proposed Changes**: Set `"repo": "cristianfritis/lineage.nvim"`, add `"engine_tag": "engine-v0.1.0"` and `"engine_source_hash"`, and update all 4 target digests via `scripts/sync_checksums.sh engine-v0.1.0` prior to cutting `v0.1.0`.

#### 26. `.github/workflows/release.yml` (Modify)
* **Type / Scope**: Cross-Platform Binary Release Workflow
* **Proposed Changes**: Switch tag trigger to `engine-v*` and add `workflow_dispatch` with `dry_run: boolean`. Assert `engine/Cargo.toml` matches stripped `engine-v<semver>`. Add post-build smoke verification (`file`, static `ldd` on musl, `lipo -archs` on macOS, `--version`, and a CLI `dump-ir`/`slice` check on native runners). Generate deterministic `checksums.json` via Python/`jq` including `engine_tag` and `engine_source_hash`. Set `make_latest: false` on `softprops/action-gh-release@v2` and skip publishing when `dry_run == true`.

#### 27. `.github/workflows/ci.yml` (Modify)
* **Type / Scope**: Continuous Integration Workflow
* **Proposed Changes**: Add `cargo check --features mimalloc` to the `rust` job, ensure `golden_nullability` and `test_two_phase_lsp.lua` execute in CI, and add a manifest validation step verifying `checksums.json` schema and `repo == "cristianfritis/lineage.nvim"` on every PR (and rejecting `"TBD"` hashes or stale `engine_source_hash` on `v*` tag pushes).

#### 28. `scripts/sync_checksums.sh` (Create)
* **Type / Scope**: Maintainer Release Helper Script
* **Proposed Changes**: Fetch `checksums.json` from `https://github.com/cristianfritis/lineage.nvim/releases/download/<engine_tag>/checksums.json`, verify 4 valid 64-hex SHA-256 digests, matching `repo`, `engine_tag`, and local `engine_source_hash`, and write repo-root `checksums.json`.

#### 29. `.gitignore` (Modify)
* **Type / Scope**: Repository Hygiene
* **Proposed Changes**: Ignore `.dogfood-cache/` and `benchmarks/reports/` so temporary dogfooding clones and local gate reports are never committed.

#### 30. `README.md` & `doc/lineage.txt` (Modify)
* **Type / Scope**: User Documentation & Vim Help
* **Proposed Changes**: Replace all `lineage-nvim/lineage.nvim` references with `cristianfritis/lineage.nvim`; synchronize all `config.lua` defaults (`binary_path`, `version = nil`, `build_from_source = false`); document decoupled `engine-v*` / `v*` installation, `:Lineage install!`, automatic Cargo fallback on network failure with `source_build` provenance, and `v0.1.0` scope labels; verify `:helptags doc` passes cleanly.

### Verification Plan

* **Compilation/Build**:
  * `cd engine && cargo build --release`
  * `cd engine && cargo check --features mimalloc`
  * `cd engine && cargo build --release --example dogfood_gate`
* **Unit & Integration Tests (Rust)**:
  * `cd engine && cargo test` (all existing 31 tests + new `golden_nullability.rs` `36–48` probes enforcing `0 / N` false-safe `NonNull`)
* **Static Analysis & Linting**:
  * `cd engine && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
  * `stylua --check plugin lua tests`
  * `nvim --headless -c 'helptags doc' -c 'q'` (and clean temporary `doc/tags`)
* **Headless Neovim E2E**:
  * `for t in tests/test_*.lua; do nvim --headless -u tests/minimal_init.lua -l "$t"; done` (including new `tests/test_two_phase_lsp.lua` and refactored `tests/test_install.lua`)
* **Layer 1 Automated Dogfooding Gate**:
  * `./benchmarks/stage_a_gate.sh` passing `G1` (`0` panics), `G2` (Phase 1 warm `p95 < 50ms`), `G3` (Tree/`FromHere` warm `p95 < 350ms`), `G4` (peak & sustained OS RSS `<= 300 MB`), `G5a` (`0 / N` false-safe `NonNull`), and `G5b` (workspace+stdlib `Unknown <= 25%` with full `UnknownReason` breakdown).
* **Layer 2 Live Neovim + Real LSP Sign-Off**:
  * Run `nvim -u tests/dogfood_init.lua` across TS (`vtsls` OR `ts_ls`), Python (`pyright` OR `ty`), and Rust (`rust-analyzer`) following `docs/verification/stage-a-checklist.md`, verifying `Phase 2 p95 <= 250ms` (`>= 20` probes/LSP), append-only hover stability, Tree/Focus UX, and zero panics/ghost diagnostics.
* **Release Dry-Run & Tag Cut**:
  * Push publicly to `cristianfritis/lineage.nvim`, verify `ci.yml` green across Neovim `v0.10.4` and `v0.11.0`, execute `workflow_dispatch` `dry_run = true` on `release.yml` verifying all 4 target builds and smoke tests, then tag `engine-v0.1.0`, run `scripts/sync_checksums.sh engine-v0.1.0`, commit + pass CI on `main`, and cut `v0.1.0`.

--------------------------------------------------------------------------------

*Drafted with EGM*
