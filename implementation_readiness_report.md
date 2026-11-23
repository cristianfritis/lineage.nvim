# Stage 9: Implementation Readiness Report (`openspec/changes/lineage-v1-foundation/tasks.md`)

Reviewers: `egm-implementation-readiness` + `egm-codebase-details` + `egm-style`  
Standard: `implementation-readiness-standard.md`  
Context: `docs/designs/lineage-engine-v1.md`, `concepts_brief.md`

**Verdict: FAIL (Not Implementation-Ready — 6 Blockers, 9 Majors, 6 Nits)**

---

## Blockers

1. **[Blocker — `egm-implementation-readiness`] M0/M1 Dependency Ordering Inversion (`LanguageAdapter` Trait)**
   - **Affected Scope:** `tasks.md` (Tasks 1.2, 1.6, 1.9, 2.1)
   - **Rationale:** M0 implements `cli.rs`, `cache/`, and `slicer/` (which depend on `LanguageAdapter`), but `engine/src/adapters/mod.rs` is not created until M1 Task 2.1. Move `engine/src/adapters/mod.rs` + a minimal in-memory stub adapter into M0 so M0 compiles and passes tests independently.

2. **[Blocker — `egm-implementation-readiness`, `egm-codebase-details`] Unspecified NDJSON Wire Envelope, RPC Method Catalog, DTO Fields & `expansion_token` Handler**
   - **Affected Scope:** `tasks.md` (Tasks 1.3, 1.4, 1.8, 2.5, 4.4)
   - **Rationale:** Neither `tasks.md` nor `design.md` defines the outer NDJSON envelope (JSON-RPC 2.0 vs. custom tagged JSON + error schema), exact RPC method strings (`initialize`, `lineage/slice`, `lineage/expand`, `lineage/status`, `lineage/debugDump`, `lineage/syncBuffer`, `lineage/closeBuffer`, `lineage/sliceDelta`, `lineage/lspResolveBatch`, `$/cancelRequest`, `shutdown`), `protocol_version` type/initial value, concrete projection DTO fields, or the Rust-side `expansion_token` handler required by Task 4.4.

3. **[Blocker — `egm-implementation-readiness`, `egm-codebase-details`] Missing Required Rust Dependencies, Unpinned MSRV & ABI/Crate Incompatibilities**
   - **Affected Scope:** `tasks.md` (Tasks 1.1, 1.2, 1.4, 1.7, 2.2, 3.1, 3.2)
   - **Rationale:** Task 1.1 omits concrete MSRV (`rust-version = "1.xx"`), crate versions/features, and required crates:
     - `tracing-subscriber` (required to wire `tracing-appender::rolling`)
     - Concurrency/channel crate for `oneshot::Sender` + worker pools (`tokio` vs. `crossbeam-channel`/`oneshot`/`flume`)
     - Total host RAM lookup (`memory-stats` only reads process RSS, not total RAM for `clamp(5% RAM, 128 MB, 300 MB)`)
     - `libmimalloc-sys` (needed for raw C `mi_malloc`/`mi_calloc`/`mi_realloc`/`mi_free` symbols in `ts_set_allocator`)
     - JSONC parser for `tsconfig.json` (comments + trailing commas fail under `serde_json`) and `toml` parser for `pyproject.toml`/`Cargo.toml`
     - CLI parser (`clap` vs. manual) and content-hashing algorithm/crate
     - Exact grammar crate names (`tree-sitter-typescript`, `tree-sitter-javascript`, `tree-sitter-python`, `tree-sitter-rust`), `tree-sitter >= 0.23` + `tree-sitter-language`, and `lsp-types` 0.95 vs. 0.97 URI type choice.

4. **[Blocker — `egm-codebase-details`] `mimalloc` + `ts_set_allocator` FFI & Linux `musl` Cross-Compilation Trap**
   - **Affected Scope:** `tasks.md` (Tasks 1.1, 1.2, 5.3)
   - **Rationale:** `tree-sitter` has no safe Rust wrapper for `ts_set_allocator` (requires `unsafe extern "C"` before any `Parser::new`); `mimalloc`'s `override` feature fails to link on static Linux `musl` due to duplicate `malloc`/`free` symbols with `libc.a`; building C grammars + `libmimalloc-sys` for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (aarch64 omitted in Task 5.3) fails on `ubuntu-latest` without an explicit musl C cross-toolchain (`cargo-zigbuild` / `cross` / `musl-tools`).

5. **[Blocker — `egm-implementation-readiness`, `egm-codebase-details`] In-Repo `checksums.json` vs. Tag-Triggered `release.yml` Chicken-and-Egg Trap**
   - **Affected Scope:** `tasks.md` (Tasks 5.1, 5.3)
   - **Rationale:** Task 5.1 verifies downloads against `checksums.json` pinned inside the plugin git repo, while Task 5.3 generates `checksums.json` inside `release.yml` on version tags. A tag `v<semver>` cannot contain hashes of binaries built after the tag is pushed. Lock a concrete two-stage release workflow (`engine-v<semver>` builds/uploads binaries -> commit `checksums.json` -> tag plugin `v<semver>`).

6. **[Blocker — `egm-implementation-readiness`, `egm-codebase-details`] Incomplete Target File Boundary & Missing `engine/src/lib.rs` / Module Roots**
   - **Affected Scope:** `tasks.md` (Tasks 1.1, 1.3–1.6, 2.2, 2.3, 2.7, 2.8, 3.1, 3.2, 4.2, 5.2)
   - **Rationale:** Integration tests in `engine/tests/` cannot import items from `main.rs` without `engine/src/lib.rs` (omitted). Also missing `protocol/mod.rs`, `ir/mod.rs`, `text/mod.rs`, `stylua.toml`, exact file paths for Tasks 2.2, 2.7, 2.8, 4.2, and explicit test file names (replacing globs like `tests/test_*.lua`), plus `Create`/`Modify`/`Delete` annotations per `implementation-readiness-standard.md`.

---

## Majors

7. **[Major — `egm-implementation-readiness`, `egm-codebase-details`] Missing Per-Milestone Verification Checkpoints & Deferred Neovim Tests**
   - **Affected Scope:** `tasks.md` (M0, M1, M2, M3, M4)
   - **Rationale:** M0, M2, and M3 have no verification checkpoint tasks; M0 lacks unit tests for `protocol`, `ir`/`lattice`/`guards`, `cache`/`memory`, `resolver`, and `slicer`; headless Neovim tests for M1/M3 are deferred to M4 Task 5.2; `ripgrep.rs` tests and negative installer/checksum tests are missing.

8. **[Major — `egm-codebase-details`] Neovim 0.10 vs. 0.11+ LSP API Breakage Not Specified in `lsp_compat.lua`**
   - **Affected Scope:** `tasks.md` (Task 2.5)
   - **Rationale:** On Neovim 0.10.x, `client.request(...)` / `client.supports_method(...)` / `client.cancel_request(...)` are dot-calls (colon syntax breaks on 0.10.0), whereas 0.11+ deprecates dot calls and requires explicit `offset_encoding` in `make_position_params`. Specify `vim.fn.has("nvim-0.11") == 1` branching and lock `vim.uv.spawn` for the long-lived stdio sidecar.

9. **[Major — `egm-implementation-readiness`, `egm-codebase-details`] M1 Sidecar Binary Discovery Gap Before M4 `install.lua`**
   - **Affected Scope:** `tasks.md` (Tasks 2.4, 2.5, 5.1)
   - **Rationale:** Task 2.5 launches `lineage-engine serve` in M1 before `install.lua` exists in M4. Define binary resolution precedence (`opts.binary_path` -> version-keyed `stdpath('data')` -> local dev `engine/target/release/lineage-engine` / `PATH`).

10. **[Major — `egm-implementation-readiness`, `egm-codebase-details`, `egm-style`] Unspecified Numeric Constants, Unmeasurable M1 Criteria & Unresolved Windows M4 Decision**
    - **Affected Scope:** `tasks.md` (Tasks 1.7, 1.8, 2.8, 4.1, 4.3, 5.1, 5.3)
    - **Rationale:** Lock single defaults for `120–150ms` timeout and `<= 200–500` parsed cap; specify EWMA `alpha`, cold-start clamp, circuit-breaker consecutive-failure threshold, OS RSS polling interval, and Focus Mode debounce; replace `"near-0% false positives"` and `"3–5 TypeScript repositories (zod, TypeScript, etc.)"` with concrete thresholds/repo list; resolve Windows prebuilt vs. Cargo-only in Task 5.3.

11. **[Major — `egm-implementation-readiness`, `egm-codebase-details`] Spec Requirements Omitted from Actionable Tasks**
    - **Affected Scope:** `tasks.md` (Tasks 1.6–1.9, 2.1, 2.4–2.7, 4.2–4.3)
    - **Rationale:** Add explicit task steps for `trace_id` propagation, immediate `Tree` disposal + `(file_uri, symbol_id, function_content_hash)` keying + active-buffer eviction protection + 60% logical LRU cap + 2–4 parse workers, `PolymorphicBoundary` emission (>5 implementations), `vtsls` nested `item.data` handling, `classify_test_file`, `.js`/`.jsx` grammar dispatch, `stderr` ring buffer, `:Lineage update-engine` + mismatch prompt, hover anchor/guard/`Unverified` rules, and cross-buffer `:Lineage pin` / `gp`.

12. **[Major — `egm-style`] Overloaded M2 Tasks (3.1–3.2)**
    - **Affected Scope:** `tasks.md` (Tasks 3.1, 3.2)
    - **Rationale:** Each task bundles adapter lowering, stub extraction, stdlib table, project resolver, and 40–60 golden tests into one line (unlike M1's 2.1–2.3 split). Split 3.1 and 3.2 to match M1 granularity.

---

## Nits

13. **[Nit — `egm-codebase-details`, `egm-implementation-readiness`, `egm-style`]** Clarify/remove unused `grep-searcher`; note macOS `arm64` `strip = true` invalidates ad-hoc signature unless re-signed (`codesign -s -`); specify public `require("lineage.install").install()` entrypoint; attach `(SourceNode | ...)` strictly to `FlowNode` (not `FlowEdge`) in Task 1.3; replace `"Dogfood"`, `"curated"`, `"zero-collision"`, `"coverage honesty"`, `"proving"`, `"Wire"`, `"40–60"` (use `"at least 40"`), and `"non-vim.fn.sha256"`.
