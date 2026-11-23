# lineage.nvim: Demand-Driven Null-Aware & Data-Aware Interprocedural Analyzer

*   **Author:** cristianfritis
*   **Status:** Draft
*   **Date:** 2026-11-23

--------------------------------------------------------------------------------

## Overview

`lineage.nvim` is a Neovim 0.10+ plugin backed by a standalone Rust sidecar (`lineage-engine`) that traces how values and absence/nullability states flow across variables, branches, guards, functions, and files on demand. It targets developers working in multi-file TypeScript/JavaScript, Python, and Rust repositories (with the core representation designed to extend cleanly to Go, Java, C#, C/C++, and PHP in later releases).

Rather than scanning an entire repository at startup or acting as a background linter, `lineage.nvim` answers targeted, cursor-driven questions in real time: where a value or `null`/`undefined`/`None`/`Option::None`/`Result::Err` came from, which guards narrowed it along the way, where it flows inside the current function or across files, and which upstream boundaries could not be verified.

## The Problem

Neovim's built-in LSP workflows (`definition`, `references`, `callHierarchy`, `hover`) answer structural and point-in-time type questions, not dataflow questions:

1. **No origin provenance:** `hover` tells a developer that `user` has type `User | null | undefined`, but does not show *which* branch, helper return, or optional chain three files away can actually produce `null` or `undefined` at the current line.
2. **No guard or narrowing visibility:** `references` lists every textual occurrence of a symbol in a file or repository without separating dead branches (after `return`, `throw`, `raise`, or `panic!`), assignments that re-introduce absence, and narrowing guards that already proved the value present.
3. **High friction in multi-file tracing:** Answering "where can this `None` come from?" or "what breaks if this returns `null`?" currently forces manual jump-chasing across files, losing cursor orientation and polluting Neovim's jumplist.
4. **Heavyweight alternatives fail editor constraints:** Whole-program static analyzers require slow eager repository indexing, consume hundreds of megabytes of RAM alongside an already-running language server, and emit noisy false-positive null warnings on unannotated dynamic code or third-party libraries.

`lineage.nvim` solves this by running demand-driven, flow-sensitive slicing outward from the cursor, separating proven absence origins from unverified boundaries, and surfacing jumpable results directly inside Neovim through three workflows: an Interactive Jumpable Hover, Inline Focus Mode, and an Interactive Tree Panel.

## Goals

*   **Sub-50ms local hover (Phase 1):** Return intraprocedural and warm-cached static cross-file provenance and absence state in under `50ms` (p95) without eager repository indexing or startup LoC scanning.
*   **Bounded progressive cross-file enrichment (Phase 2):** Enrich unresolved cross-file edges via attached Neovim LSP location endpoints inside a fixed `250ms` wall-clock ceiling without mutating Phase 1 jump indices, shifting hover anchor placement, or blocking normal Neovim editing.
*   **High precision and honest coverage:** Report only proven or explicitly typed absence origins in `Can be null / absent from...` and danger-sink lists. Keep unverified boundaries (`Unknown`) structurally separate from `Absent` and display unverified/truncated counts inline on Line 1 of the hover so users are never shown an unqualified `NonNull`/`Present` verdict when coverage is partial.
*   **Zero-collision Vim ergonomics:** Provide immediate jump redirection (`<CR>`, `g1`–`g9`) and workflow pivots (`gt`, `gf`, `gn`, `gs`, `gp`, `gu`, `q`/`<Esc>`) inside the interactive hover without breaking Vim counts (`3j`) or navigation motions, paired with native jumplist (`<C-o>`) and jump-back breadcrumb navigation.
*   **Strict memory and CPU ceilings:** Keep sidecar memory bounded to `clamp(5% host RAM, 128 MB, 300 MB)` with an OS RSS safeguard, drop Tree-sitter ASTs immediately after function-level lowering, and prevent standalone caller scans from stalling Neovim or evicting the active working set.
*   **Standalone determinism:** Deliver full intraprocedural accuracy and baseline static cross-file resolution with zero running LSP servers, enabling deterministic headless CLI snapshot testing and immediate usability before LSP servers finish indexing.
*   **Tier-1 multi-paradigm validation:** Ship v1 across TypeScript/JavaScript (including TSX), Python, and Rust, plus a Go `(val, err)` IR validation test suite, delivered in vertical milestones (`M0`–`M4`) so the flagship TypeScript hover loop is dogfooded before expanding language and UI surface area.

## Non-Goals

*   **Eager repository indexing or persistent on-disk symbol databases:** No startup workspace scans, no `.lineage` index files on disk, and no retaining Tree-sitter ASTs in RAM.
*   **LSP `textDocument/hover` Markdown scraping in v1:** The LSP bridge uses structured location endpoints only (re-evaluated in `M2` only if the `M1` real-repo measurement gate shows `> 25%` of external generic calls degrading to `Unknown`).
*   **Background Neovim buffer creation or synthetic LSP `didOpen`/`didClose`:** `lineage.nvim` will never call `bufload`/`:badd`/`:edit` in the background or send synthetic `didOpen`/`didClose` notifications over Neovim's active LSP connections in v1.
*   **Soundness-first background linting:** No whole-program points-to/heap alias analysis, concurrency or async race tracking, deep container-element tracking, interprocedural uncaught exception unwinding, dynamic metaprogramming/reflection, complex C/C++/Rust macro expansion, raw pointer arithmetic, or static virtual-dispatch resolution.
*   **Post-v1 languages and visualizations (deferred to v1.1/v1.2):** Tier-2/3 languages (Go, Java, C#, C/C++, PHP), Sugiyama 2D ASCII/Unicode DAG buffer, DOT/Mermaid/HTML+SVG export, Kitty/Ghostty image preview, Multi-Location Queue, opt-in workspace null-risk diagnostics, and Mason registry publishing.

## Open Questions

*   **M1 External Generic `Unknown` Gate:** After implementing declaration-only Tree-sitter signature extraction, the outer-wrapper nullability rule, and the curated stdlib summary table, what percentage of real-world external generic calls in `M1` TypeScript dogfooding repositories still resolve to `Unknown`? Only if this exceeds `25%` will an opt-in, server-specific fallback be designed for `M2`.
*   **Windows `x86_64` Prebuilt Binary Priority in M4:** Should `M4` GitHub Release CI ship `x86_64-pc-windows-msvc` prebuilt binaries on day one alongside Linux (`musl`) and macOS (`arm64`/`x86_64`), or treat Windows as `cargo build --release` fallback until post-v1?

## Critical User Journeys (CUJs)

### CUJ 1: Direct Origin & Nullability Triage via Interactive Hover
*   **Given**: A developer has a TypeScript, Python, or Rust file open in Neovim 0.10+ (with or without an active LSP server) and the cursor is on a variable `user` whose value flows through local assignments, guards, and a cross-file helper function that can return `null`/`None`/`Option::None`.
*   **When**: The developer runs `:Lineage hover` (or a user-configured `<Plug>(LineageHover)` mapping).
*   **Then**:
    *   Within `50ms` (Phase 1), a focused floating window renders with Line 1 showing the provisional verdict and any unverified boundary count (e.g., `user: Nullable (2 origins · 1 unverified [gu])`), language-native origin snippets numbered `[1]..[k]`, active guards, and intraprocedural risks in the current function.
    *   Within `250ms` (Phase 2), any LSP-discovered cross-file origins append strictly into reserved slots `[k+1]..[9]` without renumbering `[1]..[k]` or flipping the float across the cursor line.
    *   Pressing `g1` or `<CR>` on an item cancels any in-flight Phase 2 request, pushes the current position onto Neovim's native jumplist (`<C-o>`) and the Lineage Jump Stack, and jumps directly to the exact producer line across files. Running `:Lineage back` returns to the origin cursor.

### CUJ 2: Honest Partial Coverage on Unverified External Boundaries
*   **Given**: A variable receives its value from an unannotated third-party library call or dynamic dispatch boundary that neither static resolution nor LSP declaration-signature extraction can verify.
*   **When**: The developer triggers `:Lineage hover`.
*   **Then**:
    *   The unverified call is excluded from `Can be null / absent from...` and danger-sink highlights (zero false-positive null warnings).
    *   Line 1 explicitly qualifies the verdict (e.g., `user: NonNull (partial — 1 unverified [gu])`), never displaying an unqualified `NonNull`/`Present`.
    *   Pressing `gu` expands the `Unverified / Unknown boundaries` section showing the exact call site and machine-readable reason (e.g., `external_untyped`, `lsp_timeout`, `dynamic_dispatch`).

### CUJ 3: Zero-Noise Peek on Trivial Variables
*   **Given**: The developer's cursor is on a trivially present local variable or constant (`const count = 0`) with zero absence origins, zero danger sinks, and zero unverified boundaries.
*   **When**: The developer triggers `:Lineage hover`.
*   **Then**: A compact 1–3 line summary renders (e.g., `count: Always present (literal at L12)`) instead of an empty multi-section dashboard, avoiding visual clutter.

### CUJ 4: Scoped Understanding via Inline Focus Mode & Variable Pinning
*   **Given**: A developer is reading a long function (or tracing a value across two buffers) and wants to isolate only statements that read, write, guard, or propagate the variable under the cursor.
*   **When**: The developer triggers `:Lineage focus` (or presses `gs` from Hover), and optionally pins the symbol (`gp` / `:Lineage pin`) before switching buffers.
*   **Then**:
    *   Irrelevant lines in the active buffer dim using theme-derived highlight links (`Comment`), while relevant statements receive semantic highlights and text-paired state tags (`[NULL]`, `[OK]`, `[?]`).
    *   An unobtrusive active-mode indicator confirms Focus Mode or Pinning is active. Editing the buffer marks the slice `[stale]` immediately rather than blocking keystrokes, refreshing only on debounced idle or manual refresh.

### CUJ 5: Multi-Hop Origin & Impact Investigation via Interactive Tree Panel
*   **Given**: A developer needs to trace more than 1–2 hops upstream ("Data Flow To Here") or discover cross-file callers consuming a return value or parameter ("Data Flow From Here"), including in a repository with no active LSP server.
*   **When**: The developer runs `:Lineage to` / `:Lineage from` (or presses `gt` / `gf` from Hover).
*   **Then**:
    *   An interactive split opens within `350ms` (depth cap `5`, node cap `150`, fan-out cap `12`), supporting leaf grouping, null/absence filtering, test-file exclusion (applied before budget caps), cycle badges, synchronized floating code preview, and lazy expansion of `FrontierNode` / `ClusterNode` entries.
    *   In "Data Flow From Here", the engine queries attached LSP `prepareCallHierarchy` -> `incomingCalls` (falling back to `references` when unsupported or on `vtsls` unopened-file stops), or runs the bounded parallel Rust ripgrep+Tree-sitter candidate filter when standalone, displaying a `[partial]` badge if caps were reached.

## Technical Plan (Architecture)

### 1. System Topology & Strict Rust/Lua Boundary

`lineage.nvim` separates semantic analysis from editor presentation across a full-duplex **NDJSON over stdio** process boundary. `lineage-engine` (Rust) owns 100% of parsing, IR lowering, caching, static module resolution, interprocedural slicing, cycle detection, ranking, grouping, and view projections. `lineage.nvim` (Lua) owns strictly Neovim UI rendering, window/extmark/jump-stack state, buffer dirty-text sync, and proxying structured requests to Neovim's attached LSP clients.

```text
+-----------------------------------------------------------------------------+
|                        Neovim 0.10+ (lineage.nvim, Lua)                     |
|                                                                             |
|  [Commands / Keymaps]   [Hover Float / Focus Extmarks / Tree Split]         |
|           |                                 ^                               |
|           v                                 | (vim.schedule UI updates)     |
|  [Session & Jump Stack]  <-- NDJSON stdio --+--> [LSP Compat Proxy]         |
|  [Dirty Buffer Sync]         (line accum)        (definition/typeDef/impl/  |
|                                                   callHierarchy/references) |
+----------------------------------|------------------------------------------+
                                   | Full-Duplex NDJSON (stdout = RPC only;
                                   | stderr = panics; rotating log file)
+----------------------------------v------------------------------------------+
|                      lineage-engine (Standalone Rust)                       |
|                                                                             |
|  [Reader Thread] --> [Priority Router: Hover Lane | Background Graph Pool]  |
|  [Writer Thread] <-- [Multiplexed id -> oneshot Reverse-RPC Table]          |
|                                                                             |
|  [2-Granularity Parser] -> [LanguageAdapter (TS/TSX/JS, Py, Rust)]          |
|   (Statically Linked TS)     -> [Pruned SSI + Relational Guard Core IR]     |
|                                                                             |
|  [3-Tier Cache: Tier 1 FileIrCache (LRU) | Tier 2 Overlay | Tier 3 Arena]   |
|  [SymbolResolver: Static Manifests + Declaration Stubs + LspProvider + RG]  |
|  [5-Stage Deterministic Slicer & Projectors -> SliceEnvelope / GraphDelta]  |
+-----------------------------------------------------------------------------+
```

*   **Dual Priority Lanes:** Hover queries (`< 50ms` target) run on a high-priority worker lane isolated from background Tree/Graph expansions and standalone ripgrep scans so heavy queries cannot head-of-line block interactive hover.
*   **Strict `stdout` Isolation:** `lineage-engine` writes only NDJSON frames to `stdout`. All `tracing` telemetry writes to a rotating file (`stdpath("log")/lineage-engine.log`), and panics write to `stderr` (captured into a bounded ring buffer inside Neovim).

---

### 2. Unified Dataflow IR (`CoreIR`) & Pluggable `LanguageAdapter`

The analyzer splits into a syntax-blind **Core IR + Solver** and pluggable per-language **`LanguageAdapter`** implementations so Tier-2/3 languages (Go, Java, C#, C/C++, PHP) require zero core rewrites.

#### A. Pruned SSI Core Representation
Instead of a mutable CFG requiring repeated dominator walks during backward slicing, each lowered function uses **Pruned SSI (Static Single Information)**:
*   **`Phi` nodes** merge values at control-flow joins.
*   **`Pi` nodes** attach branch-edge narrowing refinements directly to def-use chains so backward slicing reduces to direct graph traversal.
*   **Explicit Projection & Unwrap Instructions:** `ProjectField(base, field)`, `ProjectIndex(base, idx)`, and `UnwrapVariant(scrutinee, variant_tag)` preserve provenance across Rust `if let Some(u)` / `?`, TypeScript/Python destructuring, and 1-level `obj.field` access.
*   **Multi-Output Calls:** `Call { callee, inputs: Vec<Slot>, outputs: Vec<OutputSlot { index_or_param, kind: Return | OutParam }>, exit_edges: Normal | EarlyExit | Exceptional }` uniformly models single returns, Go `(val, err)` tuples, Rust `?` early exits, and C/C# out-parameters.

#### B. Bounded Relational Guard Algebra
CFG branch edges carry facts from a finite, decidable propositional domain (no SMT solver):
*   `StateIn(Place, BitMask)` — copyable `u16`/`u32` bitset over the adapter's native absence states.
*   `Implies(FactA, FactB)` — bounded relational implication (cardinality capped per block) expressing Go `err == nil => val != nil`, Go comma-ok `ok == true => val != nil`, TypeScript discriminated unions `res.ok === true => res.data != undefined`, TypeScript type predicates `isUser(x) => x != null`, and Rust enum discriminant-to-payload bindings.
*   `Unreachable` — prunes dead branches after `return`, `throw`, `raise`, `panic!`, `break`, `continue`, or `Never`.

#### C. 2-Axis Product `AbsenceLattice` (`Unknown != Absent`)
Every lattice value is a 2-axis product:
```text
LatticeValue {
    known_states: BitMask,          // Adapter-native states (e.g., Null, Undefined, Present, None, Err)
    unknown_reasons: SmallBitSet,   // Machine-readable unverified boundary causes (orthogonal to Absent)
}
```
*   `join(Present, Unknown)` yields `{ known: Present, unknown: {cause} }` — never `MaybeAbsent`. It renders as `NonNull (partial — 1 unverified [gu])`, never as a nullability warning.
*   `join(Absent, Unknown)` yields `{ known: Absent, unknown: {cause} }` — preserving the witnessed absence origin alongside the unverified boundary.
*   Each `LanguageAdapter` projects its native `known_states` onto a closed universal category (`DefinitePresent | PotentiallyAbsent | DefiniteAbsent | Unreachable`) for cross-language UI grouping and coloring while preserving native terminology (`null` vs. `undefined`, `None`, `Option::None` vs. `Result::Err`).

#### D. Tier-1 Lowering Rules, Error Recovery & Shallow Mutation Semantics
*   **TypeScript / TSX / JavaScript:** Statically links both `LANGUAGE_TYPESCRIPT` and `LANGUAGE_TSX`. Distinguishes `null` vs. `undefined` (`=== null`, `== null`, default params catching `undefined` only); models truthiness asymmetry (`if (x)` removes `null | undefined` in `then`, while `else` remains `MaybeAbsent` because `0`/`""`/`false`/`NaN` are falsy); desugars `a?.b?.c`, `??`, `??=`, and `!`; respects precision over soundness even when `strictNullChecks: false`.
*   **Python:** Models truthiness asymmetry, `is None` / `is not None`, `assert x is not None` as an explicit guard, walrus `:=`, default `= None`, and comprehension filters.
*   **Rust:** Models `Option<T>` / `Result<T, E>` payload extraction (`if let`, `match`, `let ... else`), `?` early-return divergence + survivor unwrap (tracing `u: User` back through `Unwrap` edges to `Some`/`Ok` producers), `.unwrap()`/`.expect()` danger sinks, stdlib combinators (`.map`, `.and_then`, `.unwrap_or`, `.ok_or`), and marks `unsafe` raw pointers `Unknown`.
*   **Go (M2 IR Validation Target):** Zero-value initialization (`*T`/slice/map/interface start `Nil`; value types start `NonNil`), `(val, err)` via `Implies`, and mutated named returns under `defer` downgraded to `Unknown`.
*   **Broken-Buffer Recovery & Post-AST Cursor Lookup:** Because Tree-sitter ASTs are dropped right after lowering, every `FileIr` retains a compact sorted byte-span index + `line-index` table mapping `(line, utf8_byte_col)` to IR nodes. Tree-sitter `ERROR` and `MISSING` subtrees lower deterministically to poison `Unknown` (`tree_sitter_error_node`) nodes while sibling valid statements remain sliceable.
*   **Shallow 1-Level `obj.field`, Closures & `async`/`await`:** Direct write `obj.f = v` updates only `obj.f`; base reassignment `obj = other` or computed write `obj[k] = v` kills all `obj.*`; single-assignment local aliases (`b = a`) propagate 1-level kills; unrelated calls preserve `obj.f`; calls passing `obj` preserve narrowing only if provably read-only (Rust `&self`/`&T`), otherwise downgrade narrowed `obj.f` to `Unknown` (never `Absent`). Single-assignment/immutable closure captures inherit outer narrowing; reassigned captures downgrade. `await expr` unwraps `Promise<T> -> T` / `Future -> T` transparently while acting as an invalidation boundary for mutable shared object fields.

---

### 3. Engine Internals, 3-Tier Cache & Resource Bounds

#### A. Two-Granularity Lazy Parsing
Parsing an entire 3,000-line file into SSI IR on cold hover violates the `< 50ms` budget. On first touch (or content change), `lineage-engine`:
1. Parses the file with Tree-sitter to build a lightweight **File Skeleton** (imports, exports, top-level symbol declarations, and enclosing function byte ranges + hashes).
2. Lowers **only the specific function(s)** touched by the query into SSI IR + a strictly **file-local, parametric `FnSummary`**, then drops the Tree-sitter `Tree` immediately.
3. Caches function IR by `(file_uri, symbol_id, function_content_hash)` so editing function A never evicts or re-lowers function B.

#### B. 3-Tier Cache & Hard Memory Ceiling
*   **Tier 1 (`FileIrCache` — byte-weighted LRU via `moka::sync::Cache` or `quick_cache`):** Stores File Skeletons, lazily lowered function SSI IR, and strictly file-local parametric `FnSummary` (symbolic call targets only; never bakes in callee bodies, so content-hash invalidation on edit is 100% sound with zero cascading invalidation).
*   **Tier 2 (`OverlayCache` — symbol-anchored stale-while-revalidate):** Keyed by `(uri, enclosing_symbol_id, target_symbol_key)` with `doc_version` and edited line ranges as freshness metadata. Edits outside a function keep cached Phase 2 LSP results warm; potentially stale entries serve immediately while revalidating within budget.
*   **Tier 3 (`SliceCache` / Ephemeral Query Arena):** Holds per-query interprocedural composition, SCC worklist state, and hop budgets; dropped when the query completes.
*   **Memory Cap Enforcement:** Cap formula is `clamp(5% host RAM, 128 MB, 300 MB)`. Logical Tier 1 LRU weight is capped at 60% of budget, paired with periodic OS RSS checks (`memory-stats`) triggering aggressive eviction above 85% RSS. Both Rust and Tree-sitter C allocations (`ts_set_allocator`) route through `mimalloc` to eliminate `musl` contention, `glibc` arena retention, and C-heap blindspots.
*   **Pre-Parse & Disk-Staleness Guardrails:** Before reading any disk file, verify `metadata().is_file()` (preventing hangs on FIFOs/named pipes/device symlinks), skip files `> 1 MB` or lines `> 4096` bytes (`Unknown: file_too_large`), and bound concurrent Tree-sitter parsers to 2-4 workers. Validate cached unopened disk files via cheap `stat()` (`mtime`, `size`, `inode`) on traversal.

#### C. Interprocedural Fixed-Point & Standalone Ripgrep Caller Discovery
*   **Recursive Cycles:** On recursive call subgraphs (`f -> g -> f`), run Tarjan SCC + monotone worklist iteration over the finite-height bitset lattice starting from `Unreachable` (safety cap: 8 iterations -> widen to `Unknown`). Emit structural `CycleBackEdge` edges in `FlowGraph` rather than duplicating nodes.
*   **Standalone "From Here" Caller Discovery:** When LSP is unavailable or truncated, run `ignore::WalkParallel` + word-boundary identifier matching (`grep-regex` / boundary-checked `memchr::memmem`) inside Rust with: language-extension filters, quoted-specifier prefiltering for TS/JS imports (empirically validated in `refactor-neo.nvim` to cut parses ~5x on the TypeScript repository), hard quotas (`min(4, cpus/2)` threads, `<= 5,000` files walked, `<= 200-500` files parsed, `<= 2s` wall-clock deadline returning `WalkState::Quit`), and cold-scan cache isolation so scans never evict the hot working set.

---

### 4. Hybrid LSP Bridge, Declaration Stubs & Static Project Resolvers

#### A. Wire Protocol & Cancellation
*   **Framing:** Single-line UTF-8 **NDJSON** (`\n`-delimited) over stdio, decoded in Lua via a stateful line accumulator with all Neovim API/LSP calls wrapped in `vim.schedule` (avoiding `E5560` libuv callback errors).
*   **Bidirectional Multiplexing:** Client requests use `"c-<n>"` IDs; engine reverse-RPC requests use `"e-<n>"` IDs. Every query carries a monotonic `query_id` and `trace_id`. Moving the cursor or jumping sends `$/cancelRequest`, canceling both Rust worker tokens and in-flight Neovim `client:cancel_request(lsp_id)`.
*   **Dirty Buffer Sync:** Neovim sends `lineage/syncBuffer { uri, changedtick, text }` lazily on query only when `b:changedtick` changed since the last sync, and `lineage/closeBuffer` on `BufUnload`/`BufWipeout` so `lineage-engine` shadows disk reads with unsaved edits without flooding `TextChanged`.
*   **Position Encoding:** `lineage-engine` standardizes on **0-based `(line, utf8_byte_col)`** using `line-index` with an ASCII fast-path (`utf16_col == byte_col`). Lua forwards each client's `client.offset_encoding` (`utf-8` | `utf-16` | `utf-32`) alongside raw LSP locations so Rust performs all coordinate math centrally. Neovim 0.10 vs. 0.11+ API drift (`client:request`, `client:supports_method`, `make_position_params(win, offset_encoding)`) lives in `lua/lineage/lsp_compat.lua`.

#### B. Budgeted Parallel LSP Resolution & Zero Synthetic `didOpen`
*   **No Synthetic `didOpen`/`didClose` and No Background `bufload`:** In Hover, reverse LSP queries fire only from already-open Neovim buffers. In Graph/Tree mode, direct workspace URI queries are attempted only on servers that answer natively without `didOpen`; all other unopened hops use disk-first Rust slicing + static project resolvers.
*   **Depth-Level Parallel Batching & Per-Client EWMA:** All unresolved sibling symbols at BFS depth `d` are sent in one `lineage/lspResolveBatch` reverse RPC. Per-request timeout is **`120-150ms`** inside the fixed **`250ms` total Phase 2 ceiling**. EWMA latency and circuit-breaker state (`closed` / `open` / `half-open` with 30s probe cooldown) are tracked **per `(lsp_client_name, workspace_root)`**, clamping cold-start spikes and treating fast empty replies (`null` in 2ms) as healthy.
*   **2-Step Call Hierarchy & `vtsls` Fallback:** Explicit "Data Flow From Here" issues `textDocument/prepareCallHierarchy` -> `callHierarchy/incomingCalls`. Per empirical findings in `refactor-neo.nvim` (ADR-0006), `vtsls` omits `item.data` on nested items and rejects `prepareCallHierarchy` on unopened files; when `prepareCallHierarchy` is unsupported or returns `null`, the bridge falls back cleanly to `textDocument/references` or the bounded Rust ripgrep+Tree-sitter pipeline.
*   **Polymorphic Dispatch Cap:** `textDocument/implementation` fan-out is capped at `max_implementations = 5`; above that, emit an expandable `PolymorphicBoundary (N implementations)` node marked `Unknown`.

#### C. Declaration-Only Outer-Wrapper Extraction, Stdlib Summaries & Static Resolvers
*   **Declaration-Only Signature Extraction + Outer-Wrapper Nullability Rule:** When LSP `definition` lands in `.d.ts`, `.pyi`, external crate `.rs`, or `.go` stubs, `lineage-engine` reads and parses *only* the target declaration signature span via Tree-sitter (never lowering external bodies, zero LSP `hover` scraping). Outer-wrapper absence is extracted without full generic instantiation (`Array<T>.find(): T | undefined`, `dict.get(): V | None`, `Iterator::next() -> Option<Self::Item>` prove `MaybeAbsent` immediately; only bare uninstantiated `T` that cannot be 1-step substituted falls back to `Unknown`). Backstopped by a declarative built-in stdlib summary table.
*   **Lightweight Static Project Resolvers:** Each adapter includes a ~200-line zero-LSP static import/barrel resolver (`tsconfig.json` `paths`/`baseUrl` + `index.ts` barrels, `pyproject.toml`/`src` + relative imports, `Cargo.toml` workspace/module tree, `go.mod` module paths), caching project build metadata once per workspace root (failures included).
*   **Deterministic Test Seam:** All external resolution sits behind a `SymbolResolver` / `LspProvider` trait with `NullLspProvider` (pure static tests) and `FixtureLspProvider` (recorded JSON fixtures + simulated latencies/timeouts for CI testing of Phase 2, EWMA, and circuit breakers).

---

### 5. Canonical `SliceEnvelope` / `FlowGraph` Schema & 5-Stage Reduction Pipeline

One versioned wire contract (`schema_version: 1`) serves Hover, Inline Focus Mode, Interactive Tree Panel, CLI outputs, and deferred v1.1/v1.2 views without schema rewrites:

```text
SliceEnvelope {
    schema_version: 1,
    query_id: u64,
    trace_id: String,
    root_ids: Vec<NodeId>,
    direction: "to_here" | "from_here" | "both",
    files: Vec<FileEntry { file_id, uri, doc_version, is_test }>,
    graph: {
        nodes: Vec<FlowNode>,   // Deduplicated canonical graph nodes
        edges: Vec<FlowEdge>,   // Directed edges + structural is_back_edge / cycle_group_id
    },
    projections?: {             // Engine-computed lightweight UI views
        hover_card?: HoverCardProjection,
        focus_spans?: Vec<FocusSpan>,
        tree_rows?: Vec<TreeRow>,
    },
    truncation: Vec<TruncationReason>,
    stats: QueryStats { phase1_ms, phase2_ms, lsp_calls, rss_bytes, cache_hits },
}
```

*   **Normalized `FlowNode` & `FlowEdge`:** Nodes carry `id`, `file_id`, `range` (0-based UTF-8 byte cols), `symbol`, optional `snippet` (controlled by `include_snippets`), `kind` (`SourceNode | ContractedChain | ClusterNode | FrontierNode`), joined post-node `state: { absence_category, domain_state: { domain_id, label, detail }, unknown_reasons }`, `possible_values`, and structured `facts` (e.g., `SinkHazard { severity, rule_id }`). Edges carry `from`, `to`, `kind`, `guard_predicate`, `narrowed_state`, `provenance` (`Static | LspEnriched | HeuristicFallback`), and structural `is_back_edge` / `cycle_group_id`. Phase 2 streams an idempotent `GraphDelta { query_id, add_files, add_nodes, add_edges, update_nodes, projection_delta, truncation_update }`.
*   **5-Stage Deterministic Reduction Pipeline (in Rust):**
    1. **Pre-budget filtering:** Exclude test files (when toggled off) and ignored paths *before* debiting caps.
    2. **Online zero-cost copy-chain contraction:** Collapse trivial state-preserving pass-through copies into `ContractedChain` so they do not burn `max_depth`.
    3. **Deterministic best-first scheduling:** Priority queue ordered by `(locality_tier, state_transition_significance, provenance_confidence, canonical(uri, line, col))` so truncated graphs never flicker across runs.
    4. **Orthogonal hard caps:** `max_depth = 5`, `max_nodes = 150`, `max_fanout_per_node = 12`, and wall-clock budget — emitting `ClusterNode` or `FrontierNode` with an opaque `expansion_token` whenever any cap fires.
    5. **Pure post-slice projections:** Compute Hover card rows, Focus extmark spans, and Tree rows (with leaf grouping and cycle references) in Rust over the finalized bounded graph.

---

### 6. Neovim UI Modes & Ergonomics

*   **Interactive Hover (`:Lineage hover` / `<Plug>(LineageHover)`):**
    *   Focuses float on open (`enter_on_open = true`, configurable) while preserving all standard Vim counts (`3j`) and motions (`j`/`k`/`/`/`n`/`N`/`f`/`t`). Default buffer-local mappings (remappable in `opts.hover.keymaps`): `<CR>` (jump under cursor), `g1`–`g9` (direct numbered jump), `gt` (To Here), `gf` (From Here), `gn` (Null/Absence-only graph), `gs` (Focus Slice), `gp` (Pin symbol), `gu` (toggle Unverified boundaries), `q`/`<Esc>` (close and restore cursor).
    *   **Line-1 Coverage Honesty:** Fixed-height verdict strip updates in place without vertical shift and never shows an unqualified `NonNull`/`Present` if any upstream path hit `Unknown`, hop limit, or timeout (e.g., `user: NonNull (partial — 2 unverified [gu])`).
    *   **Append-Only Phase 2 Stability:** Phase 1 caps visible slots at `[1]..[5]` when open cross-file boundaries exist, reserving `[6]..[9]` for Phase 2 appends without renumbering `[1]..[k]` or flipping window anchor placement. Guard all async updates with `(active_hover_id, nvim_win_is_valid, nvim_buf_is_valid)`.
    *   **Compact Trivial Peek & Jump Stack:** Trivially `Present` variables with zero origins/risks/unverified boundaries render a 1–3 line peek. Every jump executes `normal! m'` (preserving `<C-o>`) and pushes onto the Lineage Jump Stack (`:Lineage back` returns and optionally reopens the prior hover).
*   **Inline Focus Mode (`:Lineage focus` / `:Lineage pin`):** Dims unrelated lines via theme-derived highlight links (`Comment`), pairs semantic highlights with ASCII/Unicode badges (`[NULL]`, `[OK]`, `[?]`; Nerd Fonts opt-in), shows an unobtrusive active-mode indicator, and marks the slice `[stale]` on `TextChanged` (refreshing on debounced idle or manual refresh).
*   **Interactive Tree Panel (`:Lineage to` / `:Lineage from`):** Collapsible split consuming `tree_rows` from Rust, supporting To/From/Both, Group by Root Origin / Nullability, test-file scope toggle, synchronized floating code preview, and lazy expansion of `FrontierNode` / `ClusterNode` via `expansion_token`.

---

### 7. Packaging, Rollback Safety & Observability

*   **Statically Linked Grammars & Portable Builds:** Statically link Tier-1 grammars (`tree-sitter-typescript` [TS + TSX], `tree-sitter-javascript`, `tree-sitter-python`, `tree-sitter-rust`) directly into `lineage-engine`; OS virtual memory demand-pages unused parse tables lazily with zero `dlopen` ABI or macOS Gatekeeper `.dylib` hazards. Build Linux binaries against `musl` + `mimalloc` (or `cargo-zigbuild` glibc `<= 2.28`), sign macOS `arm64`/`x86_64` binaries, and stop the running sidecar before atomic rename on Windows.
*   **Version-Keyed Install & Safe Checksum Verification:** Pin release version and SHA-256 manifest inside the plugin git repository and download directly from versioned GitHub Release URLs (avoiding the 60 req/hr unauthenticated GitHub REST API limit). Verify checksums via platform CLI (`sha256sum` / `shasum -a 256` / PowerShell `Get-FileHash`) or pure LuaJIT `bit` SHA-256 — **never `vim.fn.sha256()`** (which truncates binary files at the first NUL byte). Install to `stdpath('data')/lineage/bin/lineage-engine-v<semver>` via atomic rename and enforce an `initialize` `protocol_version` handshake for instant, safe rollbacks. Fallback to `cargo build --release` when no prebuilt binary matches and Cargo + C/C++ compiler are present.
*   **Phase-0 Observability:** Propagate `trace_id` across Lua -> stdio -> Rust -> LSP proxy. Ship `:checkhealth lineage`, `:Lineage status` (PID, protocol version, OS RSS vs. cache bytes, loaded grammars, per-client EWMA, circuit-breaker state), `:Lineage log`, `:Lineage debug-dump`, and CLI `lineage-engine dump-ir` / `dump-cfg` / `slice --trace`.

## Alternatives Considered

### Alternative A: Salsa Incremental Computation Framework vs. Custom 3-Tier Byte-Weighted LRU
*   **Pros:** Battle-tested early-cutoff incrementality used in `rust-analyzer` and `ruff`; automatic dependency tracking across derived queries.
*   **Cons:** Monotonic revision memoization resists hard global byte-weighted LRU eviction (`clamp(5% host RAM, 128 MB, 300 MB)`) across heterogeneous artifacts, assumes pure deterministic inputs (conflicting with time-budgeted, EWMA-throttled reverse LSP overlays), and introduces heavy interning overhead and API churn.
*   **Reason for Rejection:** Rejected by author: a custom `moka`/`quick_cache` byte-weighted LRU holding strictly file-local parametric summaries paired with an ephemeral query arena enforces hard memory ceilings cleanly and avoids contaminating static caches with non-deterministic LSP state.

### Alternative B: Mutable CFG + On-Demand Dominator Walks vs. Pruned SSI (`Phi` + `Pi` + `Unwrap`)
*   **Pros:** Slightly simpler initial lowering pass without SSA renaming or `Pi` insertion.
*   **Cons:** Forces repeated forward/dominator walks during backward slicing, complicates branch-narrowing provenance, and cannot represent Rust `Option`/`Result` payload extraction (`if let`, `?`), destructuring, or Go `(val, err)` correlation without ad-hoc solver workarounds.
*   **Reason for Rejection:** Rejected by author: Pruned SSI with `Pi` refinement nodes, explicit projection/unwrap instructions, and bounded `Implies` guards makes backward slicing direct def-use traversal and supports all target languages uniformly.

### Alternative C: Pure Lua Prototype (`vim.treesitter` + `vim.lsp`) vs. Standalone Rust Sidecar (`lineage-engine`)
*   **Pros:** Zero cross-platform binary distribution, zero NDJSON framing, and reuses `nvim-treesitter` grammars already installed in Neovim.
*   **Cons:** Empirical benchmarks in sibling repo `refactor-neo.nvim` (ADR-0005) showed Lua workspace scans hitting 7.0–8.6s wall time and 60–287ms Neovim main-loop stalls on TypeScript even after text prefiltering and level batching. Pure Lua also cannot run non-blocking parallel ripgrep+Tree-sitter candidate verification, `mimalloc`-bounded C-heap tracking, or deterministic headless CLI snapshot/fuzz testing without Neovim.
*   **Reason for Rejection:** Rejected by author: a standalone Rust sidecar isolates all parsing and graph computation off Neovim's main loop, while vertical-slice milestones (`M0`–`M4`) mitigate the scope-overload risk raised during review.

### Alternative D: LSP `textDocument/hover` Markdown Scraping vs. Declaration-Only Outer-Wrapper Extraction
*   **Pros:** Exposes fully instantiated generic return types inferred by the language server for external libraries.
*   **Cons:** `hover` returns free-form Markdown whose format varies across 15+ LSP servers (`ts_ls`, `vtsls`, `pyright`, `basedpyright`, `ty`, `rust-analyzer`, `gopls`, `jdtls`, `clangd`, `intelephense`) and breaks silently across minor server updates.
*   **Reason for Rejection:** Rejected by author for v1: structured location endpoints + declaration-only Tree-sitter signature extraction with the outer-wrapper nullability rule + curated stdlib summaries avoid brittle regex parsers, gated on the `M1` `> 25%` real-repo `Unknown` measurement threshold.

### Alternative E: Opt-In Synthetic Raw RPC `didOpen` -> Query -> `didClose` vs. Disk-First Slicing
*   **Pros:** Allows querying LSP servers like `vtsls` or `pyright` for definitions/call hierarchy inside unopened files.
*   **Cons:** Sending raw `didOpen`/`didClose` behind Neovim's buffer manager risks corrupting active LSP document state if the user opens the file concurrently, floods `vim.diagnostic` with ghost diagnostics for unopened files, and triggers 500ms–5s cold-start server stalls.
*   **Reason for Rejection:** Rejected by author: dropped from v1 in favor of disk-first Rust slicing, dirty-buffer overlays, stateless workspace URI queries where supported, and lightweight static project resolvers (`tsconfig.json`, `pyproject.toml`, `Cargo.toml`, `go.mod`).

### Alternative F: Dynamic `dlopen` of `nvim-treesitter` `.so` Grammars vs. Statically Linked Grammars
*   **Pros:** Smaller `lineage-engine` binary size by reusing grammars installed by `nvim-treesitter`.
*   **Cons:** Exposes users to Tree-sitter ABI 13/14/15 version crashes, grammar commit skew (`QueryError::NodeType`), macOS Gatekeeper `.dylib` signing failures, and incompatibility with static Linux `musl` builds.
*   **Reason for Rejection:** Rejected by author: statically linking Tier-1 grammars into `lineage-engine` eliminates runtime ABI failures while OS virtual memory demand-paging keeps unused grammar tables out of RSS.

### Alternative G: LSP `Content-Length` Framing vs. NDJSON over Stdio
*   **Pros:** Matches standard LSP wire framing.
*   **Cons:** Chunked libuv `stdout` callbacks in Neovim Lua require stateful byte-offset header parsing that is more error-prone than newline-delimited framing when handling partial reads and multi-byte UTF-8 payloads.
*   **Reason for Rejection:** Rejected by author: single-line UTF-8 NDJSON with explicit request IDs and cancellation is simpler to accumulate and debug in both Lua and Rust.

### Alternative H: Bare Single-Letter Hover Keys (`1`–`9`, `f`, `t`, `n`, `u`) vs. Zero-Collision `g*` Keys
*   **Pros:** Single-keystroke actions inside the focused hover window.
*   **Cons:** Bare `1`–`9` hijacks Vim counts (typing `3j` teleports across files to origin `[3]`), and bare `f`/`t`/`n`/`p`/`u` collide with standard Vim motions, search, paste, and undo.
*   **Reason for Rejection:** Rejected by author: zero-collision prefixed keymaps (`<CR>`, `g1`–`g9`, `gt`/`gf`/`gn`/`gs`/`gp`/`gu`, `q`/`<Esc>`, all user-remappable) preserve native Vim navigation inside focused floats.

### Alternative I: Presentation Flags on Canonical Nodes & Lua Graph Algorithms vs. Normalized `SliceEnvelope` + Rust Projections
*   **Pros:** Slightly smaller Rust projection code initially.
*   **Cons:** Embedding `is_cycle_duplicate`, `is_danger`, and `null_state` on canonical nodes breaks DAG/Focus/Export consumers and couples the wire schema to nullable-pointer languages; filtering test files in Lua after Rust hits node/depth caps causes empty-graph budget starvation.
*   **Reason for Rejection:** Rejected by author: a normalized `SliceEnvelope` with Rust-side 5-stage deterministic reduction and view projections keeps Lua a thin renderer and ensures identical semantics in CLI and Neovim.

--------------------------------------------------------------------------------

## Non-Functional Requirements

*   **Performance & Latency:**
    *   Phase 1 Hover (static + warm cache): `< 50ms` (p95), scaling with enclosing function size (`O(function_size)`), not file size.
    *   Phase 2 Hover (LSP-enriched overlay): `250ms` hard wall-clock ceiling with `120–150ms` per-request parallel batched timeouts.
    *   Initial Tree/Graph query (depth `5`, max `150` nodes, max `12` fan-out): `< 350ms` target.
    *   Standalone ripgrep fallback: capped at `min(4, cpus/2)` threads, `5,000` files walked, `200–500` files parsed, `2s` deadline, zero Neovim main-loop stalls.
    *   Memory ceiling: `clamp(5% host RAM, 128 MB, 300 MB)` per instance, enforced via 60% logical LRU cap + 85% OS RSS eviction trigger + `mimalloc`/`ts_set_allocator`.
*   **Security & Privacy:**
    *   All analysis executes 100% locally; zero telemetry or source code leaves the user's machine.
    *   Prebuilt binary downloads verify SHA-256 checksums against a manifest pinned inside the plugin git repository (never trusting a remote checksum fetched from the same URL alone, and never using `vim.fn.sha256()` on binary bytes).
    *   Disk reads check `metadata().is_file()` and enforce a `1 MB` file size cap to prevent hangs on FIFOs/device files or OOM spikes on minified bundles.
*   **Rollout & Rollback:**
    *   Binaries install into version-keyed paths (`stdpath('data')/lineage/bin/lineage-engine-v<semver>`) via atomic rename, enabling instant rollback when reverting the plugin commit.
    *   Mandatory `initialize` `protocol_version` handshake fails fast with an actionable `:Lineage update-engine` prompt on plugin/binary skew.
    *   `SliceEnvelope` (`schema_version: 1`) follows additive-only evolution (readers ignore unknown fields; enum variants include an `Unknown` fallback).

--------------------------------------------------------------------------------

### Additional Resources

*   [concepts_brief.md](./concepts_brief.md) — Synthesized 7-lens brainstorm brief and locked architectural pivots.
*   `../refactor-neo.nvim/docs/decisions/0005-phase4-worker.md` — Empirical Lua vs. Rust workspace-scan benchmarks (TypeScript, Django, `rust-analyzer`), quoted-specifier TS prefilter, and ASCII fast-path results.
*   `../refactor-neo.nvim/docs/decisions/0006-call-hierarchy-reprepare.md` — Empirical `vtsls` / `clangd` / `rust-analyzer` / `gopls` / `basedpyright` `prepareCallHierarchy` and `item.data` probe records.

### Future Improvements

*   **Tier-2 & Tier-3 Language Adapters (v1.1 / v1.2):** Go, Java, C#, C/C++, and PHP plugged into the frozen SSI + relational guard core IR.
*   **Additional Graph Renderers (v1.1 / v1.2):** Sugiyama 2D ASCII/Unicode DAG buffer, DOT/Mermaid/interactive HTML+SVG export, and optional Kitty/Ghostty inline image preview built directly on `SliceEnvelope` v1.
*   **Power-User Workflows (v1.2):** Multi-Location Queue (combining 2+ variables to find convergence points), opt-in workspace null-risk diagnostics, shared Unix-socket multi-instance daemon (if multi-pane tmux RAM becomes a pain point), and Mason registry distribution.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Implementation is organized into five vertical milestones (`M0`–`M4`) so the TypeScript end-to-end Hover loop is dogfooded and benchmarked before expanding to Python, Rust, and additional UI modes.

### Target Files to Create/Modify

#### Milestone M0: Core IR/SSI Engine, NDJSON Protocol & CLI Harness

##### 1. `engine/Cargo.toml` (Create)
*   **Type / Scope**: Rust Workspace / Crate Manifest
*   **Proposed Changes**: Define the `lineage-engine` binary + library crate with pinned MSRV, `mimalloc` global allocator, `serde` + `serde_json` + `lsp-types` (DTOs only), `line-index`, `moka` (`sync` feature), `memory-stats`, `ignore` + `grep-searcher` + `grep-regex` + `memchr`, `tree-sitter` + statically linked Tier-1 grammars (`tree-sitter-typescript`, `tree-sitter-javascript`, `tree-sitter-python`, `tree-sitter-rust`), and `tracing` + `tracing-subscriber` + `tracing-appender`. Configure release LTO and symbol stripping.

##### 2. `engine/src/main.rs` & `engine/src/cli.rs` (Create)
*   **Type / Scope**: Binary Entrypoint & Standalone CLI
*   **Proposed Changes**: Register `mimalloc` as `#[global_allocator]` and wire `ts_set_allocator` for Tree-sitter C allocations. Route `tracing` exclusively to the rotating log file and panics to `stderr` (never `stdout`). Expose subcommands: `serve` (NDJSON stdio mode), `dump-ir <file>`, `dump-cfg --dot <file>`, and `slice <file>:<line>:<col> --direction to|from|both --format json|text|dot --trace --lsp-fixture <path>`.

##### 3. `engine/src/protocol/mod.rs` & `engine/src/protocol/schema.rs` (Create)
*   **Type / Scope**: NDJSON Wire Schema (`schema_version: 1`)
*   **Proposed Changes**: Define versioned RPC structs (`InitializeParams`/`Result` with `protocol_version`, `SliceRequest`, `SliceEnvelope`, `GraphDelta`, `SyncBufferParams`, `CloseBufferParams`, `LspResolveBatchParams`/`Result`, `CancelParams`, `StatusResult`), normalized `FlowNode` (`SourceNode | ContractedChain | ClusterNode | FrontierNode`), `FlowEdge` (with `provenance`, `is_back_edge`, `cycle_group_id`), `AbsenceCategory`, `DomainState`, `UnknownReason` taxonomy enum, `TruncationReason`, and view projection structs (`HoverCardProjection`, `FocusSpan`, `TreeRow`). Enforce additive-only serde defaults (`#[serde(default)]`).

##### 4. `engine/src/protocol/transport.rs` (Create)
*   **Type / Scope**: Full-Duplex NDJSON Stdio Router & Dual Priority Scheduler
*   **Proposed Changes**: Implement dedicated stdin reader thread, stdout writer thread, multiplexed outbound reverse-RPC table (`"e-<n>" -> oneshot::Sender`), inbound cancellation map (`query_id -> AtomicBool`), and two worker pools: a high-priority Hover lane and a bounded background Graph/Scan pool.

##### 5. `engine/src/ir/mod.rs`, `engine/src/ir/ssi.rs`, `engine/src/ir/lattice.rs`, `engine/src/ir/guards.rs` (Create)
*   **Type / Scope**: Language-Agnostic Pruned SSI IR, Bounded Relational Guards & Product Lattice
*   **Proposed Changes**: Define `FileSkeleton` (imports, exports, top-level function byte ranges + hashes), `FunctionIr` (Pruned SSI blocks, `Phi`, `Pi`, `ProjectField`, `ProjectIndex`, `UnwrapVariant`, multi-output `Call { inputs, outputs, exit_edges }`, sorted byte-span cursor index, `FnSummary`), bounded `GuardFact` (`StateIn(Place, BitMask)`, `Implies(FactA, FactB)`, `Unreachable`), and `AbsenceLattice` trait over 2-axis `LatticeValue { known_states, unknown_reasons }`.

##### 6. `engine/src/cache/mod.rs` & `engine/src/cache/memory.rs` (Create)
*   **Type / Scope**: 3-Tier Byte-Weighted Cache & OS RSS Guard
*   **Proposed Changes**: Implement `FileIrCache` (Tier 1 byte-weighted LRU via `moka` storing file skeletons, lazily lowered function IR, and strictly file-local parametric `FnSummary`, validated via `stat()` for unopened disk files and `changedtick`/hash for dirty buffers), `OverlayCache` (Tier 2 symbol-anchored stale-while-revalidate LSP cache), and `SliceArena` (Tier 3 ephemeral query workspace). Implement `clamp(5% host RAM, 128 MB, 300 MB)` budget calculation, pre-parse guards (`metadata().is_file()`, `<= 1 MB`, max line `<= 4096` bytes), and periodic `memory-stats` RSS sampling triggering eviction above 85%.

##### 7. `engine/src/resolver/mod.rs`, `engine/src/resolver/lsp_provider.rs`, `engine/src/resolver/ewma.rs` (Create)
*   **Type / Scope**: Transport-Agnostic `SymbolResolver`, Per-Client EWMA & Circuit Breaker
*   **Proposed Changes**: Define `SymbolResolver` and `LspProvider` traits (`definition`, `type_definition`, `implementation`, `prepare_call_hierarchy`, `incoming_calls`, `references`), with `StdioLspProvider` (depth-level `lspResolveBatch`, `120–150ms` per-request timeout inside `250ms` Phase 2), `NullLspProvider`, and `FixtureLspProvider` (JSON fixtures + simulated latency/timeouts). Implement per-`(client_name, workspace_root)` EWMA with cold-start clamping and `closed`/`open`/`half-open` (30s cooldown) circuit breaker.

##### 8. `engine/src/slicer/mod.rs`, `engine/src/slicer/scc.rs`, `engine/src/slicer/pipeline.rs`, `engine/src/slicer/projectors.rs` (Create)
*   **Type / Scope**: Demand-Driven Slicer, SCC Fixed-Point Solver & View Projectors
*   **Proposed Changes**: Implement backward (`to_here`) and forward (`from_here`) demand-driven traversal, Tarjan SCC + bounded monotone worklist iteration (cap 8 iterations -> widen to `Unknown`), the 5-stage deterministic reduction pipeline (pre-budget test filtering, zero-cost copy-chain contraction, deterministic best-first priority queue, orthogonal `max_depth=5`/`max_nodes=150`/`max_fanout=12` caps emitting `ClusterNode`/`FrontierNode` with `expansion_token`), and pure Rust projectors for `hover_card`, `focus_spans`, and `tree_rows`.

##### 9. `engine/src/text/position.rs` (Create)
*   **Type / Scope**: UTF-8/UTF-16/UTF-32 Coordinate Index
*   **Proposed Changes**: Wrap `line-index` for O(log N) conversion between canonical 0-based `(line, utf8_byte_col)` and per-client LSP `offset_encoding` (`utf-8`, `utf-16`, `utf-32`), with an ASCII-only fast-path skipping UTF-16 scans.

---

#### Milestone M1: TypeScript/TSX/JS End-to-End Hover Dogfood & Measurement Gate

##### 10. `engine/src/adapters/mod.rs` & `engine/src/adapters/typescript.rs` (Create)
*   **Type / Scope**: `LanguageAdapter` Trait & TypeScript/TSX/JS Adapter
*   **Proposed Changes**: Define `LanguageAdapter` (`parse_skeleton`, `lower_function`, `extract_declaration_signature`, `resolve_static_import`, `stdlib_summary`, `lattice`, `classify_test_file`). Implement TS/TSX/JS lowering (both `LANGUAGE_TYPESCRIPT` and `LANGUAGE_TSX` grammars): `null` vs. `undefined`, truthiness asymmetry, `?.`, `??`, `??=`, `!`, discriminated unions (`Implies`), type predicates (`x is T`), `async`/`await`, shallow 1-level `obj.field` kill/gen rules, closures, `ERROR`/`MISSING` recovery, declaration-only `.d.ts` outer-wrapper signature extraction, curated JS/TS stdlib summary table (`Array.find`, `Map.get`, `RegExp.exec`, `querySelector`), and lightweight `tsconfig.json` (`paths`/`baseUrl`) + `index.ts` barrel static resolver.

##### 11. `plugin/lineage.lua` & `lua/lineage/init.lua` & `lua/lineage/config.lua` (Create)
*   **Type / Scope**: Neovim Plugin Entrypoint, Commands & Configuration
*   **Proposed Changes**: Register `:Lineage` command dispatcher (`hover`, `to`, `from`, `focus`, `pin`, `back`, `clear`, `status`, `log`, `debug-dump`, `update-engine`) and `<Plug>(LineageHover)` / `<Plug>(LineageTo)` / `<Plug>(LineageFrom)` / `<Plug>(LineageFocus)` / `<Plug>(LineageBack)` without silently overwriting user `<leader>l*` mappings. Define `setup(opts)` defaults including `enter_on_open = true`, remappable `opts.hover.keymaps` (`<CR>`, `g1`–`g9`, `gt`, `gf`, `gn`, `gs`, `gp`, `gu`, `q`/`<Esc>`), memory cap, and `build_from_source = false`.

##### 12. `lua/lineage/client.lua` & `lua/lineage/lsp_compat.lua` & `lua/lineage/sync.lua` (Create)
*   **Type / Scope**: Sidecar Process Lifecycle, NDJSON Client, LSP Proxy & Dirty Buffer Sync
*   **Proposed Changes**: Spawn `lineage-engine serve` via `vim.uv.spawn`/`vim.system`, perform `initialize` `protocol_version` handshake, accumulate chunked `stdout` into NDJSON lines, dispatch callbacks inside `vim.schedule`, capture `stderr` ring buffer, and propagate `$/cancelRequest`. Implement `lsp_compat.lua` abstracting Neovim 0.10 vs. 0.11+ (`vim.lsp.get_clients`, `client:supports_method`, `client:request`, `client:cancel_request`, `offset_encoding`) for `definition`, `typeDefinition`, `implementation` (fan-out cap `5`), and `prepareCallHierarchy` -> `incomingCalls` / `references` fallback. Implement lazy `b:changedtick` dirty-buffer sync (`lineage/syncBuffer`, `lineage/closeBuffer`) without ever calling `bufload`/`:badd`/`:edit` or synthetic `didOpen`/`didClose`.

##### 13. `lua/lineage/ui/hover.lua` & `lua/lineage/ui/jump_stack.lua` & `lua/lineage/ui/highlights.lua` (Create)
*   **Type / Scope**: Interactive Two-Phase Hover, Jump Stack & Theme Highlights
*   **Proposed Changes**: Render `HoverCardProjection` with fixed-height Line-1 coverage honesty qualifiers (`NonNull (partial — N unverified [gu])`), compact 1–3 line trivial-variable peek, partitioned Phase 1 (`[1]..[5]`) + Phase 2 (`[6]..[9]`) append-only slots guarded by `(active_hover_id, nvim_win_is_valid, nvim_buf_is_valid)` without flipping float anchor placement, and zero-collision `g*` keymaps. Push every jump to Neovim's native jumplist (`normal! m'`) and maintain the Lineage Jump Stack (`:Lineage back` with optional hover reopen). Link highlights to standard Neovim groups (`Comment`, `DiagnosticWarn`, `DiagnosticError`, `DiagnosticOk`) paired with ASCII/Unicode badges (`[NULL]`, `[OK]`, `[?]`).

##### 14. `lua/lineage/health.lua` (Create)
*   **Type / Scope**: `:checkhealth lineage` Diagnostics
*   **Proposed Changes**: Verify Neovim `>= 0.10`, binary presence and SHA-256/protocol match, sidecar ping latency, attached LSP clients + `offset_encoding` + method capabilities, loaded grammars, live OS RSS vs. cap, per-client EWMA + circuit-breaker state, and log file path.

---

#### Milestone M2: Python & Rust Adapters + Go `(val, err)` IR Validation

##### 15. `engine/src/adapters/python.rs` (Create)
*   **Type / Scope**: Python `LanguageAdapter`
*   **Proposed Changes**: Lower Python AST (`tree-sitter-python`) into Pruned SSI: `None`, truthiness asymmetry, `is None` / `is not None`, `assert x is not None`, walrus `:=`, default `= None`, comprehensions, `raise` reachability, shallow `obj.field`, `.pyi` outer-wrapper signature extraction, curated stdlib table (`dict.get`, `re.search`, `next`), and `pyproject.toml`/`src` + relative import static resolver.

##### 16. `engine/src/adapters/rust.rs` (Create)
*   **Type / Scope**: Rust `LanguageAdapter`
*   **Proposed Changes**: Lower Rust AST (`tree-sitter-rust`) into Pruned SSI: `Option<T>` / `Result<T, E>` payload extraction (`if let`, `match`, `let ... else`) linked through `UnwrapVariant` edges, `?` early-return + survivor unwrap, `.unwrap()`/`.expect()` danger sinks, `&self`/`&T` read-only call preservation vs. `&mut` invalidation, stdlib combinators (`.map`, `.and_then`, `.unwrap_or`, `.ok_or`, `HashMap::get`, `slice::get`), `unsafe` raw pointers (`Unknown`), and `Cargo.toml` workspace/`mod`/`crate::` static resolver (caching metadata once per workspace root).

##### 17. `engine/tests/ir_go_validation.rs` (Create)
*   **Type / Scope**: Go `(val, err)` Core IR Specification Test
*   **Proposed Changes**: Unit tests constructing core IR graphs for Go zero-value pointers, correlated `val, err := fn(); if err != nil { return }; use(val)` via `Implies`, comma-ok map lookups, and `defer`-mutated named returns to prove zero core IR changes are needed for Tier 2.

---

#### Milestone M3: Inline Focus Mode, Interactive Tree Panel & Ripgrep Fallback

##### 18. `engine/src/resolver/ripgrep.rs` (Create)
*   **Type / Scope**: Bounded Standalone Caller Discovery
*   **Proposed Changes**: Implement parallel candidate search via `ignore::WalkParallel` + word-boundary matching (`grep-regex` / boundary-checked `memchr::memmem`) with language-extension filters, quoted-specifier TS/JS prefiltering, hard quotas (`min(4, cpus/2)` threads, `<= 5,000` files walked, `<= 200–500` files parsed, `<= 2s` deadline returning `WalkState::Quit`), Tree-sitter call-site verification, cold-scan cache isolation, and partial truncation telemetry.

##### 19. `lua/lineage/ui/focus.lua` (Create)
*   **Type / Scope**: Inline Focus Mode & Cross-Buffer Pinning
*   **Proposed Changes**: Render `focus_spans` via Neovim extmarks (dimming unrelated lines with `Comment`-linked highlights, tagging flow lines with configurable virtual text), display an unobtrusive active-mode/pin indicator, mark slices `[stale]` immediately on `TextChanged`, and refresh on debounced idle or explicit command.

##### 20. `lua/lineage/ui/tree.lua` (Create)
*   **Type / Scope**: Interactive Tree Panel
*   **Proposed Changes**: Render `tree_rows` in a collapsible split supporting `To Here` / `From Here` / `Both`, grouping toggles (`Group by Root Origin` / `Group by Nullability`), null/absence filtering, pre-budget test-file toggle, cycle badges, synchronized floating code preview, `[partial]` truncation badges, and lazy expansion of `FrontierNode`/`ClusterNode` via `expansion_token`.

---

#### Milestone M4: Release Packaging, Tests & Documentation

##### 21. `lua/lineage/install.lua` & `checksums.json` (Create)
*   **Type / Scope**: Prebuilt Binary Downloader, Verifier & Cargo Fallback
*   **Proposed Changes**: Pin exact release version and SHA-256 digests in `checksums.json` inside the repo. Download directly from versioned GitHub Release URLs (no GitHub REST API calls), verify digests via platform CLI (`sha256sum` / `shasum -a 256` / PowerShell `Get-FileHash`) or pure LuaJIT `bit` SHA-256 (never `vim.fn.sha256()`), stop any running sidecar before atomic rename into `stdpath('data')/lineage/bin/lineage-engine-v<semver>`, and fall back to `cargo build --release` when no prebuilt target matches and Cargo + C/C++ compiler exist.

##### 22. `.github/workflows/ci.yml` & `.github/workflows/release.yml` (Create)
*   **Type / Scope**: CI Matrix & Cross-Platform Release Pipeline
*   **Proposed Changes**: Run `cargo test`, `cargo clippy -- -D warnings`, golden/fuzz tests, `stylua --check`, and headless Neovim tests on both `0.10.x` and `0.11.x` / `nightly`. On version tags, build static Linux `musl` (`x86_64`, `aarch64` with `mimalloc`) and macOS (`arm64`, `x86_64`) release binaries and generate `checksums.json`.

##### 23. `engine/tests/golden/*.rs`, `engine/tests/fuzz_broken_syntax.rs`, `tests/minimal_init.lua`, `tests/test_*.lua` (Create)
*   **Type / Scope**: Golden Snapshot, Fuzz & Headless Neovim Integration Tests
*   **Proposed Changes**: 40–60 inline-cursor golden test cases per Tier-1 language (TS/TSX/JS, Python, Rust), `FixtureLspProvider` tests for Phase 1/Phase 2 append-only invariants, EWMA decay, circuit breaker recovery, and `vtsls` call-hierarchy fallback, malformed-syntax termination fuzzing, and headless Neovim tests for Hover keymaps, jump stack, Focus Mode `[stale]` transitions, Tree expansion, and `lsp_compat.lua`.

##### 24. `README.md` & `doc/lineage.txt` (Create)
*   **Type / Scope**: User Documentation & Vim Help Tags
*   **Proposed Changes**: Document installation (`lazy.nvim` build hook / Cargo fallback), `:Lineage` commands, `<Plug>` mappings, hover keymaps, coverage-honesty indicators, troubleshooting via `:checkhealth lineage` / `:Lineage status`, and explicit out-of-scope boundaries.

---

### Verification Plan

*   **Compilation & Build:** `cargo build --release --manifest-path engine/Cargo.toml` (plus cross-target `musl` release check in CI).
*   **Unit & Golden Snapshot Tests:** `cargo test --manifest-path engine/Cargo.toml` — must pass 100% of IR/SSI, 2-axis lattice (`Unknown != Absent`), SCC fixed-point termination, UTF-8/16/32 `line-index`, `FixtureLspProvider` (EWMA, circuit breaker, Phase 1/2 delta merge), Go `(val, err)` spec tests, and 40–60 golden snippets per Tier-1 language.
*   **Static Analysis & Linting:** `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `stylua --check lua/ plugin/ tests/`.
*   **Fuzz / Resilience Verification:** Feed truncated/mutated TS/Python/Rust source files through `parse_skeleton` + `lower_function` + `slice` verifying zero panics, zero infinite loops, and deterministic `tree_sitter_error_node` boundaries.
*   **Headless Neovim E2E Matrix:** Run `nvim --headless -u tests/minimal_init.lua` across both Neovim `0.10.x` and `0.11.x` verifying NDJSON handshake, `E5560`-free async scheduling, Hover `g*` keymaps + Line-1 qualifiers, Focus Mode `[stale]` lifecycle, Tree lazy expansion, `:checkhealth lineage`, and checksum verification.
*   **M1 Real-Repo Measurement Gate (Before M2):** Benchmark `lineage-engine` on 3–5 real TypeScript repositories (including `zod` and `TypeScript`) measuring: (a) Phase 1 p95 latency `< 50ms`, (b) Phase 2 p95 latency `<= 250ms`, (c) steady-state OS RSS within `clamp(5% RAM, 128 MB, 300 MB)`, (d) false-positive rate on proven null origins (`target: ~0%`), and (e) external generic call `Unknown` rate (must be `<= 25%` to keep zero LSP hover scraping unchanged into M2).

--------------------------------------------------------------------------------

*Drafted with EGM*
