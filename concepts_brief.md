# lineage.nvim: Demand-Driven Null-Aware & Data-Aware Interprocedural Analyzer

* **Author:** cristianfritis
* **Date:** 2026-11-23

## Overview

`lineage.nvim` is a Neovim 0.10+ plugin backed by a standalone Rust sidecar (`lineage-engine`) that traces how values and absence/nullability states flow across variables, branches, guards, functions, and files. Standard LSP endpoints (`references`, `callHierarchy`, `hover`) show where a symbol appears or its static type at a point, but do not answer *which specific assignments, branches, or cross-file returns can produce an absent or null value at the cursor, or which guards narrowed it along the way*.

The target user is a developer working in medium-to-large multi-file repositories (TypeScript/JavaScript, Python, and Rust in v1, expanding to Go, Java, C#, C/C++, and PHP in later tiers) who needs fast, low-noise origin tracing without waiting for eager whole-repository indexing or triaging high-false-positive static-analysis warnings.

Core value proposition:
1. **Priority #1 (Flagship):** Interactive jumpable hover (`:Lineage hover`) with direct jump redirection to exact absence/value producers across files and a "Data Flow To Here" origin view.
2. **Priority #2:** Scoped code comprehension via Inline Focus Mode (dimming unrelated lines, tagging flow states, pinning variables across buffers) and an Interactive Tree Panel (origins, impact, leaf grouping, test-file filtering, synchronized code preview).
3. **Priority #3 (Post-v1):** Derived visual views and opt-in unguarded-dereference diagnostics built on the same canonical graph schema.

---

## Analysis

Seven independent review lenses audited the architecture (`egm-design`, `egm-codebase-details`, `egm-zoom-out`, `egm-comprehension`, `egm-implementation-readiness`, `egm-structural`, and `egm-inventive-catalyst`), alongside empirical benchmarks and LSP probe records from the sibling repository `refactor-neo.nvim`. Below is the finalized synthesis of resolved contradictions, architectural mechanisms, and trade-offs.

### 1. Core Engineering Contradictions & Locked Resolutions

| Contradiction | Conflict Identified | Locked Resolution | Secondary Trade-Off |
| :--- | :--- | :--- | :--- |
| **Zero LSP `hover` scraping vs. external library & generic nullability** | Location endpoints (`definition`, `typeDefinition`) land in `.d.ts`/`.pyi`/Rust traits with generics or overloads, and `typeDefinition` strips primitive union members (`null`/`undefined` have no source location). Tree-sitter cannot instantiate arbitrary generics. | **Declaration-Only Signature Extraction + Outer-Wrapper Nullability Rule + Curated Stdlib Summary Table:** Parse only the target declaration span at the LSP `definition` location with Tree-sitter (never lowering external bodies). Preserve outer-wrapper absence without full generic instantiation (`Array<T>.find(): T \| undefined`, `dict.get(): V \| None`, `Iterator::next() -> Option<Self::Item>` prove `MaybeAbsent` directly; fall back to `Unknown` only for bare uninstantiated `T` that cannot be substituted in 1 step). Backstop with a curated built-in stdlib summary table. **M1 Measurement Gate:** Measure external generic `Unknown` rates during M1 TypeScript dogfooding; only consider an opt-in server-specific fallback in M2 if `> 25%` of real-world external generic calls still degrade to `Unknown`. | Requires per-adapter return-signature/overload parsing, read-only path sandboxing for `node_modules`/cargo caches, and upkeep of a small stdlib table. |
| **Zero eager indexing & immediate AST drop vs. `< 50ms` Phase 1 hover on large files** | Lowering an entire 3,000-line file into CFG/IR on cold hover or after every edit blows the `50ms` latency budget and spikes RAM. | **Two-Granularity Lazy Parsing (Skeleton Index + Function-Scoped Lowering):** Parse once to record a lightweight top-level symbol/import/function byte-range skeleton, then lower *only* the enclosing function touched by the query before dropping the AST. Cache IR per `(uri, symbol_id, content_hash)` so editing function A never evicts function B. | Nested closures and module top-level statements require explicit captured-variable and synthetic module-scope boundaries. |
| **Safe unopened-file handling vs. cross-file slicing past hop 1** | Synthetic raw RPC `didOpen` -> query -> `didClose` on Neovim's live LSP connection risks state-machine desync if the user opens the file mid-flight, floods `vim.diagnostic` with ghost diagnostics, and triggers 500ms-5s cold server stalls. | **Drop Synthetic `didOpen`/`didClose` from v1 Entirely:** Rely on **disk-first Rust slicing** (shadowed by a dirty-buffer overlay for modified Neovim buffers) + **lightweight static project resolvers** (`tsconfig.json` `paths`/`baseUrl` + `index.ts` barrels, `pyproject.toml`/`src` layout, `Cargo.toml` workspaces, `go.mod` module paths) + **stateless workspace URI queries** only when supported natively by the server. | Adds ~200 lines of manifest/import resolution per language; dynamic re-exports or macro-generated bindings in unopened files remain `Unknown`. |
| **Rich 9-language absence idioms vs. minimal language-agnostic core IR** | Modeling TS `null`/`undefined`/truthiness, Python `None`/`assert`, Rust `Option`/`Result`/`?`, Go `(val, err)`, and C/C++ pointers in a unary-guard CFG either leaks language conditionals into the solver or breaks on Tier-2 languages. | **Pruned SSI (`Phi` + `Pi` + `Unwrap`/Projections) + Bounded Relational Guard Algebra:** Stateless adapter desugaring (`?.`, `??`, `?`, destructuring) lowers into one core IR with multi-output calls (`Return \| OutParam`), explicit projection/unwrap edges, and bounded relational guard facts (`StateIn(Place, BitMask)` and `Implies(FactA, FactB)`). | Relational guard pairs require a bounded cardinality cap to avoid quadratic blowup; desugaring must carry original source-span provenance so jumps land on user syntax (`a?.b`), not synthetic temporaries. |
| **Append-only Phase 1 hover numbering vs. Vim ergonomics & coverage honesty** | Bare `1`-`9` collides with Vim counts (`3j`); bare `f`/`t`/`n`/`u` collide with Vim motions; collapsing `Unverified` hides missing coverage and creates a false sense of safety. | **Zero-Collision Prefixed Hover Keymaps + Line-1 Coverage Honesty:** Keep `enter_on_open = true` (configurable), preserve all standard Vim navigation/count keys (`j`/`k`/`/`/`n`/`N`/`f`/`t`/counts), bind `<CR>` + `g1`-`g9` + `gt`/`gf`/`gn`/`gs`/`gp`/`gu` + `q`/`<Esc>`, and always surface unverified/truncated counts directly on Line 1 (never show an unqualified `NonNull`/`Present` if any upstream path hit `Unknown`, hop limit, or timeout). | Two-keystroke prefixed actions (`g*`) instead of single-letter actions; reserves `[6]..[9]` when open cross-file boundaries exist so Phase 2 origins stay visible without scrolling. |
| **Strict `(uri, doc_version, pos)` LSP cache keying & `40ms` timeouts vs. EWMA stability** | Keying LSP cache on file-wide `doc_version` flushes 100% of entries on every keystroke; a `40ms` per-request timeout trips the circuit breaker immediately on `rust-analyzer` and `tsserver`. | **Symbol-Anchored Stale-While-Revalidate Cache + `120-150ms` Parallel Batched Requests:** Key cache by `(uri, enclosing_symbol_id, target_symbol_key)` with `doc_version` as freshness metadata. Raise per-request LSP timeout to `120-150ms` and batch all sibling unresolved symbols per BFS depth level inside the fixed `250ms` total Phase 2 ceiling. | Must adjust cached positions on line edits and mark transiently stale entries during background revalidation. |
| **Precision over soundness vs. usefulness in unannotated dynamic code** | Collapsing all unresolvable calls/params into a generic `Unknown` turns unannotated JS/Python/PHP code into unhelpful dead ends. | **Stratified `Unknown` Reason Taxonomy + Non-Alarming Local Defensive Clues:** Separate `Unknown` structurally from `Absent`, tag every boundary with a machine-readable reason (`external_untyped`, `dynamic_dispatch`, `lsp_timeout`, `max_depth`), display boundary counts on Line 1, and surface local author-written checks (`checked for None at L14`) as quiet informational context without inventing fake `Nullable` origins. | Adds compact status-line metadata that must stay concise to avoid hover clutter. |

**Ideal Final Result (IFR):** A developer invokes hover or origin tracing on any variable and receives deterministic, sub-50ms local provenance and absence state using the language's own terminology, enriched within 250ms by parallel batched, protocol-safe LSP location lookups—with zero false-positive null alerts, honest Line-1 coverage qualifiers, zero background Neovim buffer or LSP session mutations, zero hover-text scraping, and bounded memory/CPU even on 500k+ LOC monorepos.

---

### 2. Deep-Dive Architectural Blueprint Across the 7 Design Areas

#### A. Unified Dataflow IR & `LanguageAdapter`
* **Core IR Representation:** **Pruned SSI (SSA with `Phi` merges + `Pi` branch-edge refinements)** plus explicit `ProjectField`, `ProjectIndex`, `UnwrapVariant`, and multi-output `Call { inputs, outputs: Vec<Return | OutParam>, exit_edges: Normal | EarlyExit | Exceptional }`. Backward slicing reduces to direct def-use graph traversal without repeated dominator walks.
* **Bounded Guard Algebra:** Constrain core guards to `StateIn(Place, BitMask)` (`u16`/`u32` copyable bitset), `Implies(FactA, FactB)` (covering Go `err == nil => val != nil`, Go comma-ok, TS discriminated unions `res.ok => res.data != undefined`, TS type predicates `x is User`), and `Unreachable`. No SMT solver in core.
* **Structural `Unknown != Absent` Product Lattice:** Represent lattice values as a 2-axis product `(known_states: BitMask, unknown_reasons: SmallBitSet)` so `join(Present, Unknown)` never collapses into `MaybeAbsent`. `{Present} + unknown` renders as `NonNull (partial — N unverified [gu])`; `{Absent} + unknown` preserves the witnessed origin alongside the unverified count.
* **Language-Specific Lowering Rules (Golden-Tested per Adapter):**
  * **TypeScript/JavaScript:** Separate `null` vs. `undefined` (`=== null` vs. `== null` vs. default params catching `undefined` only); truthiness asymmetry (`if (x)` removes `null | undefined` in `then`, while `else` still permits `0`/`""`/`false`/`NaN`); `a?.b?.c` short-circuiting to `undefined`; `??`, `??=`, non-null `!`; statically link **both** `LANGUAGE_TYPESCRIPT` and `LANGUAGE_TSX`; handle `strictNullChecks: false` without flagging every variable.
  * **Python:** Truthiness asymmetry (`else` is `{None, Falsy}`, not guaranteed `None`); `is None` / `is not None`; `assert x is not None` as an explicit guard; walrus `:=`; default `= None`; comprehension filters.
  * **Rust:** `Option`/`Result` payload extraction (`if let`, `match`, `let ... else`) tracing through `Unwrap` edges back to `Some`/`Ok` producers; `?` simultaneous early-exit + survivor unwrap; `.unwrap()`/`.expect()` danger sinks; stdlib combinators (`.map`, `.and_then`, `.unwrap_or`, `.ok_or`); `unsafe` raw pointers (`Unknown`).
  * **Go (M2 IR Proof Target):** Zero-value initialization (`*T`/slice/map/interface start `Nil`; `int`/struct start `NonNil`), correlated `(val, err)` via `Implies`, named returns + `defer` mutations (mark mutated named returns `Unknown`), typed-nil interface caveat.
* **Post-AST Cursor Mapping & Broken Buffers:** Retain a compact sorted byte-span index + `line-index` table per `FileIr` after dropping the Tree-sitter AST. Lower Tree-sitter `ERROR`/`MISSING` subtrees deterministically into poison `Unknown` nodes while still lowering sibling valid statements so hover works mid-edit.

#### B. Engine Internals, Incrementality & Memory Ceilings
* **Custom 3-Tier Cache Architecture (Salsa Rejected):**
  1. **Tier 1 (`FileIrCache`, byte-weighted LRU via `moka::sync::Cache` or `quick_cache`):** Keyed by `(file_uri, content_hash, adapter_version)`. Stores strictly **file-local, parametric** `FnSummary` and lazily lowered function SSI IR (symbolic call edges only—never baking in callee bodies, so per-file content-hash invalidation is 100% sound with zero cascading invalidation storms).
  2. **Tier 2 (`OverlayCache`, symbol-anchored stale-while-revalidate):** Caches LSP resolution results without mutating Tier 1.
  3. **Tier 3 (`SliceCache` / Ephemeral Query Arena):** Holds active query traversal, SCC fixed-point state, and cycle detection; dropped when the query ends.
* **Interprocedural Fixed-Point & Cycle Termination:** Run Tarjan SCC + monotone worklist iteration over the visited call subgraph starting from `Unreachable` (hard safety cap: 8 iterations -> widen to `Unknown`). Represent recursion structurally via `CycleBackEdge` in the returned graph.
* **Memory Accounting & Pre-Parse Guardrails:**
  * Memory cap formula: `clamp(5% host RAM, 128 MB, 300 MB)`. Cap logical LRU weight at ~60% of budget and sample real OS RSS via `memory-stats`, triggering aggressive eviction above 85% RSS.
  * Route both Rust and Tree-sitter C allocations (`ts_set_allocator`) through `mimalloc` to eliminate `musl` allocator contention, `glibc` arena retention, and C-heap blindspots.
  * Enforce pre-parse guards: `metadata().is_file()` (never hang on FIFOs/named pipes/device symlinks), skip files `> 1 MB` or lines `> 4096` bytes (minified/generated bundles) as `Unknown: FileTooLarge`, and cap concurrent Tree-sitter parse workers (2-4).
  * Validate unopened cached disk files via cheap `stat()` (`mtime`, `size`, `inode`) to catch external `git checkout`/formatter edits without `inotify` watch exhaustion.
  * **Dual-Lane Priority Scheduler:** Separate high-priority Hover lane from background Graph/Ripgrep worker pool to eliminate head-of-line blocking.
  * **Scan-Resistant Ripgrep Caller Discovery:** Enforce language-extension filters, word-boundary identifier matching (not raw substring `memchr`), quoted-specifier prefiltering for TS/JS imports (validated in `refactor-neo.nvim` to cut candidate parses by ~5x on the TypeScript repo), hard caps (`<= 5,000` files walked, `<= 200-500` files parsed, `<= 2s` deadline, `min(4, cpus/2)` threads), and probation-isolated caching so cold scans never evict the hot working set.

#### C. Hybrid LSP Bridge Protocol
* **Full-Duplex NDJSON Transport:** Dedicated Rust reader thread + writer thread + worker pool with multiplexed `id -> oneshot::Sender` table, non-colliding ID spaces, and strict `stdout` isolation (logs to rotating file, panics to `stderr`). Lua uses a stateful line accumulator and wraps all Neovim API/LSP calls in `vim.schedule` to avoid `E5560` libuv fast-event crashes.
* **Parallel Depth-Level Batching & Per-Client EWMA:**
  * Raise per-request LSP timeout to **`120-150ms`** inside the fixed **`250ms` total Phase 2 ceiling**, batching all unresolved sibling symbols at BFS depth `d` into a single `lineage/lspResolveBatch` reverse RPC so 1-2 parallel waves complete cleanly.
  * Scope EWMA latency and circuit-breaker state **per `(lsp_client_name, workspace_root)`** (never globally), clamp cold-start latency spikes before updating EWMA, treat fast empty replies (`null` in 2ms) as healthy, and recover via a half-open probe after cooldown (30s).
  * Propagate bidirectional `$/cancelRequest` keyed by monotonic `query_id`, canceling both Rust workers and in-flight Neovim `client:cancel_request(lsp_id)`.
* **Dirty Buffer Sync, Encoding & Empirical LSP Quirks:**
  * Sync dirty buffers lazily on query via `b:changedtick` (`lineage/syncBuffer` / `lineage/closeBuffer`).
  * Standardize `lineage-engine` internally on **0-based `(line, utf8_byte_col)`** with `line-index` (using an ASCII fast-path where `utf16_col == byte_col`, proven in `refactor-neo.nvim`). Pass each client's `client.offset_encoding` alongside raw LSP locations so Rust performs all UTF-8/16/32 math centrally.
  * Isolate Neovim 0.10 vs. 0.11+ LSP API drift (`client:request`, `client:supports_method`, `make_position_params(win, offset_encoding)`) in `lua/lineage/lsp_compat.lua`.
  * **2-Step Call Hierarchy & `vtsls` Caveat (from `refactor-neo.nvim` ADR-0006):** Model `textDocument/prepareCallHierarchy` -> `callHierarchy/incomingCalls` explicitly. Because `vtsls` strips `item.data` from nested call-hierarchy items and rejects `prepareCallHierarchy` on unopened files, fall back cleanly to `textDocument/references` or the Rust ripgrep+Tree-sitter discovery pipeline when `prepareCallHierarchy` is unsupported or returns `null` on unopened hops.
  * Cache project build metadata (`Cargo.lock`/`go.mod`/`tsconfig.json`) once per workspace root (failures included), avoiding repeated subprocess stalls.
* **Deterministic CLI Testability:** Abstract external lookups behind `SymbolResolver` / `LspProvider` with `NullLspProvider` (pure static tests) and `FixtureLspProvider` (recorded JSON fixtures + simulated latencies/timeouts).

#### D. Interactive Hover UX & Mechanics
* **Zero-Collision Vim-Native Keymaps (`enter_on_open = true`, configurable):**
  * Preserve Vim counts (`3j`, `5k`) and motions (`j`, `k`, `/`, `n`, `N`, `f`, `t`). All defaults are user-remappable in `opts.hover.keymaps`:
    * `<CR>`: jump/redirect to the item under the cursor
    * `g1`–`g9`: direct numbered jump (immutable Phase 1 `[1]..[k]` + appended Phase 2 `[k+1]..[9]`, capped at 5 Phase 1 slots when open cross-file boundaries exist)
    * `gt`: open *Data Flow To Here* (Origins)
    * `gf`: open *Data Flow From Here* (Impact)
    * `gn`: open *Null/Absence-only* filtered graph
    * `gs`: toggle inline *Focus Slice*
    * `gp`: pin symbol across files
    * `gu`: toggle expanded *Unverified / Unknown boundaries*
    * `q` / `<Esc>`: close and restore cursor
  * Expose `<Plug>(LineageHover)` and `:Lineage hover`; never silently overwrite user `<leader>l*` mappings on plugin load.
* **Line-1 Coverage Honesty & Language-Native Copy:**
  * Never display an unqualified `NonNull` / `Present` if *any* upstream branch hit `Unknown`, hop cap, or Phase 2 timeout. Always qualify Line 1 (e.g., `user: NonNull (partial — 2 unverified [gu])` or `user: Nullable (2 origins · 1 unverified [gu])`).
  * Never display raw `State: Unknown` in TypeScript/JS (collides with TS's `unknown` type); use `Unverified`. Preserve native terms (`null` vs. `undefined` in TS, `None` in Python, `None` vs. `Err(E)` in Rust).
  * Render a compact 1-3 line peek for trivially `Present` variables with 0 absence origins, 0 danger sinks, and 0 unverified boundaries.
  * Push every jump onto Neovim's native jumplist (`m'`) alongside a dedicated Lineage Jump Stack (`:Lineage back` to return and optionally reopen the prior hover). Guard async Phase 2 updates with `(active_hover_id, win_is_valid, buf_is_valid)` and preserve cursor/anchor placement.
  * Link highlight groups to standard Neovim groups (`Comment`, `DiagnosticWarn`, `DiagnosticError`, `DiagnosticOk`), pair colors with text badges (`[NULL]`, `[OK]`, `[?]`), default to clean Unicode/ASCII (Nerd Fonts opt-in), and mark Inline Focus Mode `[stale]` on `TextChanged` (refreshing on debounced idle or manual trigger).

```text
╭─ user: Nullable (2 origins · 1 unverified [gu]) ────────────╮
│ Receives: fetchUser() | cachedUser | null                   │
│                                                             │
│ Can be null / undefined from:                               │
│  [1] auth.ts:42       return null        (2 calls, 1 file)  │
│  [2] session.ts:18    ?.profile          (1 call, 1 file)   │
│  ⠋ Checking cross-file LSP origins...                       │
│                                                             │
│ Guards: none  │  Risks in fn: L89 user.id                   │
│ Unverified (1): axios.get() [gu to inspect]                 │
├─────────────────────────────────────────────────────────────┤
│ <CR>/g1-9 jump │ [gt]o [gf]rom [gn]ull [gs]lice [gp]in [q]  │
╰─────────────────────────────────────────────────────────────╯
```

#### E. Hard Static Analysis Edge Cases
* **Shallow 1-Level `obj.field` Kill/Gen Rules:** Direct write `obj.f = v` updates only `obj.f`; base reassignment `obj = other` or computed write `obj[k] = v` kills all `obj.*`. Track cheap intra-function single-assignment local aliases (`b = a`). Calls not referencing `obj` preserve `obj.f`; calls passing `obj` preserve narrowing only if provably read-only (Rust `&self`/`&T`), otherwise downgrade narrowed `obj.f` to `Unknown` (never `Absent`).
* **Closures & `async`/`await`:** Immutable/single-assignment captures inherit dominating outer narrowing facts; reassigned/mutated captures reset to joined/pre-narrowed or `Unknown`. Model `await expr` transparently as a value unwrap (`Promise<T> -> T`, `Future -> T`), while treating `await` as a field-invalidation boundary for mutable shared object fields (`this.field`).
* **Polymorphic Dispatch Fan-Out:** Cap LSP `textDocument/implementation` fan-out at `max_implementations = 5`; above that, emit an expandable `PolymorphicBoundary (N implementations)` node marked `Unknown`.

#### F. Shared `FlowGraph` Schema & Explosion Controls
* **Canonical Normalized Graph + Versioned Envelope:**
  * Wrap payloads in `SliceEnvelope { schema_version: 1, query_id, root_ids, direction, files: Vec<FileEntry { file_id, uri, doc_version }>, graph: { nodes, edges }, projections?: { hover_card?, focus_spans?, tree_rows? }, truncation, stats }` and stream Phase 2 via idempotent `GraphDelta`.
  * Keep canonical `Node` free of view booleans (`is_cycle_duplicate`, `is_danger`) and language-coupled `null_state`. Represent cycles structurally (`is_back_edge` / `cycle_group_id`), hazards via typed `facts` (`SinkHazard`), and state via `{ absence_category, domain_state: { domain_id, label, detail } }`. Store joined post-node state on `Node` and guard/narrowing deltas on `Edge`. Make `snippet` optional per request flags.
  * Model non-atomic nodes as first-class `NodeKind` variants: `SourceNode`, `ContractedChain`, `ClusterNode`, and `FrontierNode { stop_reason, omitted_count, expansion_token }`.
* **Rust vs. Lua Boundary:** Rust owns 100% of graph semantics, pre-budget test filtering, copy-chain contraction, SCC detection, ranking, grouping, and future Sugiyama/DOT/SVG projections. Lua owns strictly Neovim UI rendering, window/extmark/jump-stack state, and LSP transport proxying.
* **5-Stage Deterministic Reduction & Budget Pipeline:** (1) Pre-budget test/ignore filtering -> (2) online zero-cost copy-chain contraction -> (3) deterministic best-first scheduling with canonical `(uri, line, col)` tiebreaker -> (4) orthogonal caps (`max_depth = 5`, `max_nodes = 150`, `max_fanout_per_node = 12`, wall-clock budget) emitting `ClusterNode`/`FrontierNode` -> (5) pure post-slice projections.

#### G. Vertical-Slice Rollout, Packaging & Observability
* **Vertical-Slice v1 Milestones (Mitigating YAGNI / Scope-Overload Risk):**
  * **M0:** Core IR/SSI engine, NDJSON stdio transport, `SliceEnvelope` v1, `SymbolResolver` + `FixtureLspProvider`, CLI `dump-ir`/`dump-cfg`/`slice --trace`, `:checkhealth lineage`.
  * **M1:** TypeScript/TSX/JS end-to-end Hover dogfood (validates Phase 1/2, `tsconfig.json` + barrel resolution, `.d.ts` outer-wrapper extraction, jump stack, and the `> 25%` real-repo external generic `Unknown` measurement gate).
  * **M2:** Python + Rust adapters + Go `(val, err)` IR proof (validates all absence and relational paradigms across the unified IR with zero core rewrites).
  * **M3:** Inline Focus Mode + Collapsible Tree Panel + embedded Rust ripgrep caller discovery + 2-step `prepareCallHierarchy` fallback for explicit "Data Flow From Here".
  * **M4:** Release packaging (checksummed GitHub Release binaries, Cargo fallback, Neovim 0.10/0.11 CI matrix, `:Lineage status` & `:Lineage debug-dump`).
* **Packaging & Distribution Safeguards:**
  * **Statically link Tree-sitter grammars** directly into `lineage-engine` (OS demand-pages unused parse tables lazily; zero `dlopen` ABI or macOS Gatekeeper `.dylib` hazards).
  * Build Linux artifacts with **`musl` + `mimalloc`** (or `cargo-zigbuild` glibc `<= 2.28`), sign macOS `arm64`/`x86_64` binaries, and stop the sidecar before atomic rename on Windows.
  * Pin the release version and SHA-256 manifest **inside the plugin git repo** and download directly from versioned release URLs (avoiding GitHub's 60 req/hr unauthenticated REST API limit). **Never use `vim.fn.sha256()` on binary files** (NUL-byte truncation in Vimscript strings); verify via platform CLI (`sha256sum`/`shasum`/`Get-FileHash`) or pure LuaJIT `bit` SHA-256.
  * Install binaries into **version-keyed paths** (`stdpath('data')/lineage/bin/lineage-engine-v<semver>`) with a mandatory `initialize` `protocol_version` handshake for safe plugin rollbacks.
* **Observability from M0:** Propagate `trace_id` across all 4 async hops (Lua UI -> stdio -> Rust -> LSP proxy); ship `:checkhealth lineage`, `:Lineage status` (PID, OS RSS vs. cache bytes, per-client EWMA, circuit-breaker state), `:Lineage log`, `:Lineage debug-dump`, and CLI IR/trace dump commands.

---

## Goals vs. Non-Goals

* **In-Scope Goals (v1):**
  * Standalone Rust sidecar (`lineage-engine`) with full-duplex NDJSON stdio transport, dual priority lanes, and deterministic CLI testing (`NullLspProvider` + `FixtureLspProvider`).
  * Demand-driven slicing with 2-granularity lazy parsing (file skeleton + function-level SSI lowering), strictly file-local parametric summaries, and a custom 3-tier byte-weighted LRU cache (`clamp(5% host RAM, 128 MB, 300 MB)` + `mimalloc`/`ts_set_allocator` + OS RSS safeguard).
  * Pruned SSI + bounded relational guard core IR + 2-axis product absence lattice (`Unknown` structurally separate from `Absent` with machine-readable reason taxonomy), delivered vertically across **TypeScript/JavaScript**, **Python**, and **Rust** with Go `(val, err)` validated in the M2 IR proof.
  * Hybrid LSP bridge limited to structured location endpoints (`definition`, `typeDefinition`, `implementation`, plus `prepareCallHierarchy`/`incomingCalls`/`references` for explicit forward tracing), with `120-150ms` per-request timeouts, depth-level parallel batching inside `250ms` Phase 2, per-client EWMA + circuit breaker, dirty-buffer sync, `line-index` encoding conversion, declaration-only outer-wrapper signature extraction, and a curated stdlib summary table.
  * Three v1 UI modes on a versioned canonical `SliceEnvelope`/`FlowGraph`: Interactive Jumpable Hover (`< 50ms` Phase 1, `250ms` Phase 2, zero-collision `g*` keymaps, Line-1 coverage honesty, intraprocedural-only downstream risks), Inline Focus Mode, and Interactive Tree Panel (with bounded Rust ripgrep fallback).
  * Statically linked grammars, checksum-verified prebuilt GitHub Release binaries (version-keyed install paths, local `cargo build --release` fallback), and M0 observability.
* **Out-of-Scope Non-Goals:**
  * Eager whole-repository indexing, startup LoC scanning, persistent on-disk symbol databases, or retaining Tree-sitter ASTs in RAM.
  * Scraping free-form LSP `textDocument/hover` Markdown strings in v1 (re-evaluated in M2 only if the M1 measurement gate shows `> 25%` external generic `Unknown` rate).
  * Opening background Neovim buffers (`bufload`/`:badd`/`:edit`) or sending synthetic raw RPC `didOpen`/`didClose` notifications to Neovim's live LSP connections in v1.
  * Dynamic metaprogramming/reflection, complex macro expansion & raw pointer arithmetic, whole-program heap alias/points-to analysis, concurrency data races, deep container-element tracking, interprocedural uncaught exception unwinding, and static virtual-dispatch resolution.
  * Deferred to v1.1/v1.2 (supported by the frozen `FlowGraph` schema): Tier-2/3 languages (Go, Java, C#, C/C++, PHP), Sugiyama 2D ASCII/Unicode DAG buffer, DOT/Mermaid/HTML+SVG export + Kitty/Ghostty image preview, Multi-Location Queue, opt-in workspace null-risk diagnostics, and Mason registry distribution.

---

## Alternatives Weighed

1. **Salsa vs. Custom 3-Tier Byte-Weighted LRU + Ephemeral Query Arena**
   * *Rejected by author:* Salsa — monotonic revision memoization resists hard global byte-weighted LRU eviction (`128-300 MB`) and conflicts with non-deterministic, time-budgeted LSP overlays.
   * *Selected:* Custom `moka`/`quick_cache` byte-weighted LRU holding strictly file-local parametric summaries + ephemeral query arena.
2. **Mutable CFG + Dominator Walks vs. Pruned SSI (`Phi` + `Pi` + `Unwrap`) + Bounded Relational Guards**
   * *Rejected by author:* Mutable CFG without `Pi`/projection nodes — requires repeated dominator walks during backward slicing and breaks on Rust `Option`/`Result` unwrapping and Go `(val, err)` correlation.
   * *Selected:* Pruned SSI with bounded `StateIn` + `Implies` guard facts and multi-output calls.
3. **Pure Lua (`vim.treesitter` + `vim.lsp`) vs. Standalone Rust Sidecar (`lineage-engine`)**
   * *Rejected by author:* Pure Lua (`egm-zoom-out` contrarian option) — empirical benchmarks in `refactor-neo.nvim` (ADR-0005) showed Lua workspace scans hitting 7.0-8.6s wall time and 60-287ms main-loop stalls on TypeScript, and pure Lua cannot provide headless CLI snapshot testing or non-blocking parallel ripgrep+Tree-sitter verification.
   * *Selected:* Standalone Rust sidecar, paired with vertical-slice milestones (M0-M4) to prevent v1 scope overload.
4. **LSP `textDocument/hover` Markdown Scraping vs. Declaration-Only Outer-Wrapper Extraction + Stdlib Summaries**
   * *Rejected by author:* LSP `hover` Markdown scraping — brittle across LSP servers and versions.
   * *Selected:* Structured location endpoints + declaration-only Tree-sitter signature extraction with outer-wrapper nullability rules + curated stdlib table, gated by an M1 `> 25%` real-repo `Unknown` threshold.
5. **Opt-In Synthetic Raw RPC `didOpen`/`didClose` vs. Disk-First Slicing + Static Project Resolvers (v1)**
   * *Rejected by author (after 5/7 Blocker findings):* Synthetic `didOpen`/`didClose` over Neovim's active LSP transport — risks server desync, ghost diagnostics, and multi-second cold-open stalls.
   * *Selected:* Disk-first Rust slicing + dirty-buffer overlay + lightweight project-manifest resolvers (`tsconfig.json`, `pyproject.toml`, `Cargo.toml`, `go.mod`) + stateless workspace URI queries where natively supported.
6. **Dynamic `dlopen` of `nvim-treesitter` `.so` Grammars vs. Statically Linked Grammars**
   * *Rejected by author:* `dlopen` — causes Tree-sitter ABI 13/14/15 version crashes, macOS Gatekeeper `.dylib` failures, and `musl` incompatibility.
   * *Selected:* Statically linked grammars inside `lineage-engine`.
7. **LSP `Content-Length` Framing vs. NDJSON over Stdio**
   * *Rejected by author:* `Content-Length` framing — error-prone byte-offset reassembly in chunked Neovim Lua `stdout` callbacks.
   * *Selected:* NDJSON with request IDs, explicit cancellation, and a stateful Lua line accumulator.
8. **Bare Single-Letter Hover Keys (`1`-`9`, `f`, `t`, `n`, `u`) vs. Zero-Collision Prefixed Keys (`<CR>`, `g1`-`g9`, `gt`/`gf`/`gn`/`gs`/`gp`/`gu`)**
   * *Rejected by author:* Bare single-letter keys inside a focused float — collides with Vim counts (`3j`) and standard motions.
   * *Selected:* Zero-collision `g*` hover keymaps (all user-remappable in `opts.hover.keymaps`) with `enter_on_open = true` configurable.
9. **View-Specific Raw Graph in Lua vs. Canonical Normalized `FlowGraph` + Engine-Computed View Projections**
   * *Rejected:* Embedding `is_cycle_duplicate`/`is_danger`/`null_state` on canonical nodes and running graph algorithms in Lua — breaks DAG/Focus/Export consumers and causes test-filter budget starvation.
   * *Selected:* Versioned `SliceEnvelope` + Rust-side 5-stage deterministic reduction pipeline and view projections.

---

## Key Decisions & Staging for Next Steps

All architectural objections from the 7-lens review are resolved and locked for `/egm-draft`:

* **Engine & Memory:** Standalone Rust sidecar `lineage-engine`, Custom 3-Tier Byte-Weighted LRU (`clamp(5% host RAM, 128 MB, 300 MB)`, `mimalloc` + `ts_set_allocator`, OS RSS guard at 85%), 2-granularity lazy parsing (file skeleton + function-level SSI lowering), statically linked grammars (TS + TSX + JS + Python + Rust), dual priority lanes.
* **IR & Absence Lattice:** Pruned SSI (`Phi`, `Pi`, `Unwrap`/projections, multi-output calls), bounded relational guards (`StateIn` + `Implies`), 2-axis product lattice (`Unknown` structurally separate from `Absent` with machine-readable reason taxonomy), deterministic Tree-sitter `ERROR`-node lowering.
* **LSP Bridge & Static Resolution:** NDJSON over stdio with request IDs and bidirectional cancellation, zero synthetic `didOpen`/`didClose` in v1, `120-150ms` per-request timeout with depth-level parallel batching inside `250ms` Phase 2, per-client EWMA + circuit breaker, symbol-anchored stale-while-revalidate overlay cache, declaration-only outer-wrapper signature extraction + curated stdlib table (M1 `> 25%` `Unknown` measurement gate), lightweight static project resolvers (`tsconfig.json`, `pyproject.toml`, `Cargo.toml`, `go.mod`), 2-step `prepareCallHierarchy` with `vtsls` fallback, `line-index` encoding conversion with ASCII fast-path, and `lua/lineage/lsp_compat.lua`.
* **UX & Schema:** Versioned `SliceEnvelope` (`schema_version: 1`) + `GraphDelta` + 5-stage deterministic reduction pipeline; zero-collision hover keymaps (`<CR>`, `g1`-`g9`, `gt`/`gf`/`gn`/`gs`/`gp`/`gu`, `q`/`<Esc>`), Line-1 coverage honesty qualifiers, compact trivial-variable peek, and native jumplist (`m'`) + Lineage Jump Stack.
* **Rollout & Packaging:** Vertical milestones (`M0` core/CLI -> `M1` TypeScript Hover dogfood + `Unknown` gate -> `M2` Python/Rust/Go IR proof -> `M3` Focus/Tree/Ripgrep -> `M4` release binaries + `:checkhealth`), Linux `musl` + `mimalloc`, version-keyed binary paths with in-repo pinned SHA-256 manifest (never `vim.fn.sha256()`), and `initialize` `protocol_version` handshake.
