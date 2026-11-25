## Context

`lineage.nvim` provides Two-Phase Hover, Inline Focus Mode, and an Interactive Tree Panel across TypeScript/TSX/JavaScript, Python, Rust, and Go, backed by the standalone Rust sidecar (`lineage-engine`). However, all editor views are ephemeral, the Tree Panel repeats shared subtrees on converging diamonds, and the existing `lineage-engine slice --format dot` stub omits Phase 2 LSP resolution, escapes labels improperly, and sets `workspace_root` to `file.parent()`.

Auditing `engine/src/slicer/` and `lua/lineage/` also surfaced concrete `SliceGraph` producer gaps that must be closed before any graph exporter (Stage C.1 Mermaid/DOT or Stage C.2 Offline Interactive HTML+SVG Viewer) can rely on `SliceEnvelope.graph`: `execute_phase2` discards Phase 2 graph merges, `record_inst_node` drops `ProjectField`/`ProjectIndex`/`UnwrapVariant` edges and cross-file `To` callees, `scc::tarjan_scc().back_edges` marks every edge inside a multi-node SCC instead of only DFS feedback back-edges (`C1-1`), `FlowEdge` lacks an upstream/downstream discriminator for `Both` slices (`C1-3`), and Hover/Tree keep `include_graph = false` on the wire.

Constraints:
- Preserve `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` via additive `#[serde(default)]` fields only.
- Zero new Cargo dependencies; `< 200 KB` release binary delta; Rust 1.82 MSRV preserved (`cargo check --features mimalloc`).
- 100% offline, deterministic, byte-identical output between CLI and Neovim RPC (`BTreeMap`/sorted `Vec` only, stripping volatile runtime fields `query_id`, `trace_id`, `phase1_ms`, `phase2_ms`).
- Freeze clean Stage C.1 -> C.2 extension seams (`C1-1`–`C1-6`) so Stage C.2 plugs `layout.rs`, `html.rs`, and `export_cache.lua` directly onto `ExportGraph` and `lineage/export` with zero rework.

## Goals / Non-Goals

**Goals:**
- Close Block 0 `SliceGraph` producer defects (`schema.rs`, `cache/memory.rs`, `slicer/{mod,pipeline,projectors}.rs`) including Phase 2 graph merging, cross-file `To` callees, `ProjectField`/`ProjectIndex`/`UnwrapVariant` edges, separated SCC `cycle_group_id` vs. deterministic 3-color DFS `is_back_edge`, typed guard/danger metadata, `FlowEdge.edge_side`, and percent-decoded/Windows-normalized `uri_to_path`.
- Maintain a 4-slot `QuerySnapshotCache` LRU in `TransportState` (keeping `include_graph = false` on Hover/Tree wire hot paths) updated in-place by `execute_phase2` and collision-free `expand_node` (`exp{k}_`), paired with an additive `lineage/export` RPC dispatching fresh Phase 2 settles onto the dedicated Phase 2 worker pool (zero Phase 1 starvation, configurable `lsp_timeout_ms = 500`, zero wait when `phase2_opt` is `None`).
- Build a shared pure-Rust `engine/src/export/` pipeline (`ir.rs`, `sanitize.rs`, `mermaid.rs`, `dot.rs`) with 2-tier path privacy (`<workspace-rel>` vs. `<external>/<sanitized-tail>`), centralized UTF-8 char-safe `<= 120`-char snippet trimming/scrubbing, full inspector metadata on `ExportGraph`, `max_nodes = 200` / `max_edges = 800` / `max_label_chars = 256` BFS pruning, flat file-pill GitHub-flavored Mermaid (45 KB guard), and Graphviz DOT unified with `slice --format dot`.
- Split `SliceCliFormat` and `ExportCliFormat`, add `lineage-engine export` (`Both`, `--workspace-root`, `--from-envelope`, tri-state `--snippets`/`--no-snippets`), upgrade `:Lineage` to `nargs = "*"` (`lua/lineage/commands.lua`, `lua/lineage/export.lua`), add Tree `e` / Hover `ge` / `<Plug>(LineageExport{,Mermaid,Dot})`, harden `lua/lineage/client.lua` (chunk-array NDJSON buffer, decode-error callback rejection, capability reset), and format `lineage/export` telemetry in `:Lineage status`.

**Non-Goals:**
- Stage C.2 Offline Interactive HTML+SVG Viewer (`layout.rs`, `html.rs`, `assets/`, `export_cache.lua`) — handled in the follow-on change `lineage-stage-c2-html-svg-viewer` once Stage C.1 freezes.
- 2D ASCII/Unicode Sugiyama DAG terminal buffers (permanently dropped).
- 3-tier `<stdlib>/...` path heuristics, external network/CDN calls, mandatory Graphviz runtime, or breaking wire/schema versions.

## Decisions

1. **Block 0 `SliceGraph` Producer Hardening + Separated Cycle Classification (`C1-1` & `C1-3`):**
   - *Choice:* Fix `SliceGraph` producers in `slicer/{mod,pipeline,projectors}.rs` and `cache/memory.rs` first. Separate `cycle_group_id: Option<u32>` (from `scc::tarjan_scc`) from `is_back_edge: bool` (single deterministic 3-color DFS feedback-arc pass in canonical order, including self-loops `u == v`) so forward cycle edges stay top-to-bottom across Mermaid, DOT, and Stage C.2 HTML while removing `is_back_edge` always leaves a DAG. Tag upstream vs. downstream edges with additive `FlowEdge.edge_side: Option<EdgeSide>` (`Upstream | Downstream`) so `Both` slices normalize reliably without guessing from free-form `provenance`.
   - *Alternatives rejected:* Exporting raw `SliceEnvelope.graph` as-is (drops Phase 2 edges, projections, and cross-file `To` callees); using `tarjan_scc().back_edges` directly (marks every edge in a multi-node SCC as a back-edge, collapsing cycles in Stage C.2).

2. **4-Slot `QuerySnapshotCache` LRU + Phase 2 Worker-Pool `lineage/export` RPC:**
   - *Choice:* Keep `include_graph = false` on Hover/Tree wire responses while caching up to 4 materialized snapshots keyed by `query_id` in `TransportState`, updated in-place by `execute_phase2` and `expand_node` (`exp{k}_` namespaced IDs + remapped `file_id`s). Dispatch fresh `lineage/export` Phase 2 settles onto the Phase 2 pool (`phase2_ceiling_ms = lsp_timeout_ms`, propagating per-call LSP timeout up to ceiling) so Phase 1 workers never starve; handle `InFlight` LRU hits cleanly without duplicate LSP calls or dropped frontiers.
   - *Alternatives rejected:* Re-slicing from active UI parameters (discards lazily expanded `FrontierNode` subtrees); client-side 500ms polling on `lineage/slice` or blocking Phase 1 workers (stalls local-only exports and starves Hover on 2-core hosts).

3. **Canonical `ExportGraph` IR (`C1-2`, `C1-4`) + 2-Tier Path Sanitization + Flat-Pill Mermaid:**
   - *Choice:* Type `ExportParams.include_snippets` as tri-state `Option<bool>` (`None => false` uniformly across formats; explicit `true`/`false` overrides). Centralize UTF-8 char-safe `<= 120`-char snippet trimming and `PathSanitizer::scrub_text` inside `ir.rs`; retain full inspector/legend fields (`file_index`, `range`, `unknown_reasons`, scrubbed `guard_text`, `danger_kind`, scrubbed `risk_messages`, `provenance`, `unknown_boundaries`, `completeness_badges`, export filter flags `null_only`/`exclude_tests`) and enforce `max_nodes = 200`, `max_edges = 800`, `max_label_chars = 256` so both C.1 text exporters and C.2 HTML share one IR. Emit flat Mermaid `flowchart TD` nodes with per-node `file:line` pills (45 KB guard below GitHub's 50 KB ceiling), keep `subgraph cluster_f<idx>` grouping for Graphviz DOT, and use 2-tier `<workspace-rel>` vs. `<external>/<sanitized-tail>` sanitization.
   - *Alternatives rejected:* Conditional Mermaid `subgraph` file grouping (GitHub Dagre inversions and header-escaping hazards); 3-tier `<stdlib>/...` classifier (brittle heuristics across 4 languages and OSes); plain `bool` `include_snippets` (breaks C.2 format-default negotiation).

4. **CLI Enum Split (`C1-5`), Collision-Free Keymaps & Lua Transport Hardening (`C1-6`):**
   - *Choice:* Split `CliFormat` into `SliceCliFormat { Json, Text, Dot }` and `ExportCliFormat { Mermaid, Dot }`. Bind `e` in Tree Panel (avoiding `nowait` `g` collision) and `ge` in Hover, intersecting `capabilities.export_formats` with Lua-supported formats. Standardize symmetric `--no-lsp` and `--snippets`/`--no-snippets` flags. Replace quadratic `state.buf = state.buf .. data` in `lua/lineage/client.lua` with a chunk-array accumulator (`table.concat` on `"\n"`), reject pending callbacks on JSON decode failure, clear `capabilities` on stop/exit, gate capability checks inside `ensure_started`, and update `lua/lineage/debug.lua:show_status` for `lineage/export`.
   - *Alternatives rejected:* Shared `CliFormat` (allows invalid `export --format json`); `ge` in Tree Panel (collides with `nowait` `g`); asymmetric `--include-snippets`/`--redact-snippets`/`static` flags.

## Risks / Trade-offs

- **Pre-Stage-C engine binaries lacking `export_formats`** → `lua/lineage/export.lua` inspects `capabilities.export_formats` (exact membership via `vim.tbl_contains`) inside `ensure_started` and catches `-32601 Method not found`, failing fast with `:Lineage install!` guidance.
- **Stage C.1 Lua paired with a future Stage C.2 engine advertising `"html"`** → Intersecting `capabilities.export_formats` with Lua-supported delivery handlers in the `e`/`ge` picker prevents C.1 Lua from copying raw HTML to clipboard.
- **GitHub Mermaid 50 KB `maxTextSize` truncation** → Enforce a 45 KB ceiling in `mermaid.rs` (dropping optional snippets first, then re-pruning via BFS priority with explicit warning and header comment).
- **Phase 2 queue saturation under rapid export bursts** → Handle `phase2_tx.try_send` full error by logging `tracing::warn!(reason = "phase2_queue_full", ...)` and falling back immediately to Phase 1 stamped `Static only (LSP queue full)`.

## Migration Plan

1. Implement Block 0 `SliceGraph` producer hardening (`schema.rs`, `cache/memory.rs`, `slicer/{mod,pipeline,projectors}.rs`) and confirm all existing Rust golden/nullability tests pass.
2. Implement `engine/src/export/{mod,ir,sanitize,mermaid,dot}.rs`, 4-slot `QuerySnapshotCache` LRU + `lineage/export` in `transport.rs`, and unified CLI in `cli.rs`/`main.rs`, passing `engine/tests/export_golden.rs` and `< 200 KB` binary delta check.
3. Implement Lua transport hardening (`client.lua`, `debug.lua`), `commands.lua`, `export.lua`, `config.lua`, `plugin/lineage.lua`, `tree.lua`, and `hover.lua`, passing `tests/test_export.lua` and all existing Neovim 0.10/0.11 E2E suites plus `stylua --check`.
4. Update `README.md` (fixing stale Go adapter note) and `doc/lineage.txt`, perform GitHub Markdown visual sign-off on 4 golden Mermaid diagrams, and freeze Stage C.1 before implementing `lineage-stage-c2-html-svg-viewer`.

## Open Questions

None — all Stage C.1 decisions and Stage C.2 pre-freeze folds (`C1-1`–`C1-6`) are resolved.
