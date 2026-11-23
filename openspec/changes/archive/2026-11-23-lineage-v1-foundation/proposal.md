## Why

Neovim's built-in LSP workflows (`definition`, `references`, `callHierarchy`, `hover`) show where symbols appear and their static types at a point, but do not trace which specific assignments, branches, guards, or cross-file returns can produce a `null` or `undefined` value at the cursor. `lineage.nvim` + `lineage-engine` addresses this with demand-driven, flow-sensitive interprocedural dataflow analysis that prioritizes high precision, explicit partial-coverage indicators, and `< 50ms` (p95) Phase 1 local hover without eager repository indexing.

## What Changes

- Scope this foundation change strictly to **Milestones M0 + M1** (core engine, NDJSON transport, TypeScript/TSX/JS adapter, Interactive Two-Phase Hover, observability/CLI debug tools, and the M1 empirical measurement gate). Defer M2–M4 (Python/Rust/Go adapters, Inline Focus Mode, Tree Panel, ripgrep caller discovery, and cross-platform prebuilt release packaging) to follow-on OpenSpec changes once M1 validates the external-generic `Unknown` gate (`<= 25%`).
- Introduce standalone Rust sidecar binary **`lineage-engine`** communicating with Neovim 0.10+ over full-duplex NDJSON stdio with `stdout` isolation and cancellation propagation.
- Implement a **Pruned SSI + bounded relational guard core IR** paired with a **2-axis product absence lattice** keeping `Unknown` structurally separate from `Absent`, extensible to future language adapters and graph views without core rewrites.
- Implement **two-granularity lazy parsing** (file skeleton + function-scoped SSI lowering with query-scoped AST disposal), a **byte-weighted LRU cache** (`clamp(5% host RAM, 128 MB, 300 MB)`) with a bounded dirty-buffer overlay and pre-parse size/line guards, and a **5-stage deterministic reduction pipeline** emitting versioned `SliceEnvelope` + `GraphDelta` payloads.
- Ship the first **`LanguageAdapter`** for **TypeScript/TSX/JavaScript** with statically linked grammars, declaration-only outer-wrapper signature extraction, a built-in JS/TS stdlib summary table, and a bounded static `tsconfig.json`/barrel resolver.
- Build a **Hybrid LSP Bridge** restricted to structured location endpoints (`definition`, `typeDefinition`, `implementation`) with zero hover Markdown scraping, zero synthetic `didOpen`/`didClose`, depth-level parallel batching inside the `250ms` Phase 2 ceiling, per-client EWMA + circuit breaker, dirty-buffer sync, and Neovim 0.10/0.11 compatibility.
- Deliver the flagship **Interactive Two-Phase Hover** in **`lineage.nvim`** (`:Lineage` namespace, `<Plug>` mappings, prefixed `g*` keymaps, Line-1 partial-coverage qualifiers, compact peek for simple present values, and native jumplist + Lineage Jump Stack).
- Provide **M0 observability and CLI tools**: `:checkhealth lineage`, `:Lineage status`/`log`/`debug-dump`, and standalone CLI `dump-ir`/`dump-cfg`/`slice --trace`.

## Capabilities

### New Capabilities
- `core-ir-and-slicer`: Pruned SSI + bounded relational guard IR, 2-axis product absence lattice (`Unknown` separate from `Absent`), two-granularity lazy parsing with query-scoped AST disposal, bounded LRU cache + dirty-buffer overlay + pre-parse/path guards, SCC fixed-point termination, and 5-stage deterministic reduction pipeline.
- `language-adapters`: Pluggable `LanguageAdapter` contract and TypeScript/TSX/JS implementation with statically linked grammars, declaration-only outer-wrapper signature extraction, built-in JS/TS stdlib summaries, and bounded static `tsconfig.json`/barrel resolution.
- `hybrid-lsp-protocol`: Full-duplex NDJSON stdio transport, versioned `SliceEnvelope`/`GraphDelta` wire contract, parallel batched location-only LSP bridge, per-client EWMA + circuit breaker, sidecar crash recovery, dirty-buffer sync, UTF-8/16/32 encoding translation, and Neovim 0.10/0.11 compatibility.
- `neovim-ui`: Interactive Two-Phase Hover with prefixed `g*` keymaps, Line-1 partial-coverage qualifiers, compact simple-value peek, and native jumplist + Lineage Jump Stack.
- `observability-and-cli`: Startup protocol-version handshake, `trace_id` propagation, `:checkhealth lineage`, `:Lineage status`/`log`/`debug-dump`, source-free rotating logs, and standalone CLI debug/trace commands.

### Modified Capabilities
<!-- None: greenfield repository with no existing specs under openspec/specs/. -->

## Impact

- **Reference Design:** Anchored in [docs/designs/lineage-engine-v1.md](../../../docs/designs/lineage-engine-v1.md) and [concepts_brief.md](../../../concepts_brief.md).
- **New Codebases:** Rust crate `engine/` (`lineage-engine`) and Neovim Lua plugin `plugin/` + `lua/lineage/` (`lineage.nvim`).
- **Deferred Follow-On Changes:** M2 (Python + Rust adapters + Go `(val, err)` IR validation), M3 (Inline Focus Mode, Interactive Tree Panel, bounded Rust ripgrep fallback, 2-step `prepareCallHierarchy`), and M4 (cross-platform prebuilt release binaries, `checksums.json`, CI release workflow).
- **M1 Measurement Gate:** Before starting M2, measure `unknown_external_generic_calls / total_external_generic_calls <= 25%`, false-positive rate `<= 1%`, Phase 1 p95 `< 50ms`, Phase 2 p95 `<= 250ms`, and steady-state RSS on `zod`, `trpc`, and `TypeScript`.
