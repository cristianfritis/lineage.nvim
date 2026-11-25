# lineage.nvim Stage C.1: SliceGraph Hardening, Shared Export IR, and Offline Text Exporters (Mermaid & Graphviz DOT)

*   **Author:** cristianfritis
*   **Status:** Proposed
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

`lineage.nvim` currently helps developers trace nullability and dataflow inside Neovim across TypeScript/TSX/JavaScript, Python, Rust, and Go via Two-Phase Hover, Inline Focus Mode, and the Interactive Tree Panel. However, all three views are ephemeral and confined to the editor, and the Tree Panel duplicates shared subtrees whenever multiple dataflow paths converge.

Stage C.1 solves two linked gaps before introducing an offline browser viewer in Stage C.2. First, it closes concrete producer defects in `SliceGraph` so the engine's canonical graph accurately captures Phase 2 LSP resolutions, cross-file `To` callees, field/index/unwrap edges, cycle back-edges, and typed guard/danger metadata without breaking `SCHEMA_VERSION = 1` or `PROTOCOL_VERSION = 1`. Second, it introduces a shared, deterministic, 100% offline export pipeline (`engine/src/export/` and `lineage/export`) delivering **Mermaid** and **Graphviz DOT** diagrams from both Neovim and the standalone CLI with byte-identical output and workspace-relative path privacy.

## The Problem

Two real user workflows fail in `lineage.nvim` today, and a third underlying engine gap currently blocks any accurate graph exporter from shipping on top of `SliceEnvelope`:

1. **Convergence and Diamond Topologies Are Hard to Read in a Tree:**
   When two distinct `null`/`None`/`nil` producers flow into a shared helper or downstream variable, the Interactive Tree Panel repeats the shared downstream or upstream subtree under every branch. Developers cannot see at a glance where paths merge, which guards cover all branches, or which cycles loop back on themselves.
2. **Slices Cannot Be Shared or Persisted Outside Neovim:**
   Hover, Focus Mode, and the Tree Panel disappear when closed. Developers have no built-in way to paste a verified null-propagation chain into a GitHub pull request review, bug report, or architecture document, nor a deterministic Graphviz DOT export for CI/CLI tooling. The existing `lineage-engine slice --format dot` stub in `engine/src/main.rs` emits raw unsanitized labels, mishandles backslash escaping, omits `both` direction, ignores `--lsp-fixture` Phase 2 resolution, and sets `workspace_root` to the file's parent directory instead of the project root.
3. **The Underlying `SliceGraph` Is Currently Incomplete for Exports:**
   Audit of `engine/src/slicer/` and `lua/lineage/` shows that exporting `SliceEnvelope.graph` as-is would silently mislead users:
   * `execute_phase2` updates Hover and Tree projections with LSP results, but returns the unmodified Phase 1 `pipeline_graph`.
   * `record_inst_node` hardcodes `file_id: 0` and `is_back_edge: false` (`tarjan_scc` is never called), omits cross-file callee nodes in `To` slices, and drops edges for `ProjectField`, `ProjectIndex`, and `UnwrapVariant`.
   * `To` edges point from consumer to origin while `From` edges point from callee to caller, producing inverted or conflicting arrow directions when rendered directly.
   * Danger sinks exist only in `hover_card.local_risks` (with no `node_id`), and guards are inferred by matching `"narrow "` string prefixes.
   * Hover and Tree both request `include_graph = false` and discard `SliceEnvelope`, while `lineage/expand` returns only `tree_rows` with colliding `n0`/`n1` IDs—making live Tree/Hover snapshot export impossible without engine-side snapshot tracking.

Stage C.1 resolves the `SliceGraph` producer gaps as a prerequisite block (Block 0), establishes an engine-side bounded live-snapshot cache and additive `lineage/export` RPC, and ships deterministic Mermaid and Graphviz DOT exporters over a shared Export IR that Stage C.2's Offline Interactive HTML+SVG viewer will reuse directly.

## Goals

*   **Accurate, Complete `SliceGraph` Foundation (Block 0):** Merge Phase 2 LSP results into `SliceGraph`; emit real `file_id`s, cross-file `To` callee nodes, and `ProjectField`/`ProjectIndex`/`UnwrapVariant` edges; populate real `is_back_edge`/`cycle_group_id` flags via `scc::tarjan_scc`; add backward-compatible typed `Guard` and `node_id`-linked `DerefDanger` metadata under `SCHEMA_VERSION = 1`; and normalize percent-encoded and Windows file URIs cleanly.
*   **Exact Live Snapshot Export Without Slowing Interactive Views:** Keep `include_graph = false` on the Hover and Tree wire hot path while maintaining a bounded 4-slot `query_id` graph LRU in `TransportState` (updated in-place by `execute_phase2` and `expand_node` with collision-free node/file IDs), falling back to a fresh slice when a `query_id` is evicted or when invoked from a normal source buffer.
*   **Bounded Phase 2 LSP Settle Without Blocking Phase 1:** Provide a dedicated additive `lineage/export` RPC (advertised via `EngineCapabilities.export_formats`, preserving `PROTOCOL_VERSION = 1`) that settles Phase 2 on the Phase 2 worker pool up to a configurable `export.lsp_timeout_ms = 500` ceiling, returns immediately (`< 50ms`) when no Phase 2 work is pending, and stamps explicit completeness status (`Budget-capped (partial)` vs. `Static only (LSP timed out / no LSP / --no-lsp)`).
*   **100% Offline, Deterministic Mermaid & DOT Exporters:** Produce byte-identical output between CLI (`lineage-engine export` and unified `slice --format dot`) and Neovim (`:Lineage export`) for identical graph inputs, using canonical `origin -> consumer` causal edge orientation, safe canonical node IDs (`N0..Nk`), valid GitHub-flavored Mermaid escaping (with a 45 KB guard below GitHub's 50 KB ceiling), valid DOT escaping, Coverage-Honesty 2-axis visual styling (`NonNull`, `Nullable`, `Absent`, `Unverified`, `Guard`, `Local Risk`, `Cycle`), `< 50ms` render time for `<= 100` nodes, zero new Cargo crates, and `< 200 KB` binary delta.
*   **Strict Path Privacy and Safe Delivery:** Strip all raw `file://` URIs and host absolute prefixes across every exported field (`FileEntry`, `symbol`, `label`, `UnknownBoundary`) into a 2-tier policy (`<workspace-rel>` POSIX paths vs. `<external>/<sanitized-tail>`, with CLI-only `--absolute-paths`). Omit raw source snippets by default (`--no-snippets`; opt-in `--snippets`). Default Neovim Mermaid/DOT delivery to system clipboard (`+`, warning on fallback to `"`) or explicit file path (requiring `:Lineage export!` to overwrite).
*   **Ergonomic Multi-Arg Commands & Zero Keymap Collisions:** Support multi-argument `:Lineage export ...` parsing (`nargs = "*"`), non-colliding buffer triggers (`e` in Tree Panel, `ge` in Hover, plus `<Plug>(LineageExport*)`), symmetric flags (`--no-lsp`, `--snippets`/`--no-snippets`, `--direction to|from|both`, `--exclude-tests`, `--null-only`), and source-free export observability in engine logs and Neovim's telemetry ring.

## Non-Goals

*   **Stage C.2 — Offline Interactive HTML+SVG Viewer (Immediate Follow-On):** Custom pure-Rust top-to-bottom layered-DAG SVG coordinate layout (bounded dummy-node waypoints `<= 600`, shared interval-packed right cycle gutter and left overflow gutter via inter-rank corridors, integer `i32` coordinates, monospace `<clipPath>` sizing, zero overlapping boxes, no enclosing SVG file rectangles), embedded vanilla JS interactive viewer (`engine/src/export/assets/{viewer.css, viewer.js}`, pan/zoom/fit, causal path highlight, OR-composed focus-dimming filters with live counts, sequential `palette[file_idx % N]` clickable per-file legend, inverted `file://`-safe 3-step click-to-copy, `"Save Without Snippets"` via isolated `#lineage-snippets` island + pre-boot clone, strict CSP + matching Rust/JS JSON Unicode escaping), and managed `0700`/`0600` content-addressed HTML cache (`20` most recent files by `mtime`, lazy prune on export, `:Lineage export clean`, SSH/headless/WSL-safe `vim.ui.open`) are intentionally deferred to Stage C.2 so they build cleanly on top of the verified Stage C.1 Export IR and RPC.
*   **2D ASCII/Unicode Sugiyama DAG Terminal Buffer:** Permanently dropped due to poor readability-to-complexity ratio in 80–120 column splits. Any future in-terminal convergence view would use a `git log --graph`-style vertical rail layout.
*   **3-Tier `<stdlib>/...` Heuristic Path Classifier:** Excluded in favor of the deterministic 2-tier `<workspace-rel>` vs. `<external>/<sanitized-tail>` policy.
*   **Reverse Browser-to-Neovim RPC Bridge, External Network/CDN Assets, or Mandatory `graphviz` Runtime:** Strictly out of scope; all Stage C exports are 100% offline.
*   **Breaking Wire or Schema Changes:** `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` remain backward-compatible via additive `#[serde(default)]` fields only.
*   **Post-v1.2 Distribution & Power Workflows:** Windows prebuilt binaries, Mason registry publishing, Multi-Location Queue, and workspace null-risk diagnostics remain deferred.

## Open Questions

*   None — all Stage C.1 design decisions are resolved; Stage C.2 (Offline Interactive HTML+SVG Viewer) contracts are staged in Future Improvements.

## Critical User Journeys (CUJs)

### CUJ 1: Sharing a Converging Null-Propagation Slice in a GitHub PR from Active Tree Panel

*   **Given**: A developer has `:Lineage to` (or `:Lineage both`) open in the Tree Panel on a TypeScript, Python, Rust, or Go symbol, has lazily expanded one `FrontierNode`, toggled `exclude_tests = true`, and Phase 2 LSP enrichment has settled.
*   **When**: The developer presses `e` in the Tree Panel (or runs `:Lineage export mermaid`).
*   **Then**:
    *   `lineage.nvim` calls `lineage/export` referencing the active Tree `query_id` and active UI filters without re-slicing from scratch.
    *   `lineage-engine` reads the updated graph snapshot from its 4-slot `TransportState` LRU (including Phase 2 edges and expanded frontier nodes with collision-free IDs), normalizes arrows in causal `origin -> consumer` order, sanitizes all file paths to workspace-relative POSIX paths (or `<external>/...`), omits raw code snippets, and returns valid GitHub-flavored Mermaid text in `< 50ms`.
    *   `lineage.nvim` writes the Mermaid diagram to system clipboard (`+`) and notifies the user with node/edge counts (or falls back to `"` with an explicit `WARN` if system clipboard is unavailable). Pasting the diagram into a GitHub PR comment renders cleanly with zero parse errors and distinct styling for `Absent`/`Nullable`, `Unverified`, `Guard`, `Local Risk`, and `Cycle` edges.

### CUJ 2: Fresh Cursor Export to a File with Bounded Phase 2 LSP Settle

*   **Given**: A developer has their cursor on a variable in a normal source buffer with an attached LSP server (`vtsls`, `pyright`/`ty`, `rust-analyzer`, or `gopls`).
*   **When**: The developer runs `:Lineage export dot docs/auth-flow.dot --direction both`.
*   **Then**:
    *   If `docs/auth-flow.dot` already exists and `!` was not passed, `lineage.nvim` refuses to overwrite it and emits an actionable error pointing to `:Lineage export!`.
    *   Otherwise, `lineage-engine` runs Phase 1 and settles Phase 2 LSP edges on the Phase 2 worker pool (without blocking Phase 1 Hover workers) up to `export.lsp_timeout_ms` (`500ms` default). If no Phase 2 work is pending, it returns immediately (`< 50ms`).
    *   If Phase 2 settles, the exported Graphviz DOT file contains Phase 2 nodes/edges and workspace-relative `subgraph cluster_*` grouping. If Phase 2 times out (or `--no-lsp` was passed), the DOT output renders the Phase 1 / partial result stamped with an explicit completeness note (`Static only (LSP timed out after 500ms)` or `Static only (--no-lsp)`), distinct from budget truncation (`Budget-capped (partial)`).

### CUJ 3: Byte-Identical Standalone CLI and CI Graph Generation

*   **Given**: A maintainer or CI script runs `lineage-engine export path/to/file.rs:42:8 --format mermaid --direction to --workspace-root . --no-lsp` (or `lineage-engine slice ... --format dot`, or `--from-envelope snapshot.json`).
*   **When**: The CLI executes outside Neovim against the same workspace and source files.
*   **Then**:
    *   `lineage-engine` runs the exact same `engine/src/export/` canonicalization and serialization pipeline as the Neovim `lineage/export` RPC, stripping volatile runtime fields (`query_id`, `trace_id`, wall-clock timings).
    *   Output bytes are deterministic and byte-identical to an equivalent Neovim `--no-lsp` export on the same cursor target and workspace root, and valid DOT output passes `dot -Tsvg` when Graphviz is installed.

### CUJ 4: Safe Capability Check Against Pre-Stage-C Engine Binaries

*   **Given**: A user upgrades `lineage.nvim` Lua files before updating their pinned `lineage-engine` binary (`PROTOCOL_VERSION = 1` without `export_formats` in `EngineCapabilities`).
*   **When**: The user runs `:Lineage export mermaid` or presses `e` / `ge`.
*   **Then**: `lineage.nvim` inspects `EngineCapabilities.export_formats` from the `initialize` handshake (and handles `-32601 Method not found` defensively), failing fast with a clear notification instructing the user to run `:Lineage install!` rather than hanging or raising an unhandled Lua error.

## Technical Plan (Architecture)

Stage C.1 is organized into four clean layers that preserve `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` via additive `#[serde(default)]` fields only: (1) Block 0 `SliceGraph` producer hardening, (2) a bounded 4-slot engine-side snapshot LRU and async-settled `lineage/export` RPC, (3) a pure-Rust `engine/src/export/` canonicalization and text-serializer module (`mermaid`, `dot`), and (4) unified CLI and Neovim multi-argument command/delivery plumbing.

```text
+------------------------------------------------------------------------------+
|                        Neovim 0.10+ (lineage.nvim, Lua)                      |
|                                                                              |
|  [:Lineage export ... (nargs="*")]   [Tree Panel (e)]   [Hover Float (ge)]   |
|         |                                  |                   |             |
|         +---------> [lua/lineage/export.lua] <-----------------+             |
|                      - checks EngineCapabilities.export_formats              |
|                      - resolves source: Tree > Hover > Cursor                |
|                      - clipboard (+, WARN -> ") or file (! to overwrite)     |
+------------------------------------|-----------------------------------------+
                                     | NDJSON stdio (lineage/export)
+------------------------------------v-----------------------------------------+
|                      lineage-engine (Standalone Rust)                        |
|                                                                              |
|  [Phase 1 Pool] ---> [TransportState: 4-slot query_id Snapshot LRU]          |
|       |                 ^  (updated in-place by execute_phase2 & expand)     |
|       | (if fresh +     |                                                    |
|       |  Phase 2 needed)|                                                    |
|       v                 |                                                    |
|  [Phase 2 Pool (lsp_timeout_ms = 500)] ---> [SlicerEngine (Block 0 Fixes)]   |
|                                              - Phase 2 graph merging         |
|                                              - Cross-file To & Proj edges    |
|                                              - tarjan_scc back-edges         |
|                                              - Typed Guard & DangerSink IDs  |
|                                                         |                    |
|                                                         v                    |
|  [CLI: lineage-engine export / slice --format dot] ---> [engine/src/export/] |
|                                                          1. sanitize.rs      |
|                                                          2. ir.rs (causal)   |
|                                                          3. mermaid.rs       |
|                                                          4. dot.rs           |
|                                                          (C.2: layout+html)  |
+------------------------------------------------------------------------------+
```

### 1. Block 0: `SliceGraph` Producer Hardening (`SCHEMA_VERSION = 1` Additive)

Before any exporter consumes `SliceEnvelope`, Block 0 fixes six producer gaps so `SliceGraph` reflects the same dataflow reality that Hover and Tree display:

*   **Additive Schema Metadata (`engine/src/protocol/schema.rs`):**
    *   Extend `FlowNode` with backward-compatible optional fields (`#[serde(default)]`):
        *   `is_guard: bool` and `guard_text: Option<String>` (populated directly from `SsiOp::Pi` guards, retiring `node.label.starts_with("narrow ")` heuristics).
        *   `is_danger_sink: bool` and `danger_kind: Option<String>` (populated directly when a `DangerSink` attaches to a node).
    *   Extend `FlowEdge` with `#[serde(default, skip_serializing_if = "Option::is_none")] pub edge_side: Option<EdgeSide>` (`Upstream | Downstream`) so `Both` slices explicitly distinguish upstream `To` edges from downstream `From` edges (falling back to provenance classification only on legacy `--from-envelope` JSONs).
    *   Extend `HoverRisk` with `#[serde(default, skip_serializing_if = "Option::is_none")] pub node_id: Option<String>` so local risks link directly to their `FlowNode`.
    *   Extend `ExpandParams` with `#[serde(default)] pub parent_query_id: Option<u64>` and `#[serde(default)] pub parent_node_id: Option<String>` so `lineage/expand` merges expanded subgraphs into the parent cached snapshot.
    *   Extend `EngineCapabilities` with `#[serde(default)] pub export_formats: Vec<String>` (`["mermaid", "dot"]` in Stage C.1; `["mermaid", "dot", "html"]` in Stage C.2) so `lineage.nvim` detects pre-Stage-C binaries at `initialize` without bumping `PROTOCOL_VERSION`.
*   **Complete Node, Edge, and Cycle Emission (`engine/src/slicer/mod.rs`, `pipeline.rs`, `projectors.rs`):**
    *   In `record_inst_node`: emit def-use edges for `SsiOp::ProjectField { base, .. }`, `SsiOp::ProjectIndex { base, .. }`, and `SsiOp::UnwrapVariant { base, .. }` (matching `backward_reachable`); populate `is_guard`/`guard_text` on `SsiOp::Pi`; and link `DangerSink` nodes via `is_danger_sink`/`danger_kind`/`HoverRisk.node_id`.
    *   In `eval_inst`: when a `CallTarget::Imported` or Go same-package call resolves to a cross-file function during a `To` slice, emit a `RawSliceNode` with the resolved callee's real `file_id`, range, symbol label, and `return_absence`, plus a connecting call edge (`provenance = "cross_file_call"`, `edge_side = Some(EdgeSide::Upstream)`). Tag all `From` / caller edges with `edge_side = Some(EdgeSide::Downstream)`.
    *   Wire existing `engine/src/slicer/scc.rs::tarjan_scc` after reduction to populate `FlowEdge.cycle_group_id` (SCC membership), and compute `FlowEdge.is_back_edge` via a deterministic 3-color DFS feedback-arc pass (`White`/`Gray`/`Black`, including self-loops `u == v`) so forward cycle edges stay top-to-bottom while only feedback back-edges are marked `is_back_edge = true`. Fix `project_tree_rows` (`projectors.rs:269`) to track the active DFS recursion stack rather than a global visited set so diamond fan-in is never mislabeled `[cycle]`.
*   **Phase 2 LSP Graph Merging (`engine/src/slicer/mod.rs`):**
    *   Parameterize both the Phase 2 wall-clock ceiling (`phase2_ceiling_ms` on `Phase2Context`, defaulting to `PHASE2_CEILING_MS = 250` for interactive Hover/Tree and `500` for fresh exports) and propagate `ExportParams.lsp_timeout_ms` to per-call `SliceParams.lsp_timeout_ms` (capped at `phase2_ceiling_ms`) instead of leaving individual LSP requests capped at `150ms`.
    *   When `execute_phase2` resolves `Definition`/`TypeDefinition`/`Implementation` targets or `CallHierarchyIncoming`/ripgrep callers, append the resolved `FlowNode`s and `FlowEdge`s (with explicit `edge_side`) into `ctx.pipeline_graph`, update target `FlowNode.absence` after clearing resolved `UnknownReason` bits, update `stats.nodes_emitted`, and record whether any Phase 2 item timed out or hit the ceiling.
*   **URI Normalization (`engine/src/cache/memory.rs`, `engine/src/slicer/mod.rs`):**
    *   Upgrade `uri_to_path` to percent-decode UTF-8 escape sequences (`%20`, etc.) and normalize Windows `file:///C:/...` drive prefixes so `Path::strip_prefix(workspace_root)` works across spaces and OSes. Replace ad-hoc `format!("file://{}", ...)` calls in `slicer/mod.rs` with `path_to_uri`.

### 2. Engine-Side Bounded Snapshot LRU & Dedicated `lineage/export` RPC

*   **4-Slot `query_id` Graph LRU (`engine/src/protocol/transport.rs`):**
    *   Hover (`lua/lineage/ui/hover.lua`) and Tree (`lua/lineage/ui/tree.lua`) keep `include_graph = false` on the wire so interactive latency and JSON payload size remain unchanged.
    *   Inside `TransportState`, a mutex-guarded 4-slot LRU (`QuerySnapshotCache`) stores `MaterializedSliceSnapshot { files, root_ids, direction, graph, local_risks, unknown_boundaries, truncation, phase2_status, expand_seq }` keyed by `query_id` whenever `lineage/slice` completes Phase 1 (capturing the internally built `pipeline_graph` before omitting `env.graph` from the wire reply).
    *   When `execute_phase2` finishes on the Phase 2 pool, it updates the matching `query_id` entry in-place with the merged graph, updated `files`, `truncation`, and `phase2_status = Complete | TimedOut`.
    *   When `lineage/expand` runs with `parent_query_id` and `parent_node_id`, `expand_node` runs with `include_graph = true` internally, increments `expand_seq` (`k`), remaps expanded `file_id`s against the parent's `files` table, namespaces every expanded node ID as `exp{k}_{id}` (eliminating `n0`/`n1` collisions), links the expanded root to `parent_node_id` (clearing `FrontierNode` status), and merges into the cached snapshot in-place while still returning lightweight `ExpandResult` on the wire.
*   **Dedicated `lineage/export` RPC Contract (`schema.rs`, `transport.rs`):**
    ```text
    ExportParams {
        format: "mermaid" | "dot",             // Stage C.2 adds "html"
        query_id?: u64,                        // Active Tree/Hover snapshot ID
        // Fresh-slice target (used when query_id is absent or evicted from LRU):
        uri?: String,
        position?: LspPosition,
        offset_encoding?: OffsetEncoding,
        direction?: SliceDirection,            // default: To
        exclude_tests?: bool,
        lsp_attached?: bool,
        lsp_client_name?: String,
        workspace_root?: String,
        // Export & settle options:
        null_only?: bool,                      // default: false
        include_snippets?: Option<bool>,       // tri-state: None => false uniformly in engine; explicit true/false overrides
        no_lsp?: bool,                         // default: false
        lsp_timeout_ms?: u64,                  // default: 500
        absolute_paths?: bool,                 // CLI-only; RPC always false
    }

    ExportResult {
        schema_version: 1,
        format: String,
        content: String,                       // Pure UTF-8 Mermaid or DOT
        source_mode: "live_snapshot" | "fresh_slice",
        phase2_status: "complete" | "not_needed" | "skipped" | "timed_out",
        node_count: usize,
        edge_count: usize,
        truncated_nodes: usize,
        truncated_edges: usize,
        truncation: Vec<TruncationReason>,
        completeness_badges: Vec<String>,      // e.g. "Budget-capped (partial)", "Static only (LSP timed out after 500ms)"
        warnings: Vec<String>,                 // e.g. Mermaid 45 KB guard notice
    }
    ```
*   **Worker Pool Isolation (Zero Phase 1 Starvation):**
    *   If `query_id` hits the LRU and Phase 2 is not in flight (`Complete | NotNeeded | Skipped | TimedOut`), `handle_request("lineage/export")` renders immediately in `< 50ms` and replies on `out_tx`.
    *   If `query_id` hits the LRU while Phase 2 is still `InFlight` (and `!no_lsp`), `handle_request` renders the current Phase 1 snapshot stamped `Static only (LSP still settling)` (or attaches a bounded waiter up to `lsp_timeout_ms` on the Phase 2 pool without re-slicing or blocking Phase 1 workers—avoiding duplicate LSP calls or dropped expanded frontiers).
    *   If a fresh slice is needed, `handle_request("lineage/export")` runs `slice_phase1` (`< 50ms`). If `no_lsp = true` or `phase2_opt` is `None`, it renders and replies immediately with `phase2_status = "skipped"` or `"not_needed"` (zero artificial 500ms wait).
    *   If `phase2_opt` is `Some(ctx)` and `!no_lsp`, `handle_request` hands an `ExportPhase2Job` (carrying `RpcId`, `ExportParams`, and `ctx` with `phase2_ceiling_ms = lsp_timeout_ms`) onto the dedicated Phase 2 worker pool (2–4 threads) and returns immediately—never blocking Phase 1 workers (`parse_worker_cap() == 1` on 2-core hosts). If `phase2_tx.try_send` fails because the 64-slot queue is full, it logs `tracing::warn!(reason = "phase2_queue_full", ...)` and falls back immediately to rendering Phase 1 stamped `Static only (LSP queue full)`.

### 3. Shared Pure-Rust Export IR & Path Sanitization (`engine/src/export/`)

All export logic lives in a dedicated top-level Rust module (`engine/src/export/{mod.rs, ir.rs, sanitize.rs, mermaid.rs, dot.rs}`) with **zero new Cargo dependencies** and pure functions over `(&SliceEnvelope, &ExportOptions)`:

*   **Canonical `ExportGraph` IR (`ir.rs`):**
    *   **Volatile Metadata Stripping:** Ignores `query_id`, `trace_id`, `stats.phase1_ms`, and `stats.phase2_ms` completely so CLI and Neovim produce byte-identical output for identical graphs.
    *   **Causal Edge Orientation Normalization & `Both` Partitioning:** Normalizes raw `SliceGraph` edges into uniform top-to-bottom causal flow (`origin -> consumer`), preserving `direction: SliceDirection`, canonical `root_ids: Vec<String>`, and per-node `partition: Upstream | Focal | Downstream`:
        *   `To` slices reverse def-use edges (`consumer -> origin` becomes `origin -> consumer`, placing the target cursor symbol at the bottom sink).
        *   `From` slices retain `root -> caller` flow (placing the target symbol at the top).
        *   `Both` slices use `FlowEdge.edge_side` (`Upstream | Downstream`, falling back to provenance classification only on legacy `--from-envelope` JSONs) to reverse upstream `To` edges (`origin -> root`) and retain downstream `From` edges (`root -> caller`), placing `root_ids` at the center waist.
    *   **Separated SCC Identity vs. Deterministic DFS Feedback Back-Edges:** Runs `scc::tarjan_scc` over the normalized causal graph strictly to populate `cycle_group_id: Option<u32>` (SCC membership), and runs a single deterministic 3-color DFS feedback-arc pass (`White`/`Gray`/`Black` in canonical `N0..Nk` order, including self-loops `u == v`) to set `ExportEdge.is_back_edge: bool` (never unioning raw pre-reversal `tarjan_scc().back_edges`, which marks every edge inside a multi-node SCC). Removing `is_back_edge` edges is guaranteed to yield a DAG while preserving forward cycle edges (`A -> B -> C`) top-to-bottom across Mermaid, DOT, and Stage C.2 HTML. Merges parallel identical `(from, to)` edges by deduplicating labels.
    *   **Centralized Snippet Normalization, Scrubbing & Full Inspector Metadata:** Performs UTF-8 char-safe single-line trimming (`<= 120` chars) and `PathSanitizer::scrub_text` on `FlowNode.snippet` directly in `ir.rs` (never byte slicing, never duplicated per serializer). Retains full typed metadata on `ExportGraph`/`ExportNode`/`ExportFile` so Stage C.2's HTML side inspector consumes `&ExportGraph` exclusively without bypassing to `&SliceEnvelope`: `file_index`, 0-based `range: CanonicalRange`, sorted deduplicated `unknown_reasons: Vec<UnknownReason>`, scrubbed `guard_text`, `danger_kind`, scrubbed `risk_messages`, `provenance`, scrubbed `unknown_boundaries`, `completeness_badges`, and export-time filter flags (`null_only`, `exclude_tests`).
    *   **`null_only` Causal Path Preservation:** When `null_only = true`, retains every node belonging to a causal path reaching or leaving any `Absent`/`Nullable` node (keeping connecting edges intact instead of producing disconnected dots).
    *   **Deterministic BFS Node & Edge Pruning + Safe Canonical IDs:** Uses `BTreeMap` and sorted `Vec` exclusively (no `HashMap` iteration). Enforces hard caps of `max_nodes = 200`, `max_edges = 800`, and `max_label_chars = 256` via deterministic BFS from `root_ids` (prioritizing `Absent`/`Nullable`, `Local Risk`, and root-incident items, dropping pruned nodes together with all incident edges) and records exact `truncated_nodes`/`truncated_edges`. Sorts surviving nodes by `(norm_path, start.line, start.col, kind, label, raw_id)` and assigns safe identifiers `N0`, `N1`, ... (eliminating `caller:1:0` colons and Mermaid reserved keywords).
    *   **2-Axis Coverage-Honesty Taxonomy:** Preserves orthogonal node state matching `README.md` (`absence_class: NonNull | Nullable | Absent` separate from `is_unverified`, `is_guard`, `is_danger_sink`, `is_frontier`, `is_root`, `in_cycle`). A node that is both `Nullable` and `Unverified` displays both cues rather than collapsing into one mutually exclusive bucket.
*   **2-Tier Path Sanitization (`sanitize.rs`):**
    *   Normalizes all `FileEntry.uri` entries (percent-decoded, forward slashes) against `workspace_root`:
        *   Inside `workspace_root`: workspace-root-relative POSIX path (`src/auth/session.ts`).
        *   Outside `workspace_root`: `<external>/<sanitized-tail>` (keeping at most the last 2–3 non-home path components such as package + filename, stripping `/home/<user>`, `/Users/<user>`, and Windows drive prefixes).
    *   Scrubs any embedded `file://` URI or `workspace_root` prefix found inside `FlowNode.label`, `symbol`, `snippet`, `guard_text`, `HoverRisk.message`, or `UnknownBoundary` (preventing leaks from `guard_error_envelope` or debug-formatted `CallTarget`).
    *   When CLI `--absolute-paths` is explicitly set (disallowed over RPC), preserves normalized filesystem paths.
*   **Mermaid Serializer (`mermaid.rs` — Flat File-Pill Layout):**
    *   Always emits `flowchart TD` with **flat styled nodes and per-node `file:line` pills** (no `subgraph` blocks, per locked Option 1), eliminating GitHub Dagre cross-subgraph inversions and header-escaping bugs.
    *   Emits deterministic completeness header comments (`%% lineage.nvim export · direction=to · ...`), safe IDs `N0..Nk`, strict label quoting/entity-escaping (`"`, `<`, `>`, `::`, `|`, `%%`, `;`, `[`, `]`, `{`, `}`, `(`, `)`, newlines), reading pre-trimmed scrubbed snippets from `ExportNode.snippet` only when `--snippets` is enabled, `classDef` styling distinguishing `NonNull`, `Nullable`, `Absent`, `Unverified`, `Guard`, `Local Risk`, and `Frontier`, and dashed cycle back-edges (`-. cycle .->`).
    *   Enforces a **45 KB character ceiling** (below GitHub's 50 KB `maxTextSize`): if rendered text exceeds 45 KB, trims optional snippets first and then re-prunes via BFS priority with an explicit warning in `ExportResult.warnings` and header comment.
*   **Graphviz DOT Serializer (`dot.rs` — Unified with `slice --format dot`):**
    *   Emits `digraph lineage` (`rankdir=TB`, monospace node font) with deterministic `subgraph cluster_f<idx>` file groupings (labeled with sanitized relative/`<external>` paths), proper DOT escaping (`\` before `"` and newlines, plain quoted strings without brittle HTML-table syntax), semantic border/fill/style attributes matching Mermaid's Coverage-Honesty taxonomy, dashed red/orange back-edges for cycles, and a header comment + graph label carrying completeness badges.
    *   Replaces the inline DOT formatting loop in `engine/src/main.rs:195-210` so `lineage-engine slice --format dot` and `lineage-engine export --format dot` emit byte-identical output.

### 4. CLI & Neovim Command, Delivery, and Observability Architecture

*   **Standalone CLI (`engine/src/cli.rs`, `engine/src/main.rs`):**
    *   Split `CliFormat` into `SliceCliFormat { Json, Text, Dot }` and `ExportCliFormat { Mermaid, Dot }` (Stage C.2 adds `Html`) so invalid pairings (`export --format json`, `slice --format html`) are rejected at parse time.
    *   Add `Commands::Export` accepting either `<file>:<line>:<col>` or `--from-envelope <path|->`, plus `--format mermaid|dot`, `--direction to|from|both`, `--workspace-root <dir>` (defaulting to nearest VCS root `.git` or current working directory—never `file.parent()`), `--exclude-tests`, `--null-only`, `--no-lsp`, `--lsp-fixture <path>`, tri-state `--snippets` / `--no-snippets` (`conflicts_with`, resolving to `Option<bool>`), `--absolute-paths`, and `--out <path>` (stdout when omitted).
    *   Add `Both` to `CliDirection` and fix `Commands::Slice` so `--lsp-fixture` sets `lsp_attached = true` and runs Phase 2 when provided.
*   **Neovim Commands & Export Orchestration (`plugin/lineage.lua`, `lua/lineage/commands.lua`, `lua/lineage/export.lua`, `lua/lineage/client.lua`, `lua/lineage/debug.lua`, `lua/lineage/config.lua`):**
    *   Move `:Lineage` command dispatch and tab-completion into `lua/lineage/commands.lua` and update `plugin/lineage.lua` from `nargs = "?"` to `nargs = "*"` (preserving all existing subcommands and `:Lineage install!`). Expose `<Plug>(LineageExport)`, `<Plug>(LineageExportMermaid)`, and `<Plug>(LineageExportDot)`.
    *   Support order-independent flag parsing on `:Lineage export <format> [path] [flags]` using symmetric flags (`--direction to|from|both`, `--exclude-tests`, `--null-only`, `--no-lsp`, `--snippets`, `--no-snippets`, `--out <path>`, rejecting simultaneous positional `[path]` + `--out`). Pass `include_snippets = nil` unless `--snippets` or `--no-snippets` was explicitly supplied.
    *   **Transport Hardening (`lua/lineage/client.lua`):** Replace `state.buf = state.buf .. data` with a chunk-array accumulator (`table.concat` only when a chunk contains `"\n"`) so multi-chunk export responses avoid O(N²) string churn; reject/fail pending request callbacks cleanly if `vim.json.decode` fails; clear `state.capabilities = nil` and `state.buf` on `stop()` and process exit; gate capability inspection inside `client.ensure_started(...)`.
    *   **Context Precedence in `lua/lineage/export.lua`:** `Focused Tree buffer > Focused Hover float > Normal source buffer at cursor` (Focus/Pin does not hijack cursor-based export). Active Tree/Hover buffers pass their `query_id` plus fallback `(origin_uri, origin_pos, direction, exclude_tests, null_only)` so evicted LRU entries re-slice cleanly without user error.
    *   **Capability Check & Delivery:** Inspect `client.capabilities().export_formats` for exact membership of the requested format (`vim.tbl_contains`) and intersect picker options with Lua-supported delivery handlers (preventing a Stage C.1 Lua client paired with a Stage C.2 engine from dumping raw HTML to clipboard), while defensively handling `-32601 Method not found` with `:Lineage install!` guidance on older binaries. For clipboard delivery (default when no path is given), verify `vim.fn.has("clipboard") == 1` before writing `+`; if unavailable, write `"` and notify with `vim.log.levels.WARN`. When an explicit `[path]` is given, refuse to overwrite an existing file unless `!` (`:Lineage export!`) is supplied.
    *   **Keymaps & Config:** Add `e` in Tree Panel (`lua/lineage/ui/tree.lua`, advertised in the header bar as `e:export`, avoiding any collision with `nowait` `g`) and `ge` in Hover (`lua/lineage/ui/hover.lua`). By default (`export.picker = true`), pressing `e` or `ge` opens a `vim.ui.select` format picker over the intersection of `EngineCapabilities.export_formats` and Lua-supported formats (`Mermaid (clipboard)`, `Graphviz DOT (clipboard)` in C.1; extensible to `HTML (browser)` in C.2) while keeping Tree/Hover open; setting `export.picker = false` skips the menu and exports immediately to `export.default_format` (`"mermaid"`). Add `export = { default_format = "mermaid", lsp_timeout_ms = 500, picker = true }` and `tree.keymaps` to `lua/lineage/config.lua`.
    *   **Observability:** Emit source-free `tracing::info!` spans for every CLI and RPC export (`trace_id`, `query_id`, `format`, `source_mode`, `phase2_status`, `phase2_wait_ms`, `render_ms`, `node_count`, `edge_count`, `output_bytes`, `truncation`—zero snippets or labels), record `lineage/export` timing/status in `lua/lineage/client.lua`'s bounded `telemetry_ring`, and update `lua/lineage/debug.lua:show_status` so `:Lineage status` formats `lineage/export` entries cleanly instead of printing `phase1=nilms`.

### 5. Extension Seam for Stage C.2 (Offline Interactive HTML+SVG Viewer)

Stage C.2 plugs directly into Stage C.1 with zero architectural rework:
*   Adds `engine/src/export/layout.rs` (pure integer `i32` top-to-bottom layered coordinate pass over `&ExportGraph`: bounded dummy-node waypoints `<= 600`, shared `O(k log k)` interval lane packer for right cycle gutter + left long-span overflow gutter via inter-rank horizontal corridors, `Both` focal waist pinning, monospace `<clipPath>` sizing, zero overlapping bounding boxes, no enclosing SVG file boxes) and `engine/src/export/html.rs` + `engine/src/export/assets/{viewer.css, viewer.js}` (`include_str!`, strict `<meta>` CSP, isolated `#lineage-snippets` + `#lineage-meta` JSON islands with matching Rust/JS Unicode escaping, ~250–350 LOC vanilla JS with pan/zoom/fit, causal cone focus-dimming, OR-composed `Null-only`/`Guards`/`Unverified` filters with live counts, sequential `palette[file_idx % N]` clickable per-file legend, inverted `file://`-safe 3-step click-to-copy, OS dark/light + toolbar toggle, and `"Save Without Snippets"` via pre-boot `pristineRoot` clone).
*   Registers `"html"` in `EngineCapabilities.export_formats`, `ExportCliFormat`, and `lineage/export` (keeping engine default `include_snippets = None => false` uniformly across all formats, while Neovim passes `include_snippets = true` only for ephemeral private-cache HTML opens unless `--no-snippets` is set).
*   Extends `lua/lineage/export.lua` with `lua/lineage/export_cache.lua` (content-addressed `0700`/`0600`-from-birth cache in `stdpath("cache")/lineage/exports/`, count-only 20-file lazy prune on export, `:Lineage export clean`, silent explicit-path saves with `--open` opt-in, and SSH/headless/WSL-safe `vim.ui.open`).

## Alternatives Considered

### Alternative A: Shipping a 2D ASCII/Unicode Sugiyama DAG Buffer Inside Neovim

*   **Pros**: Displays converging diamond topology directly inside a Neovim split without copying text or opening another tool.
*   **Cons**: Even at 15–20 nodes, diamond joins, back-edges, and file/symbol labels cause crossing edges, wide boxes, and severe horizontal wrapping in 80–120 column terminal splits. Implementing and maintaining a 2D character-grid crossing-reduction and edge-routing engine in Rust is high cost for poor terminal legibility.
*   **Reason for Rejection**: Rejected by author: poor readability-to-complexity ratio compared to the existing Interactive Tree Panel for in-terminal navigation plus Mermaid/DOT (Stage C.1) and offline HTML+SVG (Stage C.2) for diamond inspection. Any future in-terminal convergence view would use a `git log --graph`-style vertical rail layout instead.

### Alternative B: Shipping Mermaid, DOT, and Interactive HTML+SVG in a Single Stage C Milestone

*   **Pros**: Delivers the offline browser viewer and text exporters in one planning and release cycle.
*   **Cons**: Combines Block 0 `SliceGraph` producer fixes, engine-side snapshot caching, `lineage/export` RPC, Mermaid/DOT serialization, custom Rust layered-SVG layout (dummy waypoints + cycle gutter), ~300 LOC embedded vanilla JS, CSP/XSS hardening, and temp HTML cache management into one large blast radius.
*   **Reason for Rejection**: Deferred to Stage C.2 by author: sequencing Stage C.1 (Block 0 + Export IR + LRU + RPC + Mermaid/DOT) ahead of Stage C.2 (full Interactive HTML+SVG viewer) keeps C.1 focused and verifiable without cutting any HTML+SVG layout or security requirements.

### Alternative C: Dropping Engine-Side Snapshot Caching and Re-Slicing from Active UI Parameters

*   **Pros**: Eliminates the 4-slot `QuerySnapshotCache` LRU in `TransportState` and avoids merging `expand_node` subgraphs into parent graphs.
*   **Cons**: Re-slicing from `(origin_uri, origin_pos, direction, exclude_tests)` silently discards lazily expanded `FrontierNode` subtrees that the developer opened in the Tree Panel, breaking WYSIWYG parity between what is on screen and what is exported.
*   **Reason for Rejection**: Rejected by author: a bounded 4-slot `query_id` graph LRU in `TransportState` keeps `include_graph = false` on the Hover/Tree wire hot path, preserves expanded frontiers and Phase 2 merges in-place, and falls back cleanly to a fresh slice only if evicted.

### Alternative D: Conditional Mermaid `subgraph` File Grouping on Acyclic Cross-File Slices

*   **Pros**: Draws enclosing file rectangles in Mermaid when cross-file hops are strictly acyclic (`handler.ts -> service.ts -> repo.ts`).
*   **Cons**: GitHub's Mermaid/Dagre build frequently degrades on multi-subgraph diamonds in narrow PR columns, `subgraph` titles containing paths and colons are a recurring parser hazard across Mermaid versions, and falling back to flat pills on ping-pong hops (`a.ts -> b.ts -> a.ts`) produces two shifting output structures.
*   **Reason for Rejection**: Rejected by author: always emitting flat Mermaid nodes with per-node `file:line` pills guarantees uniform, predictable rendering in GitHub Markdown while leaving enclosing `subgraph cluster_*` grouping to Graphviz DOT (where Graphviz handles cross-cluster routing natively).

### Alternative E: 3-Tier Path Sanitization Classifying `<stdlib>/...` Across All Four Languages

*   **Pros**: Distinguishes standard library stubs (`<stdlib>/...`) from third-party packages (`<external>/...`) in exported diagrams.
*   **Cons**: Detecting stdlib paths reliably across TypeScript (`node_modules/@types`, lib.dom), Python (`venv`, `site-packages`, `/usr/lib/python3.*`, `typeshed`), Rust (`~/.rustup/toolchains/.../library`), and Go (`GOROOT`, `GOMODCACHE`) across Linux, macOS, Windows, and Nix is brittle heuristic code with high maintenance cost.
*   **Reason for Rejection**: Rejected by author: a deterministic 2-tier policy (`<workspace-rel>` POSIX paths vs. `<external>/<sanitized-tail>`) protects privacy consistently with minimal code.

### Alternative F: Blocking Phase 1 Workers or Client-Side 500ms Polling on `lineage/slice`

*   **Pros**: Avoids adding an additive `lineage/export` RPC method and `ExportPhase2Job` dispatch onto the Phase 2 pool.
*   **Cons**: `lineage/slice` emits no `lineage/sliceDelta` when `phase2_opt` is `None` (no unresolved LSP items), so client-side polling stalls every local-only export for 500ms and falsely stamps `"LSP timed out"`. Blocking a Phase 1 worker synchronously for 500ms starves interactive Hover and `lineage/syncBuffer` on 2-core machines (`parse_worker_cap() == 1`).
*   **Reason for Rejection**: Rejected by author: a dedicated additive `lineage/export` RPC dispatching Phase 2 waiting onto the Phase 2 pool returns immediately in `< 50ms` when no Phase 2 work is pending and never starves interactive Phase 1 queries.

### Alternative G: Asymmetric Format Flags (`static`, `--include-snippets`, `--redact-snippets`) and `ge` in Tree Panel

*   **Pros**: Mirrors format-specific defaults directly in flag names (`--include-snippets` for text, `--redact-snippets` for HTML) and reuses one key (`ge`) across Hover and Tree.
*   **Cons**: `tree.lua` already binds single-key `g` with `nowait = true` (toggle grouping), so `ge` is shadowed or lags `g`; `"redact"` implies full secret/PII scrubbing rather than snippet omission; and having two names (`static` vs. `--no-lsp`) creates CLI/Neovim drift.
*   **Reason for Rejection**: Rejected by author: binding `e` in Tree Panel and `ge` in Hover, and standardizing on symmetric `--no-lsp` and `--snippets` / `--no-snippets` flags across CLI and Neovim, keeps keymaps collision-free and options uniform.

--------------------------------------------------------------------------------

## Non-Functional Requirements

*   **Privacy & Information Leakage:** All exporters strip raw `file://` URIs and host filesystem prefixes (`/home/<user>`, `/Users/<user>`, Windows drives) across `FileEntry.uri`, `FlowNode.label`, `symbol`, and `UnknownBoundary` by default (2-tier `<workspace-rel>` vs. `<external>/<sanitized-tail>`). Raw code snippets are omitted by default in Mermaid and DOT (`--no-snippets`). Engine tracing logs remain strictly source-free (metrics and counts only, never snippets, labels, or rendered diagram payloads).
*   **Performance, Memory & Binary Size:**
    *   Export rendering (`ExportGraph` canonicalization + Mermaid/DOT serialization): `< 50ms` for typical slices (`<= 100` nodes), hard safety cap `max_nodes = 200` via deterministic BFS priority pruning.
    *   Fresh-slice Phase 2 LSP settle ceiling: configurable `export.lsp_timeout_ms = 500`, zero wait when `phase2_opt` is `None`, zero blocking of Phase 1 workers.
    *   4-slot `QuerySnapshotCache` LRU bounded to 4 recent slices (negligible memory inside the existing `<= 300 MB` OS RSS ceiling).
    *   Zero new Cargo dependencies; release binary delta `< 200 KB`.
*   **Determinism & Byte-Identity:** No `HashMap`/`HashSet` iteration order in `engine/src/export/` (`BTreeMap`/sorted `Vec` only); all volatile runtime fields (`query_id`, `trace_id`, `phase1_ms`, `phase2_ms`) excluded from rendered output; identical `SliceEnvelope` + `workspace_root` inputs produce byte-identical output across CLI, RPC, Linux, macOS, and Windows.
*   **Backward Compatibility & Rollout Safety:**
    *   `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` remain backward-compatible via additive `#[serde(default)]` fields (`FlowNode`, `HoverRisk`, `ExpandParams`, `EngineCapabilities.export_formats`) and additive `lineage/export` RPC.
    *   Older `v1` engine binaries without `export_formats` fail fast in `lua/lineage/export.lua` with an actionable `:Lineage install!` prompt.
    *   Existing file paths passed to `:Lineage export <fmt> <path>` are never overwritten unless `:Lineage export!` is explicitly invoked.

--------------------------------------------------------------------------------

### Additional Resources

*   [concepts_brief.md](../../concepts_brief.md) — Stage C.1 7-lens Goldfish brainstorm synthesis and locked decisions.
*   [lineage-engine-v1.md](./lineage-engine-v1.md) — Frozen v1 CoreIR, `SliceEnvelope`, and protocol architecture.
*   [lineage-stage-a-v0-1-0-closure.md](./lineage-stage-a-v0-1-0-closure.md) — Stage A two-phase async transport split, release gates, and roadmap.
*   [core-ir-and-slicer spec](../../openspec/specs/core-ir-and-slicer/spec.md), [observability-and-cli spec](../../openspec/specs/observability-and-cli/spec.md), and [neovim-ui spec](../../openspec/specs/neovim-ui/spec.md) — Existing v1 capability specifications.

### Future Improvements

*   **Stage C.2 — Offline Interactive HTML+SVG Viewer:** Add `engine/src/export/{layout.rs, html.rs}` + `engine/src/export/assets/{viewer.css, viewer.js}` over Stage C.1's `ExportGraph` and `lineage/export` RPC with bounded dummy-node waypoints (`<= 600`), shared `O(k log k)` interval lane packer for right cycle gutter + left long-span overflow gutter via inter-rank corridors, integer `i32` coordinates, monospace `<clipPath>` sizing, strict `<meta>` CSP + matching Rust/JS JSON Unicode escaping against `</script>` XSS, isolated `#lineage-snippets` + pre-boot clone `"Save Without Snippets"`, OR-composed causal focus-dimming filters (`Null-only`, `Guards`, `Unverified` with live counts), sequential `palette[file_idx % N]` clickable per-file legend, inverted `file://`-safe 3-step click-to-copy, OS dark/light + toolbar toggle, and `0700`/`0600` content-addressed cache in `stdpath("cache")/lineage/exports/` (`20` most recent files by `mtime`, lazy prune on export only, `:Lineage export clean`, silent explicit-path saves with `--open` opt-in, SSH/headless/WSL-safe `vim.ui.open`).
*   **Vertical-Rail In-Terminal Convergence View:** If in-terminal diamond visualization is later needed without leaving Neovim, explore a `git log --graph`-style vertical rail layout rather than 2D character-grid boxes.
*   **Post-v1.2 Distribution & Power Workflows:** Windows prebuilt binaries, Mason registry entry, Multi-Location Queue, and opt-in workspace null-risk diagnostics.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

### Target Files to Create/Modify

#### 1. `engine/src/protocol/schema.rs` (Modify)
*   **Type / Scope**: Rust Wire Protocol Schema (`PROTOCOL_VERSION = 1`, `SCHEMA_VERSION = 1`, strictly backward-compatible additive fields)
*   **Proposed Changes**:
    *   Add `#[serde(default)] pub export_formats: Vec<String>` to `EngineCapabilities`.
    *   Add `#[serde(default)] pub is_guard: bool`, `#[serde(default, skip_serializing_if = "Option::is_none")] pub guard_text: Option<String>`, `#[serde(default)] pub is_danger_sink: bool`, and `#[serde(default, skip_serializing_if = "Option::is_none")] pub danger_kind: Option<String>` to `FlowNode`.
    *   Add `EdgeSide` (`Upstream | Downstream`) and `#[serde(default, skip_serializing_if = "Option::is_none")] pub edge_side: Option<EdgeSide>` to `FlowEdge`.
    *   Add `#[serde(default, skip_serializing_if = "Option::is_none")] pub node_id: Option<String>` to `HoverRisk`.
    *   Add `#[serde(default)] pub parent_query_id: Option<u64>` and `#[serde(default)] pub parent_node_id: Option<String>` to `ExpandParams`.
    *   Define `ExportFormat` (`Mermaid | Dot`), `ExportSourceMode` (`LiveSnapshot | FreshSlice`), `Phase2Status` (`Complete | NotNeeded | Skipped | TimedOut`), `ExportParams` (with tri-state `#[serde(default)] pub include_snippets: Option<bool>`), and `ExportResult` structs matching the Technical Plan RPC contract.

#### 2. `engine/src/cache/memory.rs` (Modify)
*   **Type / Scope**: Rust URI & Path Normalization Helper
*   **Proposed Changes**:
    *   Update `uri_to_path` to percent-decode UTF-8 escape sequences (`%20`, `%23`, etc.) and strip a leading `/` before Windows drive letters (`file:///C:/...` -> `C:/...`) so `Path::strip_prefix(workspace_root)` works reliably on paths with spaces and across Windows/Unix.
    *   Ensure `path_to_uri` produces normalized forward-slash `file://` URIs consistently on all platforms.

#### 3. `engine/src/slicer/pipeline.rs` (Modify)
*   **Type / Scope**: Rust Slicer Reduction Pipeline
*   **Proposed Changes**:
    *   Extend `RawSliceNode` with `is_guard: bool`, `guard_text: Option<String>`, `is_danger_sink: bool`, and `danger_kind: Option<String>`, and `RawSliceEdge` with `edge_side: Option<EdgeSide>`, propagating them through `to_flow_node`/`to_flow_edge` into `FlowNode`/`FlowEdge`.
    *   Preserve `is_guard` and `is_danger_sink` nodes during copy-chain contraction (never contract away a guard or danger sink).
    *   Invoke `crate::slicer::scc::tarjan_scc` on the finalized reduced graph to populate `FlowEdge.cycle_group_id` (SCC membership), and run a deterministic 3-color DFS feedback-arc pass (`White`/`Gray`/`Black`, including self-loops) to populate `FlowEdge.is_back_edge` so forward cycle edges stay distinct from feedback back-edges.

#### 4. `engine/src/slicer/projectors.rs` (Modify)
*   **Type / Scope**: Rust UI Projections (`Hover`, `Focus`, `Tree`)
*   **Proposed Changes**:
    *   Update `project_focus_spans` to read `node.is_guard` / `node.is_danger_sink` instead of matching `node.label.starts_with("narrow ")`.
    *   Fix `project_tree_rows` DFS cycle detection (`is_cycle`) to track the active recursion stack (`in_stack: HashSet<String>`) instead of a single global `visited` set so diamond fan-in is never mislabeled `[cycle]`.

#### 5. `engine/src/slicer/mod.rs` (Modify)
*   **Type / Scope**: Rust Slicer Engine Core (Block 0 Producer Hardening)
*   **Proposed Changes**:
    *   Replace remaining ad-hoc `format!("file://{}", ...)` calls with `path_to_uri`.
    *   In `record_inst_node`: emit def-use edges for `SsiOp::ProjectField { base, .. }`, `SsiOp::ProjectIndex { base, .. }`, and `SsiOp::UnwrapVariant { base, .. }` (`edge_side = Some(EdgeSide::Upstream)`); populate `is_guard = true` + `guard_text` on `SsiOp::Pi`; and link `DangerSink` instructions by setting `is_danger_sink = true`, `danger_kind`, and `HoverRisk.node_id = Some(id)`. Tag all `From` / caller edges with `edge_side = Some(EdgeSide::Downstream)`.
    *   In `eval_inst`: when cross-file `CallTarget::Imported` or Go same-package callees resolve during `To` slices, append a `RawSliceNode` with the resolved callee's real `file_id`, range, symbol label, and `return_absence`, plus a `cross_file_call` edge (`edge_side = Some(EdgeSide::Upstream)`) connecting the call instruction to the callee node.
    *   Add `phase2_ceiling_ms: u64` onto `Phase2Context` (defaulting to `PHASE2_CEILING_MS = 250`, overridable for `lineage/export` at `500ms`), propagate `ExportParams.lsp_timeout_ms` to per-call `SliceParams.lsp_timeout_ms` (capped at `phase2_ceiling_ms`), and track `phase2_timed_out: bool` when any LSP item times out or hits the ceiling.
    *   In `execute_phase2`: merge resolved Phase 2 LSP definitions, `CallHierarchyIncoming` callers, and ripgrep fallback callers directly as `FlowNode`s and `FlowEdge`s (with `edge_side`) into `ctx.pipeline_graph` (updating target `FlowNode.absence` after clearing resolved `UnknownReason` bits and updating `stats.nodes_emitted`), and return `(SliceEnvelope, Phase2Status)` (or populate a `MaterializedSliceSnapshot` helper).
    *   Expose a helper on `SlicerEngine` (`slice_phase1_with_snapshot`) that returns the internally computed `SliceGraph` alongside the wire `SliceEnvelope` (so `transport.rs` can store the snapshot in the 4-slot LRU even when `params.include_graph == false`).
    *   Update `expand_node` to optionally build and return an internal expanded `(SliceGraph, Vec<FileEntry>)` alongside `ExpandResult` when `parent_query_id` is supplied so `transport.rs` can merge the expanded subgraph into the cached snapshot.

#### 6. `engine/src/export/mod.rs` (Create)
*   **Type / Scope**: Rust Export Subsystem Entrypoint
*   **Proposed Changes**:
    *   Declare submodules `ir`, `sanitize`, `mermaid`, and `dot`.
    *   Define `ExportOptions { format, workspace_root, null_only, include_snippets, absolute_paths, max_nodes, max_edges, phase2_status, source_mode }` with `ExportOptions::for_format(...)` resolving tri-state `Option<bool>` snippets (`None => false` uniformly across all formats; `max_nodes = 200`, `max_edges = 800`, `max_label_chars = 256`).
    *   Expose single pure entrypoint `render_export(envelope: &SliceEnvelope, opts: &ExportOptions) -> ExportOutput` returning `(content, node_count, edge_count, truncated_nodes, truncated_edges, completeness_badges, warnings)` and emitting a source-free `tracing::info!` span with timing and count metrics only.

#### 7. `engine/src/export/sanitize.rs` (Create)
*   **Type / Scope**: Rust 2-Tier Path Privacy & Text Scrubbing Module
*   **Proposed Changes**:
    *   Implement `PathSanitizer::new(workspace_root: Option<&Path>, absolute_paths: bool)` normalizing `file://` URIs via percent-decoded `uri_to_path` into POSIX forward-slash paths.
    *   Classify paths inside `workspace_root` to workspace-root-relative POSIX paths (`src/auth/session.ts`) and paths outside `workspace_root` to `<external>/<sanitized-tail>` (keeping at most 2–3 trailing non-home components and stripping `/home/<user>`, `/Users/<user>`, and Windows drive prefixes).
    *   Provide `scrub_text(&self, text: &str) -> String` to strip raw `file://` URIs and `workspace_root` prefixes across `FlowNode.label`, `symbol`, `snippet`, `guard_text`, `HoverRisk.message`, and `UnknownBoundary`.

#### 8. `engine/src/export/ir.rs` (Create)
*   **Type / Scope**: Rust Canonical Export Graph IR (Shared by C.1 Mermaid/DOT and C.2 HTML+SVG)
*   **Proposed Changes**:
    *   Define `ExportGraph`, `ExportFile`, `ExportNode`, `ExportEdge`, `NodePartition` (`Upstream | Focal | Downstream`), `AbsenceClass` (`NonNull | Nullable | Absent`), and `ExportCompleteness`. Retain full inspector/legend fields on `ExportGraph`/`ExportNode`/`ExportFile` (`direction`, canonical `root_ids`, `file_index`, 0-based `range`, sorted deduplicated `unknown_reasons`, scrubbed `guard_text`, `danger_kind`, scrubbed `risk_messages`, `provenance`, scrubbed `unknown_boundaries`, `completeness_badges`, and export-time filter flags `null_only` / `exclude_tests`).
    *   Centralize UTF-8 char-safe single-line snippet trimming (`<= 120` chars) and `PathSanitizer::scrub_text` on `FlowNode.snippet` directly in `ir.rs` (never byte slicing), and cap `label` length (`<= 256` chars).
    *   Strip volatile `SliceEnvelope` metadata (`query_id`, `trace_id`, `phase1_ms`, `phase2_ms`).
    *   Normalize causal edge orientation (`origin -> consumer`): reverse `To` edges, retain `From` edges, and split `Both` using `FlowEdge.edge_side` (with provenance fallback only on legacy `--from-envelope` JSONs) so upstream flows into `root_ids` and downstream flows out of `root_ids`.
    *   Run `scc::tarjan_scc` over normalized edges solely for `cycle_group_id` (SCC membership), and run a single deterministic 3-color DFS feedback-arc pass (`White`/`Gray`/`Black` in canonical `N0..Nk` order, including self-loops `u == v`) to set `ExportEdge.is_back_edge` so removing `is_back_edge` always yields a DAG while keeping forward cycle edges top-to-bottom. Merge parallel `(from, to)` edges with deduplicated labels.
    *   Apply `null_only` causal-path retention (preserving connecting edges), enforce `max_nodes = 200` and `max_edges = 800` via deterministic BFS priority pruning from `root_ids` (dropping nodes with all incident edges), sort canonically via `BTreeMap` / total key `(norm_path, start.line, start.col, kind, label, raw_id)`, and assign safe IDs `N0..Nk`.
    *   Build separated completeness badges (`Budget-capped (partial)` for `truncation` vs. `Static only (LSP timed out after Nms)` / `Static only (no LSP attached)` / `Static only (--no-lsp)` / `Static only (LSP still settling)`).

#### 9. `engine/src/export/mermaid.rs` (Create)
*   **Type / Scope**: Rust GitHub-Flavored Mermaid Serializer
*   **Proposed Changes**:
    *   Render `flowchart TD` using **flat styled nodes with per-node `file:line` pills** (no `subgraph` blocks, per locked Option 1) and deterministic completeness header comments (`%% ...`).
    *   Escape all Mermaid-sensitive characters (`"`, `<`, `>`, `::`, `|`, `%%`, `;`, brackets, newlines) cleanly inside quoted labels; render pre-trimmed scrubbed `ExportNode.snippet` only when `opts.include_snippets == true`.
    *   Emit semantic `classDef` declarations and orthogonal text/icon badges matching `README.md` Coverage-Honesty terms (`NonNull`, `Nullable`, `Absent`, `Unverified`, `Guard`, `Local Risk`, `Frontier`), plus dashed cycle back-edges (`-. cycle .->`).
    *   Enforce the 45 KB character ceiling (below GitHub's 50 KB `maxTextSize`): drop optional snippets first if exceeded, then re-prune via BFS priority and append a size-guard notice to `warnings` and header comment.

#### 10. `engine/src/export/dot.rs` (Create)
*   **Type / Scope**: Rust Graphviz DOT Serializer (Unified for `export` and `slice --format dot`)
*   **Proposed Changes**:
    *   Render `digraph lineage` (`rankdir=TB`) with deterministic `subgraph cluster_f<idx>` file groupings (labeled with sanitized paths), `\`-first plain quoted string escaping (`\`, `"`, `\n`), Coverage-Honesty color/border/style attributes, dashed cycle back-edges, and completeness header comment + graph label.
    *   Handle 0-node and 1-node trivial graphs cleanly (valid single-node diagram with trivial-value note rather than malformed empty syntax).

#### 11. `engine/src/lib.rs` (Modify)
*   **Type / Scope**: Rust Crate Module Registry
*   **Proposed Changes**: Export `pub mod export;` so integration tests and `main.rs` consume the export module directly.

#### 12. `engine/src/protocol/transport.rs` (Modify)
*   **Type / Scope**: Rust NDJSON Stdio Transport, 4-Slot Snapshot LRU, and `lineage/export` Dispatch
*   **Proposed Changes**:
    *   Populate `EngineCapabilities.export_formats = vec!["mermaid".into(), "dot".into()]` in `build_init_result`.
    *   Add `QuerySnapshotCache` (4-slot bounded LRU keyed by `query_id`, mutex-protected) onto `TransportState`.
    *   Update `"lineage/slice"` to store the Phase 1 `MaterializedSliceSnapshot` into `QuerySnapshotCache` (keeping `env.graph = None` on the wire when `include_graph = false`), update it in-place when `Phase2Job` finishes, and log `tracing::warn!(reason = "phase2_queue_full", ...)` if `phase2_tx.try_send` fails.
    *   Update `"lineage/expand"` to merge the expanded subgraph into `QuerySnapshotCache` under `parent_query_id` / `parent_node_id` using collision-free `exp{k}_` node IDs and remapped `file_id`s.
    *   Add `"lineage/export"` handler: serve directly from `QuerySnapshotCache` when `query_id` hits and Phase 2 is settled (or render Phase 1 stamped `Static only (LSP still settling)` / await up to `lsp_timeout_ms` on the Phase 2 pool if Phase 2 is `InFlight`, avoiding duplicate LSP calls or dropped expanded frontiers); on LRU miss or fresh target run `slice_phase1_with_snapshot`, and if `phase2_opt.is_some()` and `!no_lsp`, dispatch an `ExportPhase2Job` onto the dedicated Phase 2 worker pool (with `phase2_ceiling_ms = lsp_timeout_ms.unwrap_or(500)` and per-call LSP timeout propagated up to `phase2_ceiling_ms`) that renders via `export::render_export` and writes `RpcResponse::ok(id, export_result)` directly to `out_tx` without blocking Phase 1 workers.

#### 13. `engine/src/cli.rs` & `engine/src/main.rs` (Modify)
*   **Type / Scope**: Rust Standalone CLI Interface
*   **Proposed Changes**:
    *   In `cli.rs`: add `Both` to `CliDirection`, split `CliFormat` into `SliceCliFormat { Json, Text, Dot }` and `ExportCliFormat { Mermaid, Dot }` (preventing invalid pairings like `export --format json`), and add `Commands::Export { target: Option<String>, from_envelope: Option<PathBuf>, format: ExportCliFormat, direction, workspace_root: Option<PathBuf>, exclude_tests, null_only, no_lsp, lsp_fixture, snippets, no_snippets, absolute_paths, out: Option<PathBuf> }` with `conflicts_with` between `--snippets` and `--no-snippets` resolving to `Option<bool>`.
    *   In `main.rs`: implement `Commands::Export` resolving default `--workspace-root` to nearest `.git` ancestor or `std::env::current_dir()` (never `file.parent()`), supporting `--from-envelope -` (stdin) or file path, running Phase 2 synchronously when `--lsp-fixture` is supplied and `!no_lsp`, rendering via `export::render_export`, and writing to `--out` or stdout.
    *   Fix `Commands::Slice` so `--lsp-fixture` sets `lsp_attached = true` and runs Phase 2, and replace the inline `SliceCliFormat::Dot` loop (`main.rs:195-210`) with `export::render_export` so `slice --format dot` and `export --format dot` are unified.

#### 14. `lua/lineage/config.lua` (Modify)
*   **Type / Scope**: Neovim Plugin Configuration Defaults
*   **Proposed Changes**:
    *   Add `export = { default_format = "mermaid", lsp_timeout_ms = 500, picker = true }` and `tree.keymaps = { export = "e" }` plus `hover.keymaps.export = "ge"` to `M.defaults`.

#### 15. `lua/lineage/client.lua` & `lua/lineage/debug.lua` (Modify)
*   **Type / Scope**: Neovim Sidecar Transport, Telemetry Client & Status Formatter
*   **Proposed Changes**:
    *   In `client.lua`: expose `M.capabilities()` returning the `capabilities` table received during `initialize` (including `export_formats`), clearing `state.capabilities = nil` and `state.buf` in `M.stop()` and on process exit so restarts against upgraded binaries never read stale capabilities.
    *   In `client.lua`: replace `state.buf = state.buf .. data` with a chunk-array accumulator (`table.concat` on `"\n"`) to avoid quadratic string reallocation on large export payloads, and reject pending callbacks cleanly if `vim.json.decode` fails rather than hanging silently.
    *   Extend `telemetry_ring` recording (`client.lua:369-383`) to capture `lineage/export` RTT, format, source mode, `output_bytes`, and `phase2_status` alongside `lineage/slice`.
    *   In `debug.lua`: update `show_status` (`debug.lua:64-80`) to format `lineage/export` telemetry entries (`format`, `source_mode`, `phase2_status`, `rtt_ms`, `output_bytes`) instead of printing `phase1=nilms`.

#### 16. `lua/lineage/export.lua` (Create)
*   **Type / Scope**: Neovim Export Orchestration & Delivery Module
*   **Proposed Changes**:
    *   Resolve export source by strict precedence: focused Tree buffer (`tree.get_export_context()`) > focused Hover float (`hover.get_export_context()`) > normal source buffer at cursor (default `direction = "to"`; Focus/Pin never hijacks cursor export).
    *   Gate capability checks inside `client.ensure_started(...)`, verify exact format membership (`vim.tbl_contains(caps.export_formats, format)`), intersect picker options with Lua-supported delivery formats (so C.1 Lua + C.2 Engine never dumps raw HTML to clipboard), and defensively catch `-32601 Method not found` with an actionable `:Lineage install!` notification on pre-Stage-C binaries.
    *   Implement `M.pick_and_export(opts)` using `vim.ui.select` when `config.export.picker == true` (or jumping straight to `config.export.default_format` when `picker == false`), keeping Tree/Hover open.
    *   Pass `include_snippets = nil` unless `--snippets` or `--no-snippets` was explicitly supplied; handle fresh-slice progress feedback (notify only if Phase 2 exceeds 150ms), capture cursor/buffer snapshot immediately before async RPC, and deliver output:
        *   No path: check `vim.fn.has("clipboard") == 1`; copy to `+` and notify with node/edge counts + completeness badges, or fall back to `"` with `vim.log.levels.WARN`.
        *   Explicit path: refuse overwrite if file exists unless `opts.force == true` (`:Lineage export!`), otherwise write UTF-8 atomically via `vim.uv` and notify path + counts.

#### 17. `lua/lineage/commands.lua` (Create) & `plugin/lineage.lua` (Modify) & `lua/lineage/init.lua` (Modify)
*   **Type / Scope**: Neovim Command Registration, Multi-Arg Parser, and `<Plug>` Mappings
*   **Proposed Changes**:
    *   Extract `:Lineage` dispatch, order-independent flag parsing (`--direction`, `--exclude-tests`, `--null-only`, `--no-lsp`, `--snippets`, `--no-snippets`, `--out`), and context-aware multi-arg tab completion into `lua/lineage/commands.lua`.
    *   Update `plugin/lineage.lua` to set `nargs = "*"`, delegate to `require("lineage.commands")`, preserve all existing subcommands and `:Lineage install!`, and register `<Plug>(LineageExport)`, `<Plug>(LineageExportMermaid)`, and `<Plug>(LineageExportDot)`.
    *   Add `M.export(opts)` entrypoint to `lua/lineage/init.lua`.

#### 18. `lua/lineage/ui/tree.lua` & `lua/lineage/ui/hover.lua` (Modify)
*   **Type / Scope**: Neovim Interactive Tree Panel & Hover Float Triggers
*   **Proposed Changes**:
    *   In `tree.lua`: pass `parent_query_id = state.query_id` and `parent_node_id = row.node_id` in `expand_row`'s `lineage/expand` params; expose `M.get_export_context()` returning `{ active_buf, query_id, origin_uri, origin_pos, direction, exclude_tests, null_only }`; bind configurable `e` (`config.tree.keymaps.export`) to `export.pick_and_export()` and advertise `e:export` in the header bar (zero collision with `nowait` `g`).
    *   In `hover.lua`: expose `M.get_export_context()` returning `{ active_buf, query_id, origin_uri, origin_pos, direction = "to", exclude_tests }`; bind configurable `ge` (`config.hover.keymaps.export`) to `export.pick_and_export()`.

#### 19. `engine/tests/export_golden.rs` (Create)
*   **Type / Scope**: Rust Unit, Invariant, Parity, and Golden Test Suite
*   **Proposed Changes**:
    *   Test Block 0 fixes: `ProjectField`/`ProjectIndex`/`UnwrapVariant` edges in `SliceGraph`, cross-file `To` callee nodes with real `file_id`, `tarjan_scc` back-edges (distinguishing cycles from diamond fan-in), typed `is_guard`/`is_danger_sink`/`HoverRisk.node_id`, Phase 2 LSP fixture graph merging, and `uri_to_path` `%20`/Windows normalization.
    *   Test 2-tier path sanitization (workspace-relative POSIX, `<external>/...`, scrubbing `file://` in labels/symbols/unknown boundaries) and escaping stress tests (TS generics `<T>`, Rust `::`/`<>`, Go `func`, quotes, backslashes, `%%`, `caller:1:0` IDs, reserved Mermaid keywords, 45 KB Mermaid ceiling).
    *   Test deterministic golden fixtures across TS, Python, Rust, and Go covering (a) linear chain, (b) diamond convergence, (c) recursive cycle/back-edge, and (d) 40–60 node multi-file graph, asserting `< 50ms` export time for `<= 100` nodes, `null_only` path preservation, and optional `dot -Tsvg` validation when `dot` is on `PATH`.
    *   Test byte-identical output between `Commands::Export` CLI and `lineage/export` NDJSON RPC over the same fixture/envelope, plus 4-slot LRU hit, `expand_node` collision-free merge, and LRU eviction fallback.

#### 20. `tests/test_export.lua` (Create)
*   **Type / Scope**: Headless Neovim Integration Test Suite
*   **Proposed Changes**:
    *   Test multi-arg `:Lineage export` command parsing and completion (`nargs = "*"`), flag order independence, and overwrite protection (`:Lineage export` refuses vs. `:Lineage export!` succeeds).
    *   Test live Tree Panel export (`e` / `:Lineage export mermaid`) with expanded frontier + mock LSP Phase 2 delta, verifying exact snapshot export and zero `g` keymap collision.
    *   Test Hover (`ge`), fresh source-buffer export with mock LSP 500ms settle vs. `--no-lsp` vs. timeout completeness badge, clipboard `+` -> `"` `WARN` fallback, and pre-Stage-C capability mismatch (`export_formats` absent / `-32601`) error handling.

#### 21. `openspec/specs/graph-export/spec.md` (Create) & Spec/Doc Updates (Modify)
*   **Type / Scope**: OpenSpec Capability Specifications & User Documentation
*   **Proposed Changes**:
    *   Create `openspec/specs/graph-export/spec.md` specifying the shared Export IR, Mermaid/DOT format contracts, 2-tier path sanitization, 4-slot LRU snapshot merge, `lineage/export` RPC, and CLI/Neovim delivery rules.
    *   Update delta requirements in `openspec/specs/core-ir-and-slicer/spec.md` (Block 0 `SliceGraph` completeness and additive typed fields), `openspec/specs/observability-and-cli/spec.md` (`lineage-engine export`, `Both`, `--workspace-root`, unified `slice --format dot`, source-free export tracing), and `openspec/specs/neovim-ui/spec.md` (`:Lineage export`, `e`/`ge`, `<Plug>(LineageExport*)`).
    *   Update `README.md` and `doc/lineage.txt` with `:Lineage export` commands, `<Plug>` mappings, Tree `e` / Hover `ge` keymaps, `export` config defaults, CLI `export` examples, and fix the stale `README.md:154` note claiming Go has no Tree-sitter adapter.

---

### Verification Plan

*   **Compilation & Build (Zero New Crates + Binary Delta Check):**
    *   `cd engine && cargo build --release && cargo check --features mimalloc`
    *   Verify stripped `target/release/lineage-engine` size delta stays `< 200 KB` over pre-Stage-C `main` baseline.
*   **Rust Unit, Block 0, Escaping, Parity, and Golden Tests:**
    *   `cd engine && cargo test` (must pass all existing suites—`m0_core`, `golden_typescript`, `golden_python`, `golden_rust`, `golden_go`, `golden_nullability`, `ir_go_validation`, `ripgrep_callers`, `fuzz_broken_syntax`—plus new `export_golden`).
    *   Run optional `dot -Tsvg` syntax validation on generated DOT fixtures when Graphviz is available locally.
*   **Static Analysis & Formatting:**
    *   `cd engine && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
    *   `stylua --check plugin lua tests`
*   **Headless Neovim E2E Integration Tests (Neovim 0.10 & 0.11):**
    *   `for t in tests/test_*.lua; do nvim --headless -u tests/minimal_init.lua -l "$t"; done` (covering `tests/test_export.lua` and confirming zero regressions in `test_hover.lua`, `test_tree.lua`, `test_two_phase_lsp.lua`, `test_install.lua`).
*   **Manual Visual Sign-Off:**
    *   Paste generated Mermaid diagrams (linear, diamond convergence, recursive cycle, and 40–60 node multi-file slice across TS/Python/Rust/Go) into GitHub Markdown preview to verify zero parse errors, clean escaping, and legible Coverage-Honesty styling.

--------------------------------------------------------------------------------

*Drafted with EGM*
