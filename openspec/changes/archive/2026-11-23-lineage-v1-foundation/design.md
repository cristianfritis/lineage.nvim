## Context

`lineage.nvim` is a greenfield Neovim 0.10+ plugin backed by a standalone Rust sidecar (`lineage-engine`) for demand-driven, flow-sensitive, null-aware and data-aware interprocedural slicing. This change delivers **Milestones M0 + M1** (core engine, NDJSON transport, TypeScript/TSX/JS adapter, Interactive Two-Phase Hover, observability/CLI tools, and the M1 empirical measurement gate), keeping `LanguageAdapter`, the absence lattice, and `SliceEnvelope` extensible for follow-on M2–M4 OpenSpec changes.

Empirical benchmarks in sibling plugin `refactor-neo.nvim` (`ADR-0005`, `ADR-0006`) established that workspace-wide scanning and Tree-sitter parsing in Neovim Lua causes 7.0–8.6s wall time and 60–287ms main-loop stalls on TypeScript, justifying a standalone Rust sidecar.

### Component Topology & Phase 2 Reverse-RPC Flow

```text
Neovim 0.10+ (lineage.nvim, Lua)                 lineage-engine (Rust)
  [Commands / Hover Float / Jump Stack]            [Reader Thread -> Worker Pool -> Writer Thread]
  [Dirty Buffer Sync (changedtick, <=1MB)]  <--->  [2-Granularity Parser -> TypeScript Adapter]
  [LSP Compat Proxy (0.10/0.11)]      NDJSON stdio [Pruned SSI Core IR + 2-Axis Product Lattice]
                                                   [FileIrCache (LRU) + SliceArena (query-scoped)]
                                                   [5-Stage Deterministic Slicer -> HoverCard]
```

**Phase 2 Reverse-RPC Control Flow:** Neovim initiates a query by sending `lineage/slice` with a client ID (`"c-<n>"`) and monotonic `query_id`. `lineage-engine` returns Phase 1 (`SliceEnvelope` with `hover_card` projection) immediately from static analysis and warm cache. If unresolved cross-file boundaries remain and an LSP client is attached, `lineage-engine` issues a reverse-RPC request `lineage/lspResolveBatch` with an engine ID (`"e-<n>"`) over the same stdio pipe containing all unresolved sibling symbols at BFS depth `d`. Neovim's Lua bridge queries the first non-circuit-broken attached LSP client supporting each method (`definition`, `typeDefinition`, `implementation`), sends the raw locations + `client.offset_encoding` back to Rust, and Rust parses the target files from disk (or synced dirty-buffer overlays) and streams an idempotent `lineage/sliceDelta` notification within the `250ms` ceiling.

## Goals / Non-Goals

**Goals (M0 + M1):**
- Deliver Phase 1 function-scoped static/warm-cache hover in `< 50ms` (p95) and Phase 2 LSP-enriched overlay within a fixed `250ms` wall-clock ceiling, with intraprocedural-only downstream danger sinks in hover.
- Enforce a memory ceiling of `clamp(5% host RAM, 128 MB, 300 MB)` using `FileIrCache`'s byte-weighted LRU cap, a bounded dirty-buffer map (max 32 buffers / 16 MB), query-scoped AST disposal, and `1 MB` / `4096`-byte pre-parse guards.
- Keep `Unknown` structurally separate from `Absent` and display unverified/truncated counts inline on Line 1 of hover so partial coverage is always visible.
- Ship the TypeScript/TSX/JavaScript adapter with statically linked grammars, declaration-only outer-wrapper signature extraction, built-in stdlib summaries, and bounded static `tsconfig.json`/barrel resolution.
- Validate the M1 empirical gate on `zod`, `trpc`, and `TypeScript`: `unknown_external_generic_calls / total_external_generic_calls <= 25%`, false-positive rate `<= 1%`, Phase 1 p95 `< 50ms`, Phase 2 p95 `<= 250ms`, and steady-state RSS within budget.

**Non-Goals (Explicitly Deferred or Excluded):**
- **Deferred to follow-on OpenSpec changes (M2–M4):** Python/Rust/Go adapters (M2), Inline Focus Mode + Interactive Tree Panel + bounded Rust ripgrep fallback + 2-step `prepareCallHierarchy` (M3), cross-platform prebuilt GitHub Release binaries + `checksums.json` + Mason (M4), Tier-2 `OverlayCache`, dual priority worker lanes, and custom allocator purging (`ts_set_allocator`/`mi_collect`/OS RSS hysteresis, added in M2+ only if M1 RSS benchmarks exceed budget).
- **Out of scope permanently:** Eager whole-repo indexing, persistent on-disk symbol databases, LSP `textDocument/hover` Markdown scraping (re-evaluated only if M1 gate exceeds `25%`), background Neovim `bufload`/synthetic `didOpen`/`didClose`, whole-program heap alias analysis, concurrency races, deep container tracking, interprocedural uncaught exceptions, dynamic reflection/metaprogramming, complex macros, and static virtual dispatch.

## Decisions

1. **Custom Byte-Weighted LRU (`FileIrCache`) + Query-Scoped `SliceArena` (No Salsa, No Premature Allocator Machinery):**
   - *Decision:* Tier 1 `FileIrCache` (`moka::sync::Cache`, byte-weighted LRU capped at `clamp(5% host RAM, 128 MB, 300 MB)`) stores `FileSkeleton`s, lazily lowered function SSI IR, and strictly file-local parametric `FnSummary` (symbolic call targets only, never baking in callee bodies; disk files validated via `stat()` `(mtime, len)`). Tier 3 `SliceArena` is per-query scratch memory holding up to 4 parsed Tree-sitter `Tree`s for intra-file multi-hop slicing and dropped unconditionally when the query ends. Dirty-buffer overlays are bounded to 32 buffers / 16 MB LRU. Defer `OverlayCache`, `ts_set_allocator`, and OS RSS polling to M2+ unless M1 benchmarks require them.
   - *Rationale:* Enforces hard memory bounds without Salsa's revision-memoization conflict or `mimalloc`/RSS spin-loop complexity, while avoiding repeated re-parses of large files within a single query.
2. **Pruned SSI (`Phi` + `Pi` + `Unwrap`/Projections) + Bounded Relational Guards + 2-Axis Product Lattice:**
   - *Decision:* Lower functions into Pruned SSI with multi-output calls (`outputs: Vec<ValueId>`, `exit_edges: Normal | EarlyExit`), explicit `ProjectField`/`ProjectIndex`/`UnwrapVariant` instructions, bounded guard facts (`StateIn(Place, BitMask)`, `Implies(FactA, FactB)`, `Unreachable`), and a 2-axis product `AbsenceLattice` (`known_states: BitMask`, `unknown_reasons: SmallBitSet`). Lower top-level module statements into a synthetic `<module>` scope.
   - *Rationale:* Turns backward slicing into direct def-use traversal, supports TS/Python/Rust/Go idioms without an SMT solver or post-v1 `OutParam`/`Exceptional` bloat, and guarantees `join(Present, Unknown)` never collapses into `MaybeAbsent`.
3. **Full-Duplex JSON-RPC 2.0 NDJSON Wire Contract:**
   - *Decision:* Single-line UTF-8 JSON-RPC 2.0 over stdio (`{"jsonrpc":"2.0","id":...,"method":...,"params":...}` + standard JSON-RPC 2.0 `error: { code, message, data? }`), dedicated stdin reader thread + stdout writer thread + worker pool (`crossbeam-channel` + `oneshot`), strict `stdout` isolation (source-free rotating log file via `tracing-subscriber` + `tracing-appender`, panics to `stderr`), `"c-<n>"` vs `"e-<n>"` ID spaces, and bidirectional `$/cancelRequest`.
   - *Method Catalog (M0+M1):* `initialize` (`protocol_version: 1u32`, `engine_version`, `capabilities`), `lineage/slice` (`uri`, `position`, `direction`, `include_graph: false`, `include_snippets: true`), `lineage/sliceDelta` (server notification), `lineage/lspResolveBatch` (reverse-RPC), `lineage/syncBuffer` (`uri`, `changedtick`, `text`), `lineage/closeBuffer` (`uri`), `lineage/status`, `lineage/debugDump`, `$/cancelRequest` (`query_id`), `shutdown`.
   - *Envelope & Projection DTOs:* `SliceEnvelope { schema_version: 1, query_id, trace_id, root_ids, direction, files: Vec<FileEntry>, graph?: { nodes, edges }, projections: { hover_card?: HoverCardProjection }, truncation: Vec<TruncationReason>, stats }`. `HoverCardProjection` carries `verdict_line`, `is_trivial_peek`, `phase1_items` (immutable `[1]..[k]`), `phase2_items` (appended `[k+1]..[9]`), `guards`, `local_risks`, and `unknown_boundaries` (each with `UnknownReason`: `external_untyped | dynamic_dispatch | lsp_timeout | lsp_unattached | lsp_empty | circuit_open | hop_budget_exhausted | barrel_fanout_cap | file_too_large | non_regular_file | tree_sitter_error_node | polymorphic_cap | max_depth`).
   - *Rationale:* Eliminates `Content-Length` partial-header bugs in chunked Lua reads, prevents Rust/Lua schema drift, and keeps `< 50ms` hover payloads lean by making full `graph` opt-in.
4. **Location-Only Batched LSP Bridge + Declaration-Only Outer-Wrapper Extraction + Bounded Static `tsconfig`:**
   - *Decision:* Restrict LSP calls to `definition`, `typeDefinition`, and `implementation` (cap `5`, above which emit `Unknown: polymorphic_cap`). Batch unresolved sibling symbols per BFS depth with a `150ms` default per-request timeout inside the `250ms` Phase 2 ceiling. Track per-`(lsp_client_name, workspace_root)` EWMA (`alpha = 0.25`, cold-start clamp `300ms`) and circuit breaker (`3` consecutive timeouts -> open for `30s` half-open probe). Query the first non-circuit-broken client supporting each method. Never call `textDocument/hover`, `bufload`, or synthetic `didOpen`/`didClose`. For `.d.ts` stubs, inspect only the outermost nullable wrapper (`T | null`, `T | undefined`, `T | null | undefined` -> `MaybeAbsent`; bare `T` -> `Unknown: external_untyped`), paired with a built-in JS/TS stdlib table. Key `tsconfig.json` caches by nearest-ancestor directory (JSONC via `jsonc-parser`, basic `paths`/`baseUrl` + single-level relative `"extends"`) and cap wildcard `export *` barrels in Phase 1 (max 2 hops, max 10 siblings, `10ms` sub-budget -> defer to Phase 2 or `Unknown: barrel_fanout_cap`).
   - *Rationale:* Avoids corrupting Neovim's active LSP session, avoids brittle hover Markdown parsers and mini type-checkers, and prevents wildcard mega-barrels from blowing the `50ms` Phase 1 budget.
5. **Prefixed Hover Keymaps, Line-1 Partial-Coverage Qualifiers & Neovim 0.10/0.11 Compatibility:**
   - *Decision:* Keep `enter_on_open = true` (configurable); bind `<CR>` (jump under cursor), `g1`–`g9` (direct numbered jump), `gu` (toggle unverified boundaries), `q`/`<Esc>` (close & restore cursor), all remappable in `opts.hover.keymaps` (reserving `gt`/`gf`/`gn`/`gs`/`gp` for M3). Anchor the float once on open (`NW`/`SW`), append Phase 2 at `[k+1]..[9]` deduplicated by `(file_id, range)` without renumbering `[1]..[k]`, qualify Line 1 whenever any upstream path is unverified/truncated, display `Unverified` (never raw `Unknown`) in TS/JS, push jumps to `m'` + Lineage Jump Stack, and branch Neovim 0.10 vs. 0.11+ LSP APIs via `vim.fn.has("nvim-0.11") == 1` in `lua/lineage/lsp_compat.lua` using `vim.uv.spawn`.
   - *Rationale:* Preserves Vim counts (`3j`) and motions, prevents false-safety impressions, and avoids Neovim 0.10 colon-call crashes.
6. **Statically Linked Grammars & M1 Local Binary Discovery:**
   - *Decision:* Statically link `tree-sitter-typescript` (TS + TSX) and `tree-sitter-javascript` (`.js`/`.jsx`/`.mjs`/`.cjs`) directly into `lineage-engine`. Resolve binary path in order: `opts.binary_path` -> `stdpath('data')/lineage/bin/lineage-engine-v<semver>` -> `engine/target/release/lineage-engine` -> `PATH`.
   - *Rationale:* Eliminates `dlopen` Tree-sitter ABI mismatches while keeping M1 dogfooding self-contained before M4 prebuilt release distribution.

## Alternatives Considered

- **Pure Lua (`vim.treesitter` + `vim.lsp`) vs. Standalone Rust Sidecar:** Pure Lua avoids binary compilation and stdio framing, but empirical benchmarks in `refactor-neo.nvim` (`ADR-0005`) showed 7.0–8.6s wall time and 60–287ms Neovim main-loop stalls on TypeScript, and Lua cannot run standalone CLI snapshot/fuzz suites. Rejected in favor of a Rust sidecar scoped strictly to M0 + M1 first.
- **Salsa vs. Custom Byte-Weighted LRU:** Salsa provides automatic incremental memoization, but resists hard global byte-weighted LRU caps and conflicts with time-budgeted LSP overlays. Rejected in favor of `moka::sync::Cache` + query-scoped `SliceArena`.
- **Mutable CFG + Dominator Walks vs. Pruned SSI:** Mutable CFGs require repeated dominator walks during backward slicing and complicate branch-narrowing provenance. Rejected in favor of Pruned SSI (`Phi` + `Pi` + projections).
- **LSP `hover` Scraping & Synthetic `didOpen`/`didClose` vs. Disk-First + Outer-Wrapper Stubs:** Scraping `hover` Markdown breaks across LSP versions, and raw `didOpen`/`didClose` over Neovim's live LSP transport risks state desync and ghost diagnostics. Rejected in favor of location-only LSP + `.d.ts` outer-wrapper extraction + bounded static `tsconfig` resolution (gated on M1 `<= 25%` measurement).
- **Dynamic `dlopen` of `nvim-treesitter` `.so` Grammars vs. Statically Linked Grammars:** `dlopen` exposes users to Tree-sitter ABI 13/14/15 crashes and macOS Gatekeeper `.dylib` failures. Rejected in favor of statically linked grammars.

## Risks / Trade-offs

- **[External generic calls degrading to `Unknown` without LSP `hover`]** → Mitigated by `.d.ts` outer-wrapper extraction + built-in JS/TS stdlib table, and gated on the M1 empirical threshold (`unknown_external_generic_calls / total_external_generic_calls <= 25%` on `zod`, `trpc`, `TypeScript`).
- **[Untrusted repo path traversal or pseudo-filesystem hangs]** → Mitigated by rejecting non-regular files (`!meta.is_file()`) and `/proc`/`/sys`, guarding static `tsconfig.json` relative joins against `..` escape outside workspace root (while trusting URIs returned by the user's attached LSP so `pnpm`/Nix symlinks work), and keeping rotating logs source-free (paths, symbol names, timings, state tags only).
- **[Sidecar crash desyncing dirty buffers or large buffers freezing Lua]** → Mitigated by a max-3-restarts cap, resetting Lua's `b:changedtick` map and rejecting pending callbacks on exit, and enforcing a Lua-side `> 1 MB` pre-check before `lineage/syncBuffer`.
- **[Large/minified bundles spiking parse RAM or time]** → Mitigated by `1 MB` file size / `4096`-byte line pre-parse guards, `min(4, max(1, cpus / 2))` parse workers, and query-scoped AST disposal (max 4 active `Tree`s in `SliceArena`).
- **[Neovim 0.10 vs. 0.11+ LSP API drift & UTF-16 offset bugs]** → Mitigated by `lua/lineage/lsp_compat.lua` (`vim.fn.has("nvim-0.11") == 1`), centralizing UTF-8/16/32 math in Rust via `line-index`, and headless CI testing on both `0.10.x` and `0.11.x`.

## Migration Plan

Greenfield repository; no existing user data migration required. Protocol compatibility is enforced from M0 via `initialize` (`protocol_version: 1`) failing fast on mismatch and additive-only `SliceEnvelope` (`schema_version: 1`) evolution. When M4 adds prebuilt binary downloads in a follow-on change, releases will use a two-stage flow (`engine-v<semver>` builds/uploads binaries -> commit `checksums.json` -> tag plugin `v<semver>`) verified via native OS SHA-256 CLIs (`sha256sum`/`shasum -a 256`/`Get-FileHash`) with `cargo build --release` fallback.

## Open Questions

- **M1 External Generic `Unknown` Gate:** To be measured during M1 benchmarking on `zod`, `trpc`, and `TypeScript` (`unknown_external_generic_calls / total_external_generic_calls <= 25%`, false-positive rate `<= 1%`, steady-state RSS within budget). Results determine whether M2 needs an opt-in server-specific fallback or custom allocator purging.
