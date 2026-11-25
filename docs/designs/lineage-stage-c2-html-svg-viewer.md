# lineage.nvim Stage C.2: Offline Interactive HTML+SVG Viewer

*   **Author:** cristianfritis
*   **Status:** Proposed
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

Stage C.2 extends `lineage.nvim`'s export pipeline with a 100% offline, deterministic interactive HTML+SVG viewer (`format = "html"`) built directly on top of Stage C.1's shared `ExportGraph` intermediate representation and `lineage/export` RPC ([lineage-stage-c1-graph-exporters.md](./lineage-stage-c1-graph-exporters.md)). Stage C.1—including its six pre-freeze contract folds (`C1-1`–`C1-6`)—must be implemented, verified, and frozen before Stage C.2 code is written.

Stage C.2 adds zero new Cargo crates and preserves `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1`. Inside `lineage-engine`, a pure-Rust layout pass (`engine/src/export/layout.rs`) computes deterministic integer SVG coordinates in `< 50ms` for graphs up to 200 nodes and 800 edges, and an HTML serializer (`engine/src/export/html.rs`) bundles real sibling assets (`engine/src/export/assets/{viewer.css, viewer.js}` via `include_str!`) into a strict-CSP single-file HTML document containing zero wall-clock timestamps and zero host workspace folder names. The engine remains stateless by returning raw HTML in `ExportResult.content` (and writing to `stdout` or `--out <path>` in the standalone CLI), while Neovim (`lua/lineage/export_cache.lua` and `lua/lineage/export.lua`) owns the `0700`/`0600` content-addressed local cache and headless/SSH/WSL-safe browser launch via `vim.ui.open`.

## The Problem

Even after Stage C.1 ships Mermaid and Graphviz DOT exporters alongside the Neovim Interactive Tree Panel, medium-to-large nullability and dataflow investigations still hit three practical walls:

1. **Mermaid Breaks Down Past ~20–25 Nodes:**
   GitHub-flavored Mermaid works well for concise PR snippets, but once a multi-file slice grows to 30–120 nodes, flat `flowchart TD` diagrams become cramped, hard to pan, and lack interactive path isolation, semantic filtering, or clickable per-file grouping.
2. **Graphviz DOT Is Static and Requires External Tooling:**
   Rendering DOT diagrams requires a local `graphviz` (`dot`) installation and produces static SVGs/PNGs with no pan/zoom focus, no click-to-highlight causal cones, no snippet inspector, and no click-to-copy `file:line` jump-back to Neovim.
3. **Tree Panel Obscures Global Convergence and Hourglass Topologies:**
   In multi-file diamond convergences (multiple `null`/`None`/`nil` producers feeding one shared sink), recursive cycles, and `Both` slices (upstream origins converging onto the cursor symbol waist and fanning out to downstream callers), the Tree Panel repeats converging subtrees under each branch and cannot display global topology at a glance.

Stage C.2 solves these gaps without adding runtime dependencies, network calls, or a browser layout library by pairing a bounded pure-Rust layered-DAG SVG coordinate generator with a small, self-contained vanilla JS viewer (`~250–350 LOC`) that runs directly from `file://` under strict Content Security Policy.

## Goals

*   **Zero-Rework Plug-In Over Frozen Stage C.1:** Consume Stage C.1's `ExportGraph` and `ExportOptions` exclusively (never bypassing to `&SliceEnvelope`), registering `"html"` in `EngineCapabilities.export_formats`, `ExportCliFormat`, and `lineage/export` with zero breaking schema or protocol changes.
*   **Deterministic, Overlap-Free Pure-Rust SVG Layout (`< 50ms`):** Lay out graphs up to 200 nodes and 800 edges (15–120 node sweet spot) in pure Rust with integer `i32` coordinates, zero node-node or node-gutter bounding-box overlaps, strict top-to-bottom rank order for non-feedback edges, focal waist pinning on `Both` slices, bounded dummy-node waypoints (`<= 600`), a shared `O(k log k)` interval lane packer serving both the right-hand cycle gutter (DFS feedback back-edges only) and a left-hand overflow long-span gutter routed strictly through inter-rank horizontal corridors, and byte-identical HTML output for identical `ExportGraph` inputs.
*   **Strict-CSP, XSS-Safe, `file://`-Compatible Offline Viewer:** Embed modular vanilla JS and CSS (`engine/src/export/assets/{viewer.css, viewer.js}` via `include_str!`) with zero network calls, zero `localStorage`/cookies, strict `<meta>` CSP (`default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; object-src 'none';`), matching Rust and JS JSON Unicode escaping (`<`, `>`, `&`, `U+2028`, `U+2029`), and DOM construction exclusively through `textContent` and SVG DOM APIs (never `innerHTML`).
*   **Destination-Aware Snippet Safety and Leak-Free `"Save Without Snippets"`:** Keep `lineage-engine`'s uniform safe default (`include_snippets: None => false`) across all formats and CLI/explicit-file exports, passing `include_snippets = true` only from Neovim on ephemeral private-cache HTML opens (unless `--no-snippets` is passed). Isolate all snippets inside a single `<script type="application/json" id="lineage-snippets">` island (zero snippets in SVG markup or attributes) so the browser `"Save Without Snippets"` button clones a pre-boot `pristineRoot` snapshot, empties `#lineage-snippets`, stamps `Snippets removed`, and emits a clean, XSS-safe standalone HTML file.
*   **Focused Interactive Debugging Ergonomics:** Provide pan/zoom, `"Fit View"` / `"Fit Active"` / `"Center Focus"`, click-node transitive causal cone focus-dimming, OR-composed `Null-only`/`Guards`/`Unverified` toggles with live counts (respecting export-time `--null-only`/`--exclude-tests` as locked pills), sequential `palette[file_idx % N]` clickable per-file legend, collapsed-by-default side inspector, inverted `file://`-safe 3-step `path:line` click-to-copy, and OS-adaptive dark/light CSS with an in-memory toolbar toggle.
*   **Safe Local Cache, Delivery Symmetry, and Observability:** Keep `lineage-engine` stateless (`ExportResult.content` on RPC; `stdout` / `--out` on CLI); manage a `0700`/`0600`-from-birth content-addressed cache (`lineage-<slug>-<sha12>.html`, count-only 20 most recent files by `mtime`, `:Lineage export clean`) in `lua/lineage/export_cache.lua`; save explicit `:Lineage export html <path>` silently (with opt-in `--open`); guard `vim.ui.open` on headless/SSH Linux without blocking WSL; add `<Plug>(LineageExportHtml)`; enforce exact format membership in `capabilities.export_formats`; and emit source-free layout telemetry plus `:checkhealth lineage` cache/opener diagnostics.
*   **Lightweight Automated Verification and Binary Budget:** Verify layout geometry, determinism, `< 50ms` performance, CSP/XSS safety, and redaction parity in `cargo test` plus optional `node --test` on `viewer.js` (zero Playwright/Chromium/Graphviz CI dependency), keeping release binary growth `< 100 KB` over Stage C.1.

## Non-Goals

*   **Implementing Stage C.2 Before Stage C.1 Freezes:** Stage C.1 (with pre-freeze folds `C1-1`–`C1-6`) must be implemented and passing all tests before Stage C.2 coding starts.
*   **External Dependencies or Browser Layout Engines:** No new Cargo crates, no bundled `dagre`/`elkjs`/external fonts/CDNs, and no Graphviz or headless browser requirements in CI.
*   **Enclosing SVG Per-File Cluster Rectangles:** File grouping uses deterministic sequential accent strips/pills (`palette[file_idx % N]`) and a clickable per-file legend rather than nested cluster boxes.
*   **Reverse Browser-to-Neovim RPC, Persistence, or Host Metadata:** No local HTTP/WebSocket bridge back to Neovim, no `localStorage`/cookies, no wall-clock timestamps or host workspace folder names inside HTML, and no optional quickfix causal-chain copy button.
*   **Multi-Line Source Context or 7-Day Cache TTL Expiry:** Stage C.2 reuses Stage C.1's single trimmed statement line (`<= 120` chars) and uses count-only cache retention (`20` most recent files by `mtime`) without a separate age-TTL eviction policy.
*   **Breaking Wire or Schema Changes:** `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` remain backward-compatible.

## Open Questions

*   None — all Stage C.2 design decisions and Stage C.1 pre-freeze folds (`C1-1`–`C1-6`) are resolved.

## Critical User Journeys (CUJs)

### CUJ 1: Inspecting a Multi-File Diamond or `Both` Slice Locally in the Browser from Neovim

*   **Given**: A developer has a 50-node multi-file `Both` slice (or converging diamond with a recursive cycle) open in the Tree Panel—or their cursor on a symbol in a normal buffer—on desktop Linux, macOS, Windows, or WSL with Stage C.2 `lineage-engine` installed (`"html"` in `EngineCapabilities.export_formats`).
*   **When**: The developer runs `:Lineage export html` (or presses `e` in Tree / `ge` in Hover and picks `HTML (browser)`).
*   **Then**:
    *   `lua/lineage/export.lua` validates `"html"` membership in `capabilities.export_formats` and calls `lineage/export` with `format = "html"` and `include_snippets = true` (since this is an ephemeral private-cache open and `--no-snippets` was not passed).
    *   `lineage-engine` lays out the `ExportGraph` in `< 50ms` with integer `i32` coordinates, zero overlapping cards or gutters, focal cursor node pinned at the `Both` waist (`▲ Upstream` / `▼ Downstream`), forward cycle edges flowing top-to-bottom, and DFS feedback back-edges routed through the right-hand cycle gutter, returning the deterministic HTML document in `ExportResult.content`.
    *   `lua/lineage/export_cache.lua` writes the HTML atomically with `0600`-from-birth permissions into `stdpath("cache")/lineage/exports/lineage-<slug>-<sha12>.html` (touching `mtime` on identical re-export and lazily keeping the 20 most recent files), launches the browser via `vim.ui.open`, and notifies the user with node/edge counts, `snippets ON`, and the exact written path.
    *   In the browser (`file://`), the viewer opens centered on the focal node (`Snippets: ON (local)` pill visible), letting the developer pan/zoom, click any node to highlight its transitive causal cone and inspect its trimmed snippet, toggle `Null-only`/`Guards`/`Unverified` or click a file in the legend, and click `Copy path:line` using the gesture-safe `file://` copy fallback.

### CUJ 2: Producing a Snippet-Free Shareable HTML Artifact (From Browser or Explicit Path)

*   **Given**: A developer either (a) is viewing a local cached HTML slice (`Snippets: ON (local)`) and wants to attach the interactive graph to a bug ticket without raw source lines, or (b) runs `:Lineage export html docs/auth-flow.html` from Neovim.
*   **When**: In case (a), the developer clicks **`Save Without Snippets`** in the viewer toolbar; in case (b), the developer executes `:Lineage export html docs/auth-flow.html` (without `--snippets` and without `--open`).
*   **Then**:
    *   In case (a), the viewer clones its pre-boot `pristineRoot` snapshot (zero pan/zoom/filter/inspector DOM leakage), empties `#lineage-snippets` to `"{}"`, stamps the **`Snippets removed`** header badge, re-serializes JSON with the Unicode escaper (`<`, `>`, `&`, `U+2028`, `U+2029`) so untrusted repo strings cannot inject `</script>`, prepends `<!DOCTYPE html>\n`, and triggers a `Blob` download (`*-no-snippets.html`) with delayed URL revocation so Firefox and Chromium both complete the save cleanly.
    *   In case (b), `lineage.nvim` refuses to overwrite an existing `docs/auth-flow.html` unless `:Lineage export!` is used, calls `lineage/export` with `include_snippets = nil` (engine resolves to `false`), writes the snippet-free deterministic HTML silently to `docs/auth-flow.html` without opening a browser, and notifies the user with the path and counts. Passing `--open` launches it immediately; passing `--snippets` explicitly includes snippets.

### CUJ 3: Headless / SSH Session and Standalone CLI Export

*   **Given**: A developer works over SSH on a Linux server without `DISPLAY` or `WAYLAND_DISPLAY`, or a script invokes `lineage-engine export src/auth.ts:42:8 --format html --direction both --no-lsp --out /tmp/flow.html`.
*   **When**: The developer runs `:Lineage export html` over headless SSH, or runs the CLI command.
*   **Then**:
    *   Over headless SSH, `lua/lineage/export_cache.lua` writes the `0600` cached HTML file, detects headless/SSH-without-display before calling `vim.ui.open` (preventing `xdg-open` from launching `lynx`/`w3m` or corrupting the TTY), and prints the exact file path cleanly via `vim.notify`.
    *   In the standalone CLI, `lineage-engine` writes deterministic, snippet-free HTML (`include_snippets = false` unless `--snippets` is passed) to `--out /tmp/flow.html` (or `stdout` when `--out` is omitted), never opens a browser, and never touches Neovim's cache directory.

### CUJ 4: Mixed-Version Capability Mismatch and Cache Cleanup

*   **Given**: A user upgrades `lineage.nvim` to Stage C.2 while still running a Stage C.1 `lineage-engine` binary (`EngineCapabilities.export_formats = ["mermaid", "dot"]`), or wants to purge cached HTML files.
*   **When**: The user runs `:Lineage export html` on the older binary, or runs `:Lineage export clean`.
*   **Then**:
    *   On the Stage C.1 binary, `lua/lineage/export.lua` checks exact membership (`"html" ∈ capabilities.export_formats`) after `client.ensure_started(...)` and fails fast with a clear notification (`lineage-engine does not support format 'html' (supported: mermaid, dot). Run :Lineage install! to update.`).
    *   On `:Lineage export clean`, `lua/lineage/commands.lua` dispatches `clean` before format parsing, deletes only regular non-symlink `lineage-*.html` files under `stdpath("cache")/lineage/exports/`, handles a missing directory without error, and notifies with the exact removed file count and bytes reclaimed.

## Technical Plan (Architecture)

Stage C.2 plugs directly into Stage C.1's `engine/src/export/` pipeline and `lua/lineage/export.lua` delivery layer with zero changes to `PROTOCOL_VERSION = 1`, `SCHEMA_VERSION = 1`, or `CoreIR`. Every new Rust component consumes Stage C.1's canonical `&ExportGraph` and `&ExportOptions` exclusively (never raw `&SliceEnvelope`), keeping `lineage-engine` stateless while isolating browser cache lifecycle in Neovim.

```text
+---------------------------------------------------------------------------------+
|                         Neovim 0.10+ (lineage.nvim, Lua)                        |
|                                                                                 |
|  [:Lineage export html [path] [flags]]  [:Lineage export clean]  [e / ge]       |
|                     |                              |                 |          |
|                     v                              v                 v          |
|        [lua/lineage/commands.lua] ---> [lua/lineage/export.lua]                 |
|         - parses --open / --no-open     - checks "html" in export_formats       |
|         - dispatches `clean` first      - bare html: include_snippets = true    |
|                                         - explicit path: include_snippets = nil |
|                                                    |                            |
|                                                    +---> [export_cache.lua]     |
|                                                          - 0700 dir / 0600 wx   |
|                                                          - sha12 content name   |
|                                                          - count-only 20 prune  |
|                                                          - SSH/headless guard   |
+----------------------------------------------------|----------------------------+
                                                     | NDJSON stdio (lineage/export)
+----------------------------------------------------v----------------------------+
|                       lineage-engine (Stateless Rust)                           |
|                                                                                 |
|  [ExportGraph (frozen C.1 ir.rs)]                                               |
|    - <= 200 nodes, <= 800 edges, DFS is_back_edge, Both edge_side/partition     |
|    - scrubbed <= 120-char snippets, full inspector fields, zero timestamps      |
|                     |                                                           |
|                     v                                                           |
|  [engine/src/export/layout.rs]  (Pure i32 Geometry, < 50ms, zero HTML strings)  |
|    1. WCC split + 0/1-node handling                                             |
|    2. Longest-path / Both waist ranking (origin -> consumer)                    |
|    3. Bounded dummy waypoints (<= 600) + 4-8 barycenter sweeps                  |
|    4. Shared O(k log k) pack_gutter_lanes (right cycles + left overflow)        |
|    5. Inter-rank horizontal corridors + card port distribution                  |
|                     |                                                           |
|                     v                                                           |
|  [engine/src/export/html.rs] <--- include_str! --- [assets/{viewer.css,js}]     |
|    - strict <meta> CSP, XML-escaped SVG, sequential palette[file_idx % 12]      |
|    - dual JSON islands (#lineage-meta + #lineage-snippets, Unicode-escaped)     |
|    - returns ExportResult.content (RPC) or stdout / --out (CLI)                 |
+---------------------------------------------------------------------------------+
```

### 1. Pure-Rust Layered-DAG Layout Engine (`engine/src/export/layout.rs`)

`layout.rs` is a pure geometry module over `(&ExportGraph, &ExportOptions) -> SvgLayout` containing zero HTML or SVG string formatting. All coordinates use deterministic integer pixels (`i32`) so output is byte-identical across platforms without floating-point formatting drift.

*   **Fixed Card & Spacing Constants:**
    *   `CARD_W = 240`, `CARD_H = 72`, `H_GAP = 40`, `V_GAP = 56`, `DUMMY_W = 24`
    *   `GUTTER_GAP = 28`, `LANE_PITCH = 14`, `CORRIDOR_STEP = 10`, `COMPONENT_GAP_Y = 72`, `ARROW_PULLBACK = 8`, `MARGIN = 32`
*   **Deterministic 7-Stage Pipeline:**
    1.  **Degenerate Cases & WCC Partitioning:**
        *   0-node graph: returns `SvgLayout` with non-zero `viewBox` (`640 x 240`) and `empty_state = true` so JS fit-to-screen never divides by zero.
        *   1-node graph: centers the single card in `640 x 240` with `trivial_single = true`.
        *   Multi-node graph: partitions weakly connected components (WCC), ordering the component containing primary `root_ids[0]` first and remaining components by lowest canonical node ID (`N0..Nk`). Each component is laid out independently and stacked vertically with `COMPONENT_GAP_Y`.
    2.  **Forward DAG vs. Feedback Back-Edge Split:**
        *   Reads Stage C.1's `ExportEdge.is_back_edge` (computed via deterministic 3-color DFS on the normalized causal graph in `ir.rs`, distinct from SCC `cycle_group_id`) plus explicit `u == v` self-loops.
        *   Forward edges (`!is_back_edge && u != v`) are guaranteed acyclic and enter topological ranking; multi-node cycle forward edges (`A -> B -> C`) flow normally top-to-bottom while retaining `cycle_group_id` for subtle cycle badges.
        *   Feedback back-edges (`is_back_edge && u != v`) route to the right-hand cycle gutter; self-loops (`u == v`) route via a dedicated right-side local loop with reserved horizontal clearance (or gutter lane 0 if adjacent to a sibling card).
    3.  **Rank Assignment (`To`, `From`, and `Both` Waist Pinning):**
        *   For `To` and `From`: compute standard longest-path topological ranks (`rank[v] = max(rank[u] + 1)` from zero-indegree sources).
        *   For `Both`: if `root_ids` is non-empty and no forward DAG edge connects two roots, pin all `root_ids` at `rank = 0`, assign upstream nodes via reverse longest-path (`rank[u] = min(rank[v] - 1)`), assign downstream nodes via forward longest-path (`rank[v] = max(rank[u] + 1)`), and shift all ranks by `-min_rank` so ranks start at `0` with `focal_rank = -min_rank`. If `root_ids` is empty/pruned (e.g., under `--null-only`) or two roots share a forward edge, fall back cleanly to standard longest-path without inverting edges.
        *   Invariant asserted in tests: `rank[u] < rank[v]` for every forward edge `(u, v)`.
    4.  **Bounded Dummy-Node Waypoints (`<= 600`) & Overflow Classification:**
        *   Collect all multi-rank forward edges (`span = rank[v] - rank[u] >= 2`) sorted canonically by `(span, from_id, to_id)` (shortest spans first so local diamonds get interior waypoints before extreme spans).
        *   Insert `span - 1` dummy nodes per edge while `dummy_count + (span - 1) <= 600`.
        *   Any remaining multi-rank forward edge that would exceed the `600`-dummy budget is classified `OverflowLeft` and routed through the left-hand long-span bypass gutter—never drawn as a raw diagonal cutting through intermediate node cards.
    5.  **Bounded Crossing Minimization & `x`-Coordinate Placement:**
        *   Run 6 alternating top-down / bottom-up barycenter sweeps (within the 4–8 sweep budget, tie-broken strictly by canonical ID) across each rank's real + dummy nodes.
        *   Assign pixel `x` coordinates via 2-pass centered median-`x` relaxation per rank, clamping left-to-right (`x[i] >= x[i-1] + width[i-1] + H_GAP`) so adjacent cards and dummy waypoints never overlap horizontally.
    6.  **Shared `O(k log k)` Interval Lane Packer (`pack_gutter_lanes`) & Inter-Rank Corridors:**
        *   One deterministic greedy interval coloring function (`pack_gutter_lanes`) serves both gutters: sort gutter intervals by `(y_top, y_bottom, from_id, to_id)` and assign each edge the lowest vertical lane index whose previous interval ends above `y_top`.
        *   Right-hand cycle gutter lanes sit at `x = max_card_right + GUTTER_GAP + lane * LANE_PITCH`.
        *   Left-hand overflow long-span lanes sit at `x = min_card_left - GUTTER_GAP - lane * LANE_PITCH` (followed by a uniform horizontal shift so all coordinates start at `x >= MARGIN`).
        *   **Inter-Rank Horizontal Corridors:** Every gutter entry and exit travels vertically from the source card's bottom border into the inter-rank gap below `rank(u)` (`y_corridor = rank_bottom[u] + CORRIDOR_STEP * slot`), horizontally to its assigned gutter lane `x`, vertically along the gutter lane, horizontally inside the inter-rank gap above `rank(v)`, and vertically into the target card's top border. Inter-rank vertical spacing expands automatically if a corridor holds many concurrent horizontal tracks, guaranteeing zero segment-vs-card-AABB intersections.
    7.  **Port Distribution, Arrowhead Pullback & Telemetry:**
        *   Sort incoming ports along each card's top border and outgoing ports along its bottom border by neighbor `x` coordinate, spacing evenly within `[card_left + 20, card_right - 20]`.
        *   Pull back every edge endpoint by `ARROW_PULLBACK` outside the card border so SVG `<marker>` arrowheads never overlap card strokes.
        *   Emit `LayoutMetrics` (`rank_count`, `dummy_nodes_added`, `dummy_cap_hit`, `feedback_back_edges`, `overflow_edges`, `crossing_sweeps`, `svg_width`, `svg_height`).

### 2. Single-File HTML Assembly, Security & Embedded Assets (`engine/src/export/html.rs` & `assets/`)

Static CSS and vanilla JS live in real sibling files (`engine/src/export/assets/viewer.css` and `engine/src/export/assets/viewer.js`) embedded at compile time via `include_str!` (zero runtime file I/O, zero new crates, directly testable via `node --test`).

*   **Strict Offline CSP & Zero Host/Time Metadata:**
    *   Every document begins with `<!DOCTYPE html>` and `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; object-src 'none';">` (no `frame-ancestors`/`report-uri` inside `<meta>`, no `img-src data:` because all icons are inline SVG).
    *   Zero wall-clock timestamps, zero host workspace folder names, and zero `CARGO_PKG_VERSION` strings appear in the HTML body so identical `ExportGraph` inputs produce byte-identical HTML across CLI, RPC, and OSes.
    *   Descriptive `<title>`: `lineage: <symbol> (<direction>) — <norm_path>:<line>` (using sanitized relative/`<external>` path and 1-based line).
*   **Dual Isolated JSON Islands & Escaping Invariants:**
    *   **XML/HTML escaping:** All text interpolated into `<title>`, SVG `<text>`, and attributes passes through XML entity escaping (`&`, `<`, `>`, `"`, `'`) after stripping ASCII control characters (`0x00..=0x1F` except `\t` and `\n`).
    *   **JSON Unicode escaping:** Both Rust (`html.rs`) and JS (`safeJsonForScript` in `viewer.js`) post-process JSON strings to escape `<` (`\u003c`), `>` (`\u003e`), `&` (`\u0026`), `U+2028` (`\u2028`), and `U+2029` (`\u2029`), neutralizing `</script>` and `<!--` breakouts on untrusted repository strings.
    *   **`<script type="application/json" id="lineage-meta">` (`viewer_schema_version: 1`):** Holds nodes (`id`, `symbol`, `label`, `file_index`, `line_1based`, `col_1based`, `absence_class`, orthogonal booleans `is_unverified`/`is_guard`/`is_danger_sink`/`is_frontier`/`is_root`/`in_cycle`, `unknown_reasons`, scrubbed `guard_text`, `danger_kind`, scrubbed `risk_messages`, `provenance`, `partition`, `bounds: {x,y,w,h}`, `topo_index`), edges (`id`, `from`, `to`, `label`, `is_back_edge`, `cycle_group_id`), files (`index`, `norm_path`, `color_idx = index % 12`), `direction`, `root_ids`, `completeness_badges`, `truncation`, `export_filters: { null_only, exclude_tests }`, and `snippets_removed: bool`.
    *   **`<script type="application/json" id="lineage-snippets">`:** Holds `{ "<node_id>": "<scrubbed <=120-char trimmed line>" }` when `opts.include_snippets == true`, or `{}` when `false`. Zero raw snippets appear inside SVG `<text>`, `<title>`, or `data-*` attributes—card geometry is identical whether snippets are included or omitted.
*   **Visual Channel Separation & Accessibility (`viewer.css` + SVG cards):**
    *   **Unclipped Header Slot + Clipped Variable Text:** Each `240 x 72` card reserves a fixed top badge row (outside `<clipPath>`) showing non-color Coverage-Honesty glyphs matching `README.md` (`● NonNull`, `◐ Nullable`, `○ Absent`, `? Unverified`, `! Risk`, guard icon) plus stroke patterns (dashed border for `Unverified`, heavy double border for `Local Risk`, outer halo + `FOCUS` badge for `is_root`). Only the variable-length symbol and `file:line` lines sit inside `<clipPath id="clip-N{k}">`, pre-formatted in Rust with UTF-8 char-safe middle-elision (`src/…/session.ts:42`) so distinguishing suffixes remain legible.
    *   **Sequential Colorblind-Safe File Palette:** Files are assigned `color_idx = file_idx % 12` in deterministic sorted `norm_path` order, mapped to 12 curated CSS custom properties (`--file-color-0..11`) meeting WCAG AA contrast (`>= 4.5:1` text, `>= 3:1` strokes) in both dark and light themes. File colors appear strictly on each card's 4px left accent bar and the file legend swatch—never on status borders or card fills.
    *   **`Both` & Cycle Visual Cues:** `Both` slices display subtle `▲ Upstream origins` / `▼ Downstream consumers` band markers around `focal_rank`; cycle back-edges in the right gutter carry explicit upward arrowheads and a `Cycle back-edge` legend entry.
    *   **Header Trust Bar:** Displays slice direction, node/edge counts, completeness badges (`Budget-capped (partial)`, `Static only (...)`), truncation banner (`Showing 200 of N nodes` when applicable), and a persistent snippet pill (`Snippets: ON (local)` vs. `Snippets: OFF` vs. `Snippets removed`).
*   **Modular Vanilla JS (`engine/src/export/assets/viewer.js`, `~250–350 LOC`):**
    *   **Zero `innerHTML`:** All dynamic UI elements use `document.createElement` / `document.createElementNS` and `.textContent`.
    *   **Pre-Boot Pristine Clone:** Line 1 of initialization captures `const pristineRoot = document.documentElement.cloneNode(true);` before applying theme attributes, initial viewport transforms, active overlays, or inspector DOM.
    *   **Pure Testable Core (`module.exports` guard for `node --test`):** Separates pure functions—`safeJsonForScript`, `computeCausalCone(meta, nodeId)`, `computeNullPaths(meta)`, `evaluateFilters(meta, state)`, `buildSnippetFreeHtml(pristineRoot)`—from thin DOM event wiring.
    *   **Pan/Zoom & Initial Framing:** Wheel zoom, pointer drag pan, and toolbar `+` / `-` / `Fit View` / `Fit Active` (`f`) / `Center Focus`. On load, fits the full graph when `node_count <= 25`, or centers on the primary focal node at `1.0` scale when `node_count > 25`.
    *   **Focus-Dimming & Filter Algebra:**
        *   Semantic toggles (`Null-only`, `Guards`, `Unverified`) show live node counts (`Unverified (0)` disabled when 0) and compose via **OR union** (`Null-only` computes full causal paths reaching/leaving `Absent`/`Nullable` nodes via BFS, matching Stage C.1 `ir.rs`). Pre-filtered exports render locked badges (`Exported: Null-only`, `Tests excluded`) rather than `OFF` toggles.
        *   Clickable scrollable/collapsible file legend toggles per-file filtering (intersected with active semantic filters when both are active).
        *   Clicking a node highlights its transitive causal cone (ancestors + descendants via cycle-safe BFS), copies/elevates active edge paths into a top-most `<g id="active-overlay">` (never mutating `pristineRoot`), and opens the collapsed-by-default side inspector. `Esc` or blank-canvas click clears node selection; `Reset` (`r`) clears all filters.
    *   **Inverted `file://`-Safe 3-Step `Copy path:line`:**
        *   Synchronously checks `window.isSecureContext && navigator.clipboard && navigator.clipboard.writeText`.
        *   On non-secure `file://` origins, runs Step 2 **synchronously inside the click handler** (preserving browser user-gesture activation): fixed off-screen `<textarea readonly>` (`position: fixed; top: 0; left: 0; opacity: 0; focus({ preventScroll: true })`) + `.select()` + `document.execCommand("copy")`.
        *   If Step 1 rejects or Step 2 returns `false`, displays Step 3: an inline pre-selected `<input readonly>` toast in the inspector (`"Clipboard blocked on file:// — press Ctrl+C / ⌘C"`), avoiding blocking `window.prompt`. Successful copies show a `1.5s` `"Copied!"` confirmation.
    *   **`"Save Without Snippets"` (`Blob` Download):** Hidden/disabled when `#lineage-snippets` is already `{}`. On click, clones `pristineRoot`, sets `#lineage-snippets` `textContent` to `"{}"`, parses `#lineage-meta`, sets `meta.snippets_removed = true`, writes `safeJsonForScript(meta)` back into `#lineage-meta`, updates the header pill text to `Snippets removed` and removes the button on the clone, prepends `"<!DOCTYPE html>\n"`, triggers `<a download="<slug>-no-snippets.html">` over `URL.createObjectURL(blob)`, and calls `setTimeout(() => URL.revokeObjectURL(url), 1000)`.
    *   **Dark / Light Theme:** Defaults to CSS `@media (prefers-color-scheme: dark)` / light variables, with an in-memory toolbar toggle setting `data-theme="dark" | "light"` on `<html>` (zero `localStorage`/cookies).

### 3. CLI & Neovim Delivery Matrix, Private Cache, and Capability Rollout

*   **CLI vs. Neovim Flag & Delivery Matrix:**

| Invocation | `include_snippets` Default | Destination | Browser Auto-Open |
| --- | --- | --- | --- |
| `lineage-engine export ... --format html` (CLI) | `false` (`--snippets` opts in) | `stdout` (or `--out <path>`) | Never |
| `:Lineage export html` (Neovim bare / `e` / `ge` picker) | `true` (`--no-snippets` opts out) | Private cache `stdpath("cache")/lineage/exports/lineage-<slug>-<sha12>.html` | Yes (unless `--no-open`, `config.export.html.open == false`, or headless/SSH) |
| `:Lineage export html <path>` or `--out <path>` (Neovim explicit) | `false` (`--snippets` opts in) | Explicit `<path>` (refuses overwrite without `:Lineage export!`) | No (saves silently; `--open` opts in) |
| `:Lineage export clean` (Neovim maintenance) | N/A (rejects export flags) | Deletes cached `lineage-*.html` only | Never |

*   **Command & Config Surface (`lua/lineage/commands.lua`, `plugin/lineage.lua`, `lua/lineage/config.lua`):**
    *   Add `"html"` to `ExportCliFormat` (`engine/src/cli.rs`), `ExportFormat` (`schema.rs`), and `EngineCapabilities.export_formats` (`["mermaid", "dot", "html"]`).
    *   Register `<Plug>(LineageExportHtml)` in `plugin/lineage.lua` alongside Stage C.1's `<Plug>` mappings, preserving `:Lineage install!` and `:Lineage! install`.
    *   Add `export.html = { open = true, cache_max_files = 20 }` to `lua/lineage/config.lua` and allow `export.default_format = "html"`.
    *   In `lua/lineage/commands.lua`: dispatch `clean` before format parsing (including `clean` in `:Lineage export <Tab>` completion, never in the `e`/`ge` picker), add `--open` and `--no-open` flags (rejecting `--open` + `--no-open` together, rejecting simultaneous positional `[path]` + `--out`, and warning if `--open`/`--no-open` is passed to `mermaid`/`dot`).
*   **Atomic Content-Addressed Private Cache (`lua/lineage/export_cache.lua`):**
    *   Uses `local uv = vim.uv or vim.loop` (matching existing `lua/lineage/` modules).
    *   **Directory hardening:** Creates `stdpath("cache")/lineage` and `stdpath("cache")/lineage/exports` via `uv.fs_mkdir(dir, tonumber("700", 8))`, verifies via `uv.fs_lstat` that the path is a real directory (refusing symlinks), and enforces `uv.fs_chmod(exports_dir, tonumber("700", 8))` on non-Windows (`uv.os_uname().sysname ~= "Windows_NT"`).
    *   **Content-addressed naming & `0600`-from-birth writes:** Computes `<sha12> = vim.fn.sha256(content):sub(1, 12)` and sanitizes `<slug>` strictly to `[A-Za-z0-9._-]` (max 40 chars) for `lineage-<slug>-<sha12>.html`. On cache hit (verified regular non-symlink file via `uv.fs_lstat`), refreshes `mtime` via `uv.fs_utime` without rewriting. On cache miss, creates a unique sibling temp file via `uv.fs_open(tmp, "wx", tonumber("600", 8))` (atomic `O_CREAT | O_EXCL` with mode `0600` from birth—zero TOCTOU window), writes bytes, closes, and calls `uv.fs_rename(tmp, final_path)`.
    *   **Lazy count-only pruning:** After each cache write/hit, scans `exports_dir` with `uv.fs_scandir` + `uv.fs_lstat`, filters strictly to regular non-symlink files matching `^lineage%-.*%.html$` (plus orphaned `%.tmp$` files older than 1 hour), sorts descending by `(st.mtime.sec, st.mtime.nsec)`, and unlinks entries beyond `config.export.html.cache_max_files` (`20`), never unlinking the just-written file and ignoring `ENOENT` races.
    *   **`:Lineage export clean`:** Unlinks only regular non-symlink `^lineage%-.*%.html$` files under `exports_dir`, handles a missing directory without error, and notifies with exact removed file count and bytes reclaimed.
*   **Headless / SSH / WSL-Safe `vim.ui.open` Guard (`lua/lineage/export_cache.lua`):**
    *   Pre-flight skips `vim.ui.open` when `#vim.api.nvim_list_uis() == 0` (headless CI), or on Linux/BSD (`sysname ~= "Darwin" and sysname ~= "Windows_NT" and vim.fn.has("wsl") == 0`) when both `vim.env.DISPLAY` and `vim.env.WAYLAND_DISPLAY` are nil/empty (preventing `xdg-open` from launching `lynx`/`w3m` over SSH while keeping WSL `wslview`/`explorer.exe` working).
    *   Wraps `vim.ui.open(path)` in `pcall`, checks `(sys_obj, err)`, attaches non-blocking exit-code reporting, and **always** prints the exact written path via `vim.notify` (including `snippets ON` or `snippets OFF`). Exposes injectable `opts.open_fn` and `opts.cache_dir` for deterministic headless testing.
*   **Mixed-Version Capability Check (`lua/lineage/export.lua`):**
    *   Validates `vim.tbl_contains(caps.export_formats, "html")` inside `client.ensure_started(...)`, failing fast with `"lineage-engine does not support format 'html' (supported: mermaid, dot). Run :Lineage install! to update."` against Stage C.1 binaries.

### 4. Observability & Diagnostics

*   **Engine Source-Free Layout Telemetry (`engine/src/export/{mod.rs, html.rs}`):** Every HTML export emits a `tracing::info!` span recording `format = "html"`, `layout_ms`, `serialize_ms`, `node_count`, `edge_count`, `rank_count`, `dummy_nodes_added`, `dummy_cap_hit`, `feedback_back_edges`, `overflow_edges`, `crossing_sweeps`, `svg_width`, `svg_height`, `output_bytes`, `truncated_nodes`, and `truncated_edges` (zero snippets, node labels, symbols, or URIs).
*   **Neovim Status & Health (`lua/lineage/debug.lua`, `lua/lineage/health.lua`):**
    *   `debug.lua:show_status` displays `open_status` (`opened | skipped_headless | skipped_flag | saved_silent | open_failed`) and `cache_pruned` alongside Stage C.1's `lineage/export` telemetry fields.
    *   `health.lua:check` verifies `capabilities.export_formats` (flagging missing `"html"` with `:Lineage install!`), inspects `stdpath("cache")/lineage/exports` directory existence, `0700` permissions (on non-Windows), cached file count, and total bytes, and reports browser-open environment readiness (`DISPLAY`/`WAYLAND_DISPLAY`/WSL/macOS/Windows vs. headless/SSH).

## Alternatives Considered

### Alternative A: Using `scc::tarjan_scc().back_edges` Directly for Stage C.2 Cycle Gutter Routing

*   **Pros**: Reuses `engine/src/slicer/scc.rs::tarjan_scc` without adding a separate 3-color DFS feedback-arc pass in Stage C.1 `ir.rs`.
*   **Cons**: `scc.rs:90-100` pushes **every** edge whose endpoints share a multi-node SCC (`sccs[scc_id].len() > 1 || u == v`) into `back_edges`. Routing all `tarjan_scc().back_edges` into the cycle gutter strips every edge of a 3-node cycle (`A -> B -> C -> A`) out of the DAG ranker, collapsing `A`, `B`, and `C` onto rank 0 with no vertical flow.
*   **Reason for Rejection**: Rejected by author: separating SCC membership (`cycle_group_id`) from deterministic 3-color DFS feedback back-edges (`is_back_edge`) inside Stage C.1 `ir.rs` keeps forward cycle edges top-to-bottom across Mermaid, DOT, and HTML while guaranteeing an acyclic DAG once feedback back-edges are removed.

### Alternative B: Dropping Dummy-Node Waypoints in `layout.rs` (Real-Node Barycenter + Direct Cubic Béziers)

*   **Pros**: Eliminates bounded dummy-node insertion (`<= 600`), dummy-aware barycenter sweeps, and the left-hand long-span overflow gutter, cutting layout code size.
*   **Cons**: On 40–100 node multi-file diamonds, expanded frontiers, and `Both` slices, direct cubic Bézier curves spanning multiple ranks slice straight through intermediate node cards, violating the zero-overlap visual readability goal that motivates Stage C.2 over Mermaid.
*   **Reason for Rejection**: Rejected by author: bounded dummy-node waypoints (`<= 600`) paired with a shared `O(k log k)` interval lane packer (`pack_gutter_lanes`) serving both the right cycle gutter and left overflow gutter via inter-rank horizontal corridors preserve zero card-edge intersections in `< 50ms`.

### Alternative C: Defaulting `format = "html"` to `include_snippets = true` Inside `lineage-engine`

*   **Pros**: Requires zero client-side special-casing in `lua/lineage/export.lua` for local browser opens.
*   **Cons**: Breaks the safe `--no-snippets` mental model established by Mermaid and DOT in Stage C.1, creating a silent source-leak trap whenever a developer runs `:Lineage export html docs/bug.html` or `lineage-engine export ... --format html --out bug.html` expecting snippet-free output.
*   **Reason for Rejection**: Rejected by author: keeping `lineage-engine`'s uniform safe default (`include_snippets: None => false`) across all formats and CLI/explicit-file exports, while having Neovim pass `include_snippets = true` only for ephemeral private-cache HTML opens (unless `--no-snippets` is passed), prevents accidental leaks into shared files.

### Alternative D: Naming the Browser Export Button `"Save Redacted Copy"` or Dropping the Button Entirely

*   **Pros**: `"Save Redacted Copy"` is brief; dropping the button completely removes client-side `Blob` serialization logic and relies solely on CLI/Neovim `--no-snippets`.
*   **Cons**: `"Redacted"` contradicts Stage C.1 Alternative G because workspace-relative file paths (`src/internal_billing/token.ts`), symbol names, and `guard_text` expressions remain intact, misleading developers into assuming full PII/secret scrubbing. Dropping the button forces a developer already inspecting a slice in the browser to return to Neovim and re-export before sharing.
*   **Reason for Rejection**: Rejected by author: renaming the button to **`"Save Without Snippets"`** (stamping `Snippets removed`) states directly what is stripped, while isolating snippets into `#lineage-snippets` with a pre-boot `pristineRoot` clone and matching `safeJsonForScript` escaper keeps implementation to ~15 LOC with zero live-DOM leak or stored `</script>` XSS risk.

### Alternative E: Hash-Based `xxh3_64(norm_path) % N` Per-File Color Palette

*   **Pros**: Reuses `xxhash-rust` (`xxh3`, already in `engine/Cargo.toml:25`) so the same file path tends to keep the same color swatch across unrelated exports.
*   **Cons**: Hashing into a 12-color colorblind-safe palette hits birthday-paradox collisions (~50% collision probability at just 4–5 files), giving distinct files inside the same slice identical accent bars and legend swatches.
*   **Reason for Rejection**: Rejected by author: sequential `palette[file_idx % 12]` over deterministically sorted `norm_path` guarantees zero within-slice collisions for up to 12 files with zero hashing overhead.

### Alternative F: Timestamped Cache Filenames (`lineage-<YYYYMMDD-HHMMSS>-<sha8>.html`) + Dual `20 Files / 7 Days` TTL Eviction

*   **Pros**: Re-exporting an identical slice creates a new file on disk, and stale files expire automatically after 7 days even if fewer than 20 exports accumulate.
*   **Cons**: Repeated identical exports churn the 20-file budget and evict older distinct slices prematurely; dual count + TTL eviction adds unnecessary `mtime` age logic for ~1–2 MB of total cache files.
*   **Reason for Rejection**: Rejected by author: content-addressed filenames (`lineage-<slug>-<sha12>.html`, touching `mtime` on cache hit) deduplicate identical re-exports with zero filename timestamps while distinct edits still get distinct files, and count-only retention (`20` most recent files by `mtime`) plus `:Lineage export clean` keeps `export_cache.lua` minimal.

### Alternative G: Auto-Opening Explicit `:Lineage export html <path>` Files by Default or Adding `"Copy Causal Path (Quickfix)"`

*   **Pros**: Auto-opening explicit paths saves one flag when saving and viewing simultaneously; a quickfix export button turns multi-file chains into a Neovim quickfix list via clipboard.
*   **Cons**: Auto-opening explicit paths breaks delivery symmetry with `:Lineage export mermaid <path>` and `dot <path>` (which write silently); a quickfix causal-chain copy button adds UI clutter and JS code for a secondary workflow already served by one-click `file:line` copy in the side inspector.
*   **Reason for Rejection**: Rejected by author: explicit `:Lineage export html <path>` saves silently (with opt-in `--open`) to match text exporters, and quickfix causal-chain copy is dropped as YAGNI.

### Alternative H: Bundling `dagre`/`elkjs`, Inline JS String Literals, Engine-Managed Cache, Host Timestamps, or Multi-Line Snippets

*   **Pros**: Bundling `dagre`/`elkjs` avoids writing `layout.rs`; inline JS string literals avoid asset files; multi-line snippets show surrounding context inside the browser.
*   **Cons**: Bundling JS layout engines bloats binary/HTML size and violates the pure-Rust `< 50ms` layout goal; inline JS string literals break direct `node --test` validation; engine-managed cache introduces filesystem side effects into CLI/CI runs; host timestamps break 100% byte-identical determinism; multi-line snippets expand `ExportGraph` and HTML payload sizes unnecessarily.
*   **Reason for Rejection**: Rejected by author: pure-Rust `layout.rs`, sibling `engine/src/export/assets/{viewer.css, viewer.js}` via `include_str!`, stateless `lineage-engine`, zero timestamps/workspace names, and Stage C.1's `<= 120`-char single-line snippet satisfy all performance, security, determinism, and testability invariants.

--------------------------------------------------------------------------------

## Non-Functional Requirements

*   **Security & Privacy:**
    *   Treat all repository strings (file paths, symbols, labels, `guard_text`, `risk_messages`, snippets) as untrusted input: apply XML/HTML entity escaping + ASCII control-character stripping on SVG/HTML text, identical Unicode JSON escaping (`<`, `>`, `&`, `U+2028`, `U+2029`) in both Rust and JS, `textContent`/SVG DOM APIs only (never `innerHTML`), and strict `<meta>` CSP (`default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; object-src 'none';`).
    *   Zero wall-clock timestamps and zero host workspace folder names in any HTML artifact; raw snippets exist exclusively inside `#lineage-snippets` (never in SVG markup or attributes) and default to `false` on `lineage-engine`, CLI, and explicit file exports.
    *   Private cache directory enforces `0700` (with symlink rejection via `uv.fs_lstat`, skipping POSIX `chmod` on `Windows_NT`) and atomic `0600`-from-birth file creation (`uv.fs_open(tmp, "wx", tonumber("600", 8))` + `uv.fs_rename`, zero TOCTOU window).
    *   All engine `tracing::info!` layout telemetry and Neovim `:Lineage status` / `:checkhealth lineage` output remain strictly source-free (counts, dimensions, timings, and cache paths/permissions only—never snippets or symbols).
*   **Performance, Payload Bounds & Binary Size:**
    *   Pure-Rust layout (`layout.rs`) completes in `< 50ms` on a 200-node / 800-edge stress graph (`<= 600` dummy waypoints, 6 barycenter sweeps, `O(k log k)` gutter lane packing).
    *   Hard `ExportGraph` caps (`max_nodes = 200`, `max_edges = 800`, `max_label_chars = 256`, `<= 120`-char single-line snippets) plus compact CSS-classed SVG keep worst-case HTML payloads bounded (`<= 600 KB`), avoiding NDJSON stalls over chunk-buffered `lua/lineage/client.lua`.
    *   Zero new Cargo dependencies; Rust 1.82 MSRV preserved (`cargo check --features mimalloc`); stripped release binary growth `< 100 KB` over the Stage C.1 baseline.
*   **Determinism & Offline `file://` Compatibility:**
    *   All layout coordinates are integer `i32` values computed strictly from `BTreeMap` / sorted `Vec` structures; identical `ExportGraph` inputs produce byte-identical HTML across CLI, RPC, Linux, macOS, and Windows.
    *   Zero `fetch()`, zero `localStorage`/cookies, zero cross-file ES imports, zero external fonts/CDNs; works offline directly under `file://` in Firefox and Chromium (including synchronous `file://` copy fallback and delayed `Blob` revocation).
*   **Backward Compatibility & Rollout Safety:**
    *   `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` remain unchanged; Stage C.2 adds `"html"` strictly via `EngineCapabilities.export_formats`.
    *   Mixed-version deployments fail safely: Stage C.2 Lua against a Stage C.1 engine fails fast with `:Lineage install!` guidance when `"html"` is absent from `export_formats`; Stage C.1 Lua against a Stage C.2 engine intersects `export_formats` with Lua-supported formats so raw HTML is never copied to clipboard.
    *   Headless/SSH Linux sessions without `DISPLAY`/`WAYLAND_DISPLAY` never invoke `xdg-open`; explicit `[path]` exports never overwrite existing files without `:Lineage export!`.

--------------------------------------------------------------------------------

### Additional Resources

*   [concepts_brief.md](../../concepts_brief.md) — Stage C.2 7-lens Goldfish brainstorm synthesis and locked decisions.
*   [lineage-stage-c1-graph-exporters.md](./lineage-stage-c1-graph-exporters.md) — Prerequisite Stage C.1 design document (updated with pre-freeze folds `C1-1`–`C1-6`).
*   [lineage-engine-v1.md](./lineage-engine-v1.md) — Frozen v1 CoreIR, `SliceEnvelope`, and NDJSON transport architecture.
*   [graph-export spec](../../openspec/specs/graph-export/spec.md), [observability-and-cli spec](../../openspec/specs/observability-and-cli/spec.md), and [neovim-ui spec](../../openspec/specs/neovim-ui/spec.md) — Stage C.1 and v1 capability specifications extended by Stage C.2.

### Future Improvements

*   **Vertical-Rail In-Terminal Convergence View:** If an in-terminal convergence view is needed later without opening a browser, explore a `git log --graph`-style vertical rail buffer over `ExportGraph`.
*   **Keyboard Stepping Along Causal Order (`n`/`p`):** Built cleanly on top of `#lineage-meta`'s pre-computed `topo_index` if users request sequential step-through inside large 100+ node browser slices.
*   **Post-v1.2 Distribution & Power Workflows:** Windows prebuilt binaries, Mason registry publishing, Multi-Location Queue, and opt-in workspace null-risk diagnostics.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

> **Sequencing Gate:** Stage C.1 ([lineage-stage-c1-graph-exporters.md](./lineage-stage-c1-graph-exporters.md), including pre-freeze folds `C1-1`–`C1-6`) must be implemented, verified, and frozen before any Stage C.2 file below is edited or created.

### Target Files to Create/Modify

#### 1. `engine/src/protocol/schema.rs` (Modify)
*   **Type / Scope**: Rust Wire Protocol Schema (`PROTOCOL_VERSION = 1`, `SCHEMA_VERSION = 1`, strictly additive)
*   **Proposed Changes**:
    *   Add `Html` (`#[serde(rename = "html")]`) variant to `ExportFormat`.
    *   Confirm `ExportParams.include_snippets` remains `Option<bool>` (frozen in Stage C.1) and `ExportResult` stays unchanged so `lineage/export` returns raw UTF-8 HTML in `ExportResult.content`.

#### 2. `engine/src/protocol/transport.rs` (Modify)
*   **Type / Scope**: Rust NDJSON Capability Advertisement
*   **Proposed Changes**:
    *   Update `build_init_result` to advertise `EngineCapabilities.export_formats = vec!["mermaid".into(), "dot".into(), "html".into()]`.
    *   Confirm `"lineage/export"` passes `ExportFormat::Html` through the exact same snapshot-LRU / Phase 2 worker-pool path established in Stage C.1 with zero filesystem side effects.

#### 3. `engine/src/cli.rs` & `engine/src/main.rs` (Modify)
*   **Type / Scope**: Rust Standalone CLI Interface
*   **Proposed Changes**:
    *   In `cli.rs`: add `Html` variant to `ExportCliFormat` (keeping `SliceCliFormat` unchanged so `slice --format html` remains a compile/parse-time error) and update CLI `--help` text documenting that `--format html` writes to `stdout` (or `--out <path>`), defaults to `--no-snippets`, and never auto-opens a browser.
    *   In `main.rs`: map `ExportCliFormat::Html => ExportFormat::Html` in `Commands::Export` and emit the rendered HTML string to `--out <path>` or `stdout`.

#### 4. `engine/src/export/mod.rs` (Modify)
*   **Type / Scope**: Rust Export Subsystem Entrypoint & Telemetry
*   **Proposed Changes**:
    *   Declare `pub mod layout;` and `pub mod html;`.
    *   Keep `ExportOptions::for_format` resolving tri-state `Option<bool>` snippets to `false` uniformly across `Mermaid`, `Dot`, and `Html` when omitted (`None`).
    *   Route `ExportFormat::Html` to `layout::compute_layout(&export_graph, opts)` followed by `html::render_html(&export_graph, &svg_layout, opts)`, timing both phases independently.
    *   Emit source-free `tracing::info!` structured telemetry fields for HTML exports: `format = "html"`, `layout_ms`, `serialize_ms`, `node_count`, `edge_count`, `rank_count`, `dummy_nodes_added`, `dummy_cap_hit`, `feedback_back_edges`, `overflow_edges`, `crossing_sweeps`, `svg_width`, `svg_height`, `output_bytes`, `truncated_nodes`, and `truncated_edges` (strictly zero URIs, symbols, labels, or snippets).

#### 5. `engine/src/export/layout.rs` (Create)
*   **Type / Scope**: Rust Pure-Integer Layered-DAG SVG Coordinate Layout Engine (Zero HTML/SVG String Formatting)
*   **Proposed Changes**:
    *   Define pure geometry output types: `SvgLayout { view_width: i32, view_height: i32, nodes: Vec<LayoutNode>, edges: Vec<LayoutEdge>, focal_band_y: Option<i32>, empty_state: bool, trivial_single: bool, metrics: LayoutMetrics }`, `LayoutNode { id, x, y, w, h, rank, topo_index }`, `LayoutEdge { id, from, to, route_kind: Forward | CycleGutter | OverflowLeft | SelfLoop, waypoints: Vec<(i32, i32)>, is_back_edge, cycle_group_id }`.
    *   Pin fixed integer constants: `CARD_W = 240`, `CARD_H = 72`, `H_GAP = 40`, `V_GAP = 56`, `DUMMY_W = 24`, `GUTTER_GAP = 28`, `LANE_PITCH = 14`, `CORRIDOR_STEP = 10`, `COMPONENT_GAP_Y = 72`, `ARROW_PULLBACK = 8`, `MARGIN = 32`, `MAX_DUMMIES = 600`, `CROSSING_SWEEPS = 6`.
    *   Implement the deterministic 7-stage pipeline over `&ExportGraph` using `BTreeMap`/sorted `Vec` exclusively:
        1. Handle 0-node (`640 x 240`, `empty_state = true`) and 1-node (centered, `trivial_single = true`) graphs; partition WCCs ordered primary-root-first then lowest `N*` ID, laying out each component independently and stacking vertically with `COMPONENT_GAP_Y`.
        2. Partition forward edges (`!is_back_edge && u != v`) vs. feedback back-edges (`is_back_edge && u != v`) vs. self-loops (`u == v`).
        3. Assign ranks: standard longest-path for `To`/`From`; for `Both`, pin primary `root_ids` at `rank = 0`, assign upstream via reverse longest-path and downstream via forward longest-path, shift by `-min_rank`, falling back cleanly to standard longest-path if `root_ids` is empty/pruned or contains an inter-root DAG edge.
        4. Insert dummy waypoints (`span - 1`) for multi-rank forward edges sorted by `(span, from_id, to_id)` up to `MAX_DUMMIES = 600`; classify any overflow long-span edges as `OverflowLeft`.
        5. Run 6 alternating top-down / bottom-up barycenter sweeps (tie-broken by canonical ID) and 2-pass centered median-`x` coordinate placement with strict left-to-right `x[i] >= x[i-1] + width[i-1] + H_GAP` clamping.
        6. Implement shared `O(k log k)` `pack_gutter_lanes` assigning non-overlapping vertical lanes for both right-hand cycle back-edges (`max_card_right + GUTTER_GAP + lane * LANE_PITCH`) and left-hand overflow long-span edges (`min_card_left - GUTTER_GAP - lane * LANE_PITCH`, shifting all coordinates so `min_x >= MARGIN`), routing every gutter entry/exit strictly through inter-rank horizontal corridors (expanding inter-rank gap if corridor tracks require extra vertical space) and giving self-loops explicit non-overlapping clearance.
        7. Distribute top/bottom card ports sorted by neighbor `x`, apply `ARROW_PULLBACK`, and compute tight `view_width`/`view_height` and `LayoutMetrics`.

#### 6. `engine/src/export/html.rs` (Create)
*   **Type / Scope**: Rust Single-File Offline HTML+SVG Serializer & Escaper
*   **Proposed Changes**:
    *   Embed `assets/viewer.css` and `assets/viewer.js` via `include_str!("assets/viewer.css")` and `include_str!("assets/viewer.js")`.
    *   Implement `xml_escape_and_strip_ctrl(&str) -> String` (stripping ASCII `0x00..=0x1F` except `\t`/`\n`, escaping `&`, `<`, `>`, `"`, `'`), `safe_json_for_script(&str) -> String` (escaping `<`, `>`, `&`, `\u{2028}`, `\u{2029}` to `\uXXXX`), and UTF-8 char-safe `middle_elide(&str, max_chars: usize) -> String` (`src/…/session.ts:42`).
    *   Render `<!DOCTYPE html>`, strict `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; object-src 'none';">`, descriptive `<title>`, header trust bar (direction, counts, completeness/truncation badges, snippet status pill `Snippets: ON (local)` / `Snippets: OFF`, and toolbar controls), scrollable per-file legend (`color_idx = file_idx % 12`), inline SVG (`<defs>` shared arrowheads + `<clipPath id="clip-N{k}">`, `<g id="edges-layer">`, `<g id="nodes-layer">`, empty `<g id="active-overlay">`, optional `Both` waist band markers, unclipped Coverage-Honesty header badges + clipped middle-elided labels, zero snippets in SVG or `data-*`), and collapsed-by-default side inspector shell.
    *   Emit `<script type="application/json" id="lineage-meta">` (`viewer_schema_version = 1`, zero timestamps/host names/`CARGO_PKG_VERSION`) and `<script type="application/json" id="lineage-snippets">` (`{}` when `!opts.include_snippets`, otherwise `{ "<node_id>": "<pre-scrubbed <=120-char snippet>" }`).

#### 7. `engine/src/export/assets/viewer.css` (Create)
*   **Type / Scope**: Embedded Offline Viewer Stylesheet (`include_str!`)
*   **Proposed Changes**:
    *   Define CSS custom properties for light theme and `@media (prefers-color-scheme: dark)` plus explicit `[data-theme="dark"]` / `[data-theme="light"]` overrides.
    *   Define 12 colorblind-safe categorical file-accent variables (`--file-color-0..11`) meeting WCAG AA contrast (`>= 4.5:1` text, `>= 3:1` strokes) in both themes, confined strictly to the 4px card accent bar and legend swatch.
    *   Style Coverage-Honesty borders/strokes (dashed `Unverified`, heavy/double `Local Risk`, outer `FOCUS` halo for `is_root`, dashed upward-arrow cycle back-edges), focus-dimming opacity classes (`.dimmed`, `.active`), scrollable file legend, collapsed/expanded side inspector, inline copy toast, and disabled zero-count filter pills.

#### 8. `engine/src/export/assets/viewer.js` (Create)
*   **Type / Scope**: Embedded Modular Offline Vanilla JS Viewer (`~250–350 LOC`, `include_str!`)
*   **Proposed Changes**:
    *   Capture `const pristineRoot = document.documentElement.cloneNode(true);` on line 1 of browser boot before any DOM mutation.
    *   Separate pure, DOM-free core functions (exported via `if (typeof module !== "undefined" && module.exports)` for `node --test`):
        *   `safeJsonForScript(value)`: matches Rust's `<`, `>`, `&`, `\u2028`, `\u2029` Unicode escaping byte-for-byte.
        *   `computeCausalCone(meta, nodeId)`: cycle-safe upstream + downstream BFS returning active node and edge ID sets.
        *   `computeNullPaths(meta)`: bidirectional BFS from `Absent`/`Nullable` nodes returning full causal paths (matching Stage C.1 `ir.rs`).
        *   `evaluateFilters(meta, filterState)`: OR union across active `Null-only`/`Guards`/`Unverified`, intersected with active file legend selection, honoring pre-filtered `export_filters`.
        *   `buildSnippetFreeHtml(pristineRoot)`: clones `pristineRoot`, sets `#lineage-snippets` to `"{}"`, marks `meta.snippets_removed = true` via `safeJsonForScript`, updates header pill to `Snippets removed`, removes the save button on the clone, and returns `"<!DOCTYPE html>\n" + clone.outerHTML`.
    *   Implement thin DOM wiring using `textContent` and `createElement`/`createElementNS` exclusively (never `innerHTML`):
        *   Pan/zoom (wheel + drag), `+`/`-`/`Fit View`/`Fit Active` (`f`)/`Center Focus`, and initial load framing (`<= 25` nodes fits view; `> 25` nodes centers primary focal node at `1.0` scale; guards `bbox.width === 0`).
        *   Semantic filter buttons with live counts (`Unverified (0)` disabled), locked pills for `export_filters`, clickable file legend, node click highlighting causal cone + cloning active edge paths into `<g id="active-overlay">` + opening side inspector (`Esc`/canvas click clears selection; `Reset`/`r` clears all).
        *   Inverted synchronous `file://` 3-step `Copy path:line` (`window.isSecureContext && navigator.clipboard?.writeText` -> synchronous fixed `<textarea readonly>` `focus({ preventScroll: true })` + `execCommand("copy")` -> inline pre-selected `<input readonly>` toast).
        *   `"Save Without Snippets"` click handler emitting `Blob` download (`*-no-snippets.html`) with `setTimeout(() => URL.revokeObjectURL(url), 1000)` (hidden/disabled when `#lineage-snippets` is already `{}`), plus in-memory Dark/Light toolbar toggle.

#### 9. `lua/lineage/config.lua` (Modify)
*   **Type / Scope**: Neovim Plugin Configuration Defaults
*   **Proposed Changes**:
    *   Extend `M.defaults.export` with `html = { open = true, cache_max_files = 20 }` and allow `export.default_format = "html"`.

#### 10. `lua/lineage/export_cache.lua` (Create)
*   **Type / Scope**: Neovim Private HTML Cache Lifecycle & Safe Browser Opener
*   **Proposed Changes**:
    *   Use `local uv = vim.uv or vim.loop` and support injectable `opts.cache_dir` and `opts.open_fn` for headless testing.
    *   Implement `M.ensure_dir(dir)`: create parent + `stdpath("cache")/lineage/exports` via `uv.fs_mkdir(..., tonumber("700", 8))`, reject symlinks via `uv.fs_lstat`, and apply `uv.fs_chmod(..., tonumber("700", 8))` when `uv.os_uname().sysname ~= "Windows_NT"`.
    *   Implement `M.write_cached(content, slug, opts)`: compute `sha12 = vim.fn.sha256(content):sub(1, 12)`, sanitize `slug` to `[A-Za-z0-9._-]` (max 40 chars) for `lineage-<slug>-<sha12>.html`; on valid regular-file hit, refresh `mtime` via `uv.fs_utime`; on miss, open sibling temp file via `uv.fs_open(tmp, "wx", tonumber("600", 8))`, write, close, and `uv.fs_rename` atomically; then lazily prune regular non-symlink `^lineage%-.*%.html$` entries past `cache_max_files` (`20`) sorted descending by `(st.mtime.sec, st.mtime.nsec)` (never deleting the just-written file, cleaning `%.tmp$` > 1h, ignoring `ENOENT`), returning `{ path, pruned, reused }`.
    *   Implement `M.clean(opts)`: unlink only regular non-symlink `^lineage%-.*%.html$` files in the cache directory (graceful no-op if directory missing), returning `{ removed_files, freed_bytes }`.
    *   Implement `M.can_open_browser()`: returns `false` when `#vim.api.nvim_list_uis() == 0`, or on Linux/BSD (`sysname ~= "Darwin" and sysname ~= "Windows_NT" and vim.fn.has("wsl") == 0`) when both `vim.env.DISPLAY` and `vim.env.WAYLAND_DISPLAY` are nil/empty.
    *   Implement `M.open_file(path, opts)`: checks `can_open_browser()`, calls `pcall(open_fn or vim.ui.open, path)`, inspects `(sys_obj, err)` and async non-zero exit code without blocking, returning `{ status = "opened" | "skipped_headless" | "open_failed", err }`.
    *   Implement `M.stats(opts)` for `:checkhealth lineage` (dir exists, non-symlink, mode bits on Unix, file count, total bytes, `can_open_browser()`).

#### 11. `lua/lineage/export.lua` (Modify)
*   **Type / Scope**: Neovim Export Orchestration & HTML Delivery
*   **Proposed Changes**:
    *   Add `"html"` to Lua-supported delivery formats (`HTML (browser)` in the `e`/`ge` picker whenever `"html"` is advertised in `EngineCapabilities.export_formats`).
    *   Verify exact format membership (`vim.tbl_contains(caps.export_formats, format)`) inside `client.ensure_started(...)`, failing fast with `"lineage-engine does not support format 'html' (supported: ...). Run :Lineage install! to update."` against Stage C.1 binaries.
    *   Apply destination-aware snippet default: for `format == "html"` with no explicit `[path]`/`--out` and no explicit `--snippets`/`--no-snippets`, pass `include_snippets = true`; for explicit `[path]`/`--out`, pass `include_snippets = nil` (engine resolves to `false`) unless `--snippets` was given.
    *   Route bare `html` delivery through `export_cache.write_cached` + conditional `export_cache.open_file` (respecting `--no-open` and `config.export.html.open`), and route explicit-path `html` through Stage C.1's atomic file writer (refusing overwrite without `!`, saving silently unless `--open` is passed). Always include the exact written path and snippet state (`snippets ON` / `snippets OFF`) in `vim.notify`, and record `open_status` / `cache_pruned` in telemetry.
    *   Expose `M.clean()` calling `export_cache.clean()` and notifying with exact removed file count and bytes reclaimed (or `"No cached HTML exports found"`).

#### 12. `lua/lineage/commands.lua` & `plugin/lineage.lua` (Modify)
*   **Type / Scope**: Neovim Command Parser, Tab Completion & `<Plug>` Registration
*   **Proposed Changes**:
    *   In `commands.lua`: intercept `:Lineage export clean` before `<format>` parsing (advertising `clean` in `:Lineage export <Tab>` completion, rejecting export flags when `clean` is used); parse `--open` and `--no-open` (rejecting `--open` + `--no-open` together, rejecting simultaneous positional `[path]` + `--out`, and emitting a warning if `--open`/`--no-open` is passed to `mermaid`/`dot`).
    *   In `plugin/lineage.lua`: register `<Plug>(LineageExportHtml)` alongside `<Plug>(LineageExport{,Mermaid,Dot})` while keeping `:Lineage install!` and `:Lineage! install` intact.

#### 13. `lua/lineage/debug.lua` & `lua/lineage/health.lua` (Modify)
*   **Type / Scope**: Neovim Status & Health Diagnostics
*   **Proposed Changes**:
    *   In `debug.lua:show_status`: append `open=<open_status>` and `pruned=<cache_pruned>` when formatting `lineage/export` entries from `telemetry_ring`.
    *   In `health.lua:check`: report `capabilities.export_formats` (flagging missing `"html"` with `:Lineage install!` advice), report `export_cache.stats()` (directory presence, `0700` mode verification on non-Windows, cached file count / max 20, total bytes), and report browser-open environment readiness (`DISPLAY`/`WAYLAND_DISPLAY`/WSL/macOS/Windows vs. headless/SSH).

#### 14. `engine/tests/export_html.rs` (Create)
*   **Type / Scope**: Rust Geometry Verifier, Security & Structure Integration Tests
*   **Proposed Changes**:
    *   **Geometry Verifier**: Assert zero node-node AABB overlaps, zero node-gutter overlaps, zero gutter-corridor segment-vs-card-AABB intersections, arrowheads strictly outside card bounds (`ARROW_PULLBACK`), monotonic `rank[u] < rank[v]` for all forward edges, `Both` focal waist pinning (including pruned-root and multi-root fallbacks), multi-node cycle forward edges top-to-bottom + DFS back-edges in right gutter, explicit self-loop clearance, 600-dummy-cap overflow into left gutter (adversarial 200-node long-span graph), multi-component WCC stacking, 0/1-node non-zero `viewBox`, byte-identical determinism, and `< 50ms` layout time on a 200-node / 800-edge stress graph.
    *   **Security & Structure Suite**: Assert `<!DOCTYPE html>`, strict `<meta>` CSP (no `frame-ancestors`), zero raw snippets in SVG `<text>`/`<title>`/`data-*`, zero timestamps/host workspace names/`CARGO_PKG_VERSION`, adversarial string safety (`</script><img onerror=alert(1)>`, `"`, `'`, `&`, `\x00`..`\x1f`, `\u{2028}`, `\u{2029}`), sequential `color_idx = file_idx % 12`, `--no-snippets` vs. `--snippets` JSON island parity, and CLI vs. `lineage/export` RPC byte-identity.

#### 15. `engine/tests/viewer_js.test.mjs` (Create)
*   **Type / Scope**: Optional Pure-JS Unit Tests (`node --test`, Zero npm Dependencies)
*   **Proposed Changes**:
    *   Import pure functions directly from `engine/src/export/assets/viewer.js` and test `safeJsonForScript` (exact parity with Rust on `</script>`, `<!--`, `U+2028`, `U+2029`), `computeCausalCone` (cycle-safe ancestors + descendants), `computeNullPaths` (causal path parity with C.1 `ir.rs`), `evaluateFilters` (OR union + file legend intersection + zero-count disable + locked pre-filters), and `buildSnippetFreeHtml` (idempotent double-save, `<!DOCTYPE html>`, zero live DOM leak, `</script>` XSS immunity).

#### 16. `tests/test_export_html.lua` (Create)
*   **Type / Scope**: Headless Neovim E2E Integration Test Suite
*   **Proposed Changes**:
    *   Test bare `:Lineage export html` private cache delivery (`0700`/`0600` permissions on non-Windows, symlink refusal, content-addressed dedup + `mtime` refresh, count-only 20-file lazy prune, `include_snippets = true` default vs. `--no-snippets`).
    *   Test explicit path `:Lineage export html <path>` (`include_snippets = false` default, silent save, `--open` opt-in, `--snippets` opt-in, `!` overwrite protection, rejecting `[path]` + `--out`).
    *   Test `:Lineage export clean` (exact file count + bytes reclaimed, ignores non-matching files/symlinks, safe on missing dir), headless/SSH `vim.ui.open` skip + WSL allowance via injectable `open_fn`, mixed-version C.1 engine capability rejection (`"html"` absent), `<Plug>(LineageExportHtml)`, and `:Lineage status` / `:checkhealth lineage`.

#### 17. OpenSpec & Documentation Updates (Modify)
*   **Files**: `openspec/specs/graph-export/spec.md`, `openspec/specs/observability-and-cli/spec.md`, `openspec/specs/neovim-ui/spec.md`, `README.md`, `doc/lineage.txt`
*   **Type / Scope**: Capability Specifications & User Documentation (leaving `openspec/specs/core-ir-and-slicer/spec.md` untouched)
*   **Proposed Changes**:
    *   Extend `graph-export/spec.md` with Stage C.2 layout invariants, strict-CSP HTML/SVG schema, destination-aware snippet policy, `"Save Without Snippets"`, content-addressed `0700`/`0600` cache, and headless/SSH `vim.ui.open` rules.
    *   Extend `observability-and-cli/spec.md` with `ExportCliFormat::Html` (`stdout` / `--out`, never auto-open), source-free HTML layout tracing fields, and `:checkhealth lineage` cache/opener checks.
    *   Extend `neovim-ui/spec.md`, `README.md`, and `doc/lineage.txt` with `:Lineage export html`, `:Lineage export clean`, `--open` / `--no-open`, `<Plug>(LineageExportHtml)`, and `export.html` config defaults.

---

### Verification Plan

*   **Compilation, MSRV & Binary Size Delta (Zero New Crates):**
    *   `cd engine && cargo build --release && cargo check --features mimalloc`
    *   Verify stripped `target/release/lineage-engine` size delta stays `< 100 KB` over the frozen Stage C.1 baseline.
*   **Rust Geometry, Security, Parity & Regression Tests:**
    *   `cd engine && cargo test` (must pass `export_html`, `export_golden`, and all existing engine suites, including `< 50ms` layout on the 200-node / 800-edge stress graph).
*   **Optional Pure-JS Unit Tests (When Node Is Present, Non-Gating):**
    *   `command -v node >/dev/null && node --test engine/tests/viewer_js.test.mjs`
*   **Static Analysis & Formatting:**
    *   `cd engine && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
    *   `stylua --check plugin lua tests`
*   **Headless Neovim E2E Integration Tests (Neovim 0.10 & 0.11):**
    *   `for t in tests/test_*.lua; do nvim --headless -u tests/minimal_init.lua -l "$t"; done` (covering `tests/test_export_html.lua` and confirming zero regressions across `test_export.lua`, `test_hover.lua`, `test_tree.lua`, `test_two_phase_lsp.lua`, and `test_install.lua`).
*   **Manual Visual Sign-Off Gate (Before Closing Stage C.2):**
    *   Generate 4 deterministic golden HTML artifacts—(1) linear chain, (2) multi-file diamond convergence, (3) recursive cycle, and (4) 50-node multi-file `Both` slice—and open each directly via `file://` in both **Firefox** and **Chromium**, verifying:
        *   Zero browser console / CSP errors.
        *   Zero overlapping cards, edges, or arrowheads; clear `Both` focal waist and upward-arrow cycle gutter routing.
        *   Pan/zoom, `Fit View` / `Fit Active` / `Center Focus`, click-node causal cone focus-dimming, OR-composed `Null-only`/`Guards`/`Unverified` filters with live counts, clickable sequential-color file legend, collapsed-to-expanded side inspector, dark/light toggle, and 3-step `file://` `Copy path:line`.
        *   `"Save Without Snippets"` downloads cleanly under `file://` in both browsers and reopens with zero snippets, `Snippets removed` badge, clean initial viewport state, and zero console errors.

--------------------------------------------------------------------------------

*Drafted with EGM*
