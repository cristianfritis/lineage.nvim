# Consolidated EGM Review Report — `openspec/changes/lineage-v1-foundation/`

*   **Target:** `openspec/changes/lineage-v1-foundation/` (`FeatureRange`: `proposal.md`, `design.md`, `specs/*/spec.md`; `DetailedRange`: `tasks.md`)
*   **Context:** `docs/designs/lineage-engine-v1.md`, `concepts_brief.md`
*   **Execution Mode:** Uninterrupted Full Flow (Stages 5, 6, 7, 8, 9)
*   **Stage Reports:** [zoom_out_report.md](./zoom_out_report.md) · [comprehension_report.md](./comprehension_report.md) · [critic_report.md](./critic_report.md) · [intermediate_zoom_out_report.md](./intermediate_zoom_out_report.md) · [implementation_readiness_report.md](./implementation_readiness_report.md)

**Overall Verdict:** **REVISIONS REQUIRED BEFORE IMPLEMENTATION** — 7 Blockers, 12 Majors, and 7 Nits remain after cross-stage deduplication and Stage 8 simplification.

---

## 🔴 Priority 1: Blockers (Must Fix Before `/opsx:apply`)

### B1. Fix M0/M1 Dependency Inversion (`LanguageAdapter` Trait & Missing Rust Module Roots)
*   **Problem:** M0 builds `cli.rs`, `cache/`, and `slicer/` which depend on `LanguageAdapter`, but `engine/src/adapters/mod.rs` is not created until M1 Task 2.1. Additionally, `engine/src/lib.rs` (required for `engine/tests/*` integration tests), `protocol/mod.rs`, `ir/mod.rs`, and `text/mod.rs` are omitted, and file paths lack `Create`/`Modify` tags or use globs (`tests/test_*.lua`).
*   **Action:** Move `engine/src/adapters/mod.rs` + a minimal in-memory stub adapter into M0; add `engine/src/lib.rs`, all `mod.rs` files, `stylua.toml`, exact file paths for Tasks 2.2/2.7/2.8/4.2 and all test files, and `Create`/`Modify` annotations.

### B2. Lock the NDJSON Wire Protocol Contract & Reverse-RPC Control Flow
*   **Problem:** Neither `design.md` nor `tasks.md` defines the outer NDJSON message envelope (JSON-RPC 2.0 vs. custom tagged JSON + error schema), the complete RPC method catalog (`initialize`, `lineage/slice`, `lineage/status`, `lineage/debugDump`, `lineage/syncBuffer`, `lineage/closeBuffer`, `lineage/sliceDelta`, `lineage/lspResolveBatch`, `$/cancelRequest`, `shutdown`), `protocol_version` type/initial value, or concrete projection DTO fields. A naive reader also cannot tell from `FeatureRange` alone who queries Neovim's LSP or what `"c-<n>"` vs. `"e-<n>"` IDs mean.
*   **Action:** Add a concise wire-contract table and 1-paragraph Phase 2 reverse-RPC flow summary to `design.md`, and reference it from `tasks.md`. (Per Stage 8, drop `expansion_token` and `lineage/expand` from M0—re-slice against warm Tier-1 cache in M3 instead.)

### B3. Complete `engine/Cargo.toml` Dependency Manifest, MSRV & Build Toolchain Choices
*   **Problem:** Task 1.1 leaves MSRV unpinned and omits crates required by later tasks: `tracing-subscriber`, concurrency/channel crate (`oneshot::Sender` + worker pools: `tokio` vs. `crossbeam-channel`/`oneshot`), total host RAM lookup (`memory-stats` only reads process RSS), JSONC parser for `tsconfig.json` (comments/trailing commas fail under `serde_json`), `toml` parser for `pyproject.toml`/`Cargo.toml`, CLI parser (`clap` vs. manual), content-hashing crate, exact grammar crate names (`tree-sitter-typescript`, `tree-sitter-javascript`, `tree-sitter-python`, `tree-sitter-rust`), `tree-sitter >= 0.23` + `tree-sitter-language`, and `lsp-types` 0.95 vs. 0.97 URI type choice.
*   **Action:** Pin exact MSRV (`rust-version = "1.xx"`), concrete crate names/versions/features, and concurrency/manifest/CLI choices in Task 1.1.

### B4. Resolve `mimalloc` / `ts_set_allocator` / `musl` Linking & RSS Eviction Complexity
*   **Problem:** `tree-sitter` has no safe Rust wrapper for `ts_set_allocator` (`unsafe extern "C"` + `libmimalloc-sys` required before `Parser::new`), `mimalloc`'s `override` feature fails to link on static Linux `musl`, C cross-compilation requires `cargo-zigbuild`/`cross`/`musl-tools` (and `aarch64-unknown-linux-musl` is missing in Task 5.3), and the 85% OS RSS trigger risks spin-looping under `mimalloc` without `mi_collect(true)` + hysteresis.
*   **Action (Stage 8 Minimal Fix):** Defer OS RSS polling, `ts_set_allocator`, `mi_collect(true)`, and hysteresis out of M0–M1; rely on `FileIrCache`'s byte-weighted LRU cap + a bounded dirty-buffer map (e.g., max 32 buffers / 16 MB) + `1 MB`/`4096`-byte pre-parse guards. If keeping `mimalloc` for Linux `musl` release builds in M4, disable its `override` feature and specify `cargo-zigbuild` (for both `x86_64` and `aarch64`) plus macOS `arm64` re-signing (`codesign -s -`) after `strip = true`.

### B5. Resolve In-Repo `checksums.json` vs. Tag-Triggered `release.yml` Chicken-and-Egg
*   **Problem:** Task 5.1 requires `checksums.json` pinned inside the plugin git repo while Task 5.3 generates `checksums.json` inside `release.yml` on version tags—a tag cannot contain SHA-256 hashes of binaries built after the tag is pushed.
*   **Action:** Lock a two-stage release workflow in `design.md` and Task 5.3 (e.g., tag `engine-v<semver>` builds/uploads binaries -> commit `checksums.json` pointing to `engine-v<semver>` -> tag plugin release `v<semver>`), and drop the bespoke pure-LuaJIT `bit` SHA-256 fallback in favor of native OS CLIs (`sha256sum`, `shasum -a 256`, `Get-FileHash`) with `cargo build --release` fallback.

### B6. Add Mandatory `## Alternatives Considered` Section to OpenSpec `design.md`
*   **Problem:** Violates `feature-spec-standard.md` ("The Alternatives Rule"): `design.md` omits `## Alternatives Considered` (and Decisions 6–7 omit `- *Rationale:*`).
*   **Action (Stage 8 Minimal Fix):** Add a concise 4–5 bullet `## Alternatives Considered` section in `design.md` (Pure Lua vs. Rust sidecar, Salsa vs. custom LRU, Mutable CFG vs. Pruned SSI, LSP hover scraping / synthetic `didOpen` vs. disk-first + outer-wrapper stubs, `dlopen` vs. statically linked grammars) and add `- *Rationale:*` to Decisions 6–7.

### B7. Add Minimal Security, Path-Guard & Privacy Requirements
*   **Problem:** Violates `feature-spec-standard.md` ("Risks & Limits"): `design.md` lists zero security risks, and specs do not guard static manifest joins against `..` traversal or `/proc`/`/sys` reads, nor state log source-code hygiene.
*   **Action (Stage 8 Minimal Fix):** Reject non-regular files (`!meta.is_file()`) and `/proc`/`/sys` pseudo-filesystems; guard static manifest relative joins (`tsconfig.json` `paths`/`baseUrl`) against `..` escape outside workspace root (trusting URIs from the user's attached LSP so `pnpm`/Nix symlinks keep working); mandate source-free rotating logs (paths, symbol names, timings, state tags only); document supply-chain and path-guard risks in `design.md`.

---

## 🟡 Priority 2: Majors (Resolve Before Implementation)

### M1. Split Monolithic 5-Milestone Change at the M1 Empirical Gate (Scope Decision)
*   **Problem:** Bundling M0–M4 in one OpenSpec change specifies M2–M4 in full even though M1's `<= 25%` external generic `Unknown` gate can alter M2's LSP strategy. Nearly half the comprehension/critic findings concern M2–M4 details.
*   **Action:** Strongly consider scoping `lineage-v1-foundation` to **M0 + M1** (core IR/slicer, NDJSON transport, location-only LSP bridge, TypeScript adapter, Hover UI, observability, and the M1 measurement gate), moving M2–M4 into follow-on OpenSpec changes once M1 passes. Also drop premature M0 front-loading (`OverlayCache`, dual priority lanes, ripgrep crates, `focus_spans`/`tree_rows` projectors) until needed.

### M2. Remove Post-v1 YAGNI Leaks (`go.mod`, `OutParam`, `Exceptional` Edges) & Fix `Present/Nil` Bug
*   **Problem:** `go.mod` appears in `proposal.md`/`design.md` despite Go being post-v1; `OutParam` (C/C#) and `Exceptional` edges (contradicting Non-Goal on interprocedural exceptions) leaked into `core-ir-and-slicer/spec.md`; `core-ir-and-slicer/spec.md` L22 writes contradictory `StateIn(err, Present/Nil)`.
*   **Action:** Remove `go.mod` from v1 scope, simplify `Call` outputs to `Vec<ValueId>` and exit edges to `Normal | EarlyExit`, and fix L22 to `StateIn(err, Nil)`.

### M3. Fix Unresolved Numeric Ranges, Unmeasurable Gate Formula & Uncalibrated Claims
*   **Problem:** Normative `SHALL` clauses and tasks use ranges (`120–150ms`, `<= 200–500` files parsed, `2–4` parse workers), `"near-0% false positives"`, `"3–5 TypeScript repositories (zod, TypeScript, etc.)"`, past-tense `"Confirmed empirically during M1"` under Open Questions, unqualified `"under 50ms"` on cold 3,000-line hovers, and omit the M1 `<= 25%` numerator/denominator, EWMA `alpha`/clamp, circuit-breaker consecutive-failure threshold, Focus debounce, and Windows M4 prebuilt vs. Cargo-only decision.
*   **Action:** Lock single concrete defaults for every constant, define the exact M1 `<= 25%` metric formula (`unknown_external_generic_calls / total_external_generic_calls` over a named repo set) and false-positive ceiling (`<= 1%`), qualify `< 50ms` as `(p95)` warm/function-scoped, and resolve the Windows M4 decision.

### M4. Complete Missing Behavioral Specs (Hover Keymaps/Anchor, Closures/`await`, Danger Sinks, Outer-Wrapper Rule, Variable Pinning)
*   **Problem:** `gn`, `gp`, `<CR>` are undefined in Hover; Phase 2 `[k+1]` vs `[6]` numbering and "fixed-height" vs append conflict; `language-adapters/spec.md` L29–31 omits `THEN` clauses for closures and `await`; danger sinks per language and outer-wrapper rule (drop 1-step substitution per Stage 8) are undefined; Variable Pinning lacks a behavioral `SHALL` body and Scenario.
*   **Action:** Define `<CR>` (jump under cursor) and drop M3-only `gn`/`gp` from M1 Hover (or document explicitly); specify anchor-once (`NW`/`SW`) + `[k+1]..[9]` append + `(file_id, range)` Phase 1/2 deduplication; add concise conservative `THEN` rules for closures/`await`; list per-language danger sinks; drop 1-step generic substitution; add a Variable Pinning scenario (if M3 stays in this change).

### M5. Add Sidecar Crash Recovery, `b:changedtick` Reset, Lua `<= 1 MB` Guard & M1 Binary Discovery
*   **Problem:** Sidecar crash/restart leaves Lua's `b:changedtick` map out of sync with Rust's empty dirty-buffer table; opening a 5–20 MB dirty buffer freezes Lua on `vim.json.encode` before Rust's 1 MB cap runs; M1 `client.lua` launches `lineage-engine` before M4 `install.lua` exists.
*   **Action:** Add max-3-restarts cap, reset `b:changedtick` map + reject pending callbacks on exit, add a Lua `> 1 MB` pre-check before `lineage/syncBuffer`, and define M1 binary discovery precedence (`opts.binary_path` -> version-keyed `stdpath('data')` -> `engine/target/release/lineage-engine` / `PATH`).

### M6. Specify Neovim 0.10 vs. 0.11+ `lsp_compat.lua` Branching & Multi-Client LSP Policy
*   **Problem:** Calling `client:request(...)` (colon syntax) breaks on Neovim 0.10.0; multi-client buffers (`vtsls` + `eslint`) have no selection/fan-out policy.
*   **Action:** Specify `vim.fn.has("nvim-0.11") == 1` branching for `request`/`supports_method`/`cancel_request`/`make_position_params`, lock `vim.uv.spawn`, and select the first non-circuit-broken client supporting the requested method.

### M7. Bound Static Wildcard Barrels, Nearest-Ancestor Manifests, Disk `stat()` & Query-Scoped ASTs
*   **Problem:** Uncapped `export *` mega-barrels can blow `< 50ms` Phase 1; workspace-root-only manifest caching breaks monorepos; `core-ir-and-slicer/spec.md` omits `stat()` invalidation and `<module>` top-level lowering; dropping `Tree` after each function forces repeated re-parses during intra-file multi-hop slices.
*   **Action:** Cap wildcard `export *` fan-out in Phase 1 (max 2 barrel hops / 10 siblings / 10ms sub-budget -> defer to Phase 2 or `Unknown: barrel_fanout_cap`), key manifests by nearest-ancestor directory (basic `paths`/`baseUrl` + single-level relative `"extends"`), add `stat()` `(mtime, len)` check + synthetic `<module>` scope, and hold up to 2–4 `Tree`s in Tier-3 `SliceArena` dropped when the query ends.

### M8. Add Per-Milestone Verification Checkpoints & Restore Omitted Spec Tasks
*   **Problem:** M0, M2, and M3 have no verification checkpoint tasks; M1/M3 Neovim tests are deferred to M4; `tasks.md` omits `trace_id` propagation, `PolymorphicBoundary`, `vtsls` nested `item.data`, `classify_test_file`, `.js`/`.jsx` grammar dispatch, `stderr` ring buffer, `:Lineage update-engine` + mismatch prompt, and hover `Unverified` TS/JS rule; M2 Tasks 3.1–3.2 are overloaded compared to M1.
*   **Action:** Add verification checkpoints and headless Neovim tests at the end of each milestone, restore all omitted spec tasks, and split M2 3.1–3.2 to match M1 granularity.

### M9. Clean Up Spec Altitude Drops
*   **Problem:** `proposal.md` and `specs/*/spec.md` hardcode Rust crates, internal struct/trait names, Lua filenames, and thread formulas instead of behavioral requirements.
*   **Action:** Move crate/struct/filename details to `design.md`/`tasks.md` and keep `specs/*/spec.md` strictly behavioral.

---

## 🟢 Priority 3: Nits

*   **N1.** Make full canonical `graph` payload opt-in on `< 50ms` hover responses (UI consumes `hover_card` projection; CLI/debug-dump requests `graph`).
*   **N2.** Stamp Tier-2 `OverlayCache` (when added) with Tier-1 `function_content_hash` instead of line-range diffing and coordinate shifting.
*   **N3.** Clarify/remove unused `grep-searcher` crate; document public `require("lineage.install").install()` entrypoint; add Go `(val, err)` Scenario to `language-adapters/spec.md` and `< 350ms` Tree Panel target to `neovim-ui/spec.md`; align statically linked grammars capability mapping in `proposal.md`.
*   **N4.** Add brief component topology overview to `design.md`; explain in one line why `vim.fn.sha256()` is forbidden (NUL-byte truncation); define `"Data Flow From Here"` = `:Lineage from` / `gf`; replace `"Phase-0"` with `"M0"`.
*   **N5.** Tighten style: attach `(SourceNode | ...)` strictly to `FlowNode` in Task 1.3; replace `"40–60"` with `"at least 40"`; remove filler/marketing phrasing (`Dogfood`, `curated`, `zero-collision`, `coverage honesty`, `self-DoS`, `proactive`, `trivially`, `unobtrusive`, `safely`, `cleanly`, `actionable`, `strictly`, `Wire`, `proving`).

---

## Traceability Matrix

| Consolidated Item | Stage 5 (`zoom_out_report.md`) | Stage 6 (`comprehension_report.md`) | Stage 7 (`critic_report.md`) | Stage 8 (`intermediate_zoom_out_report.md`) | Stage 9 (`implementation_readiness_report.md`) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **B1** M0/M1 inversion & file manifest | — | — | — | — | Impl #1, #3; Codebase #5, M1; Style M3 |
| **B2** NDJSON wire contract & reverse-RPC | — | Comp #1 | Struct #4 | Zoom8 #3, #5 | Impl #2; Codebase #4 |
| **B3** `Cargo.toml` deps & MSRV | — | — | — | — | Impl #4; Codebase #1; Style N7 |
| **B4** `mimalloc`/`ts_set_allocator`/`musl`/RSS | Zoom5 #2 | — | Design #5 | Zoom8 #2 | Codebase #2, N2 |
| **B5** `checksums.json` CI loop & SHA-256 | Zoom5 #4 | Comp #12 | — | — | Impl #8; Codebase #3 |
| **B6** `## Alternatives Considered` in `design.md` | — | — | Design #1; Struct #1 | Zoom8 #7 | — |
| **B7** Minimal security, path-guards & logs | — | — | Design #2; Struct #2 | Zoom8 #1 | — |
| **M1** Split M0+M1 before empirical gate | Zoom5 #1, #2 | — | — | Zoom8 #6 | — |
| **M2** Remove YAGNI leaks & `Present/Nil` bug | Zoom5 #3 | — | Struct #3, #6; Style M3 | — | — |
| **M3** Ranges, M1 gate formula & Windows | — | Comp #10 | Design N8, N9; Style M1, M2, M5, M6 | — | Impl #6; Style M1, M2 |
| **M4** Hover keys, closures/`await`, sinks, pinning | — | Comp #2, #3, #5–9 | Design #7; Struct #5; Style M4 | Zoom8 #5, #7 | — |
| **M5** Crash recovery, `changedtick`, 1MB, M1 bin | — | — | Design #4 | Zoom8 #7 | Codebase M3; Impl #7 |
| **M6** Neovim 0.10/0.11 shim & multi-client LSP | — | — | Design #7 | Zoom8 #7 | Codebase M2 |
| **M7** Barrels, manifests, `stat()`, query ASTs | Cat5 #1, #3 | — | Design #6 | Zoom8 #4 | — |
| **M8** Milestone checkpoints & omitted tasks | — | — | — | — | Impl #5, #7; Codebase M4, M5; Style M4 |
| **M9** Spec altitude cleanup | Zoom5 #5 | — | — | Zoom8 #5 | — |
| **N1–N5** Payload opt-in, hash stamp, style nits | Zoom5 #6; Cat5 #4 | Comp #11–13 | Struct N7–10; Style N1–8 | — | Codebase N1–3; Impl N9–10; Style N5–10 |
