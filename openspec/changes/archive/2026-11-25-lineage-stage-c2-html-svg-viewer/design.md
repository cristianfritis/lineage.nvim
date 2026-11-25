## Context

`lineage.nvim` Stage C.1 ([docs/designs/lineage-stage-c1-graph-exporters.md](../../../docs/designs/lineage-stage-c1-graph-exporters.md), including pre-freeze folds `C1-1`–`C1-6`) establishes a canonical pure-Rust `ExportGraph` IR (`engine/src/export/ir.rs`), a 4-slot `QuerySnapshotCache` LRU, an additive `lineage/export` NDJSON RPC advertised via `EngineCapabilities.export_formats`, and deterministic offline Mermaid and Graphviz DOT exporters. Stage C.1 must be implemented, verified, and frozen before Stage C.2 coding starts.

Stage C.2 ([docs/designs/lineage-stage-c2-html-svg-viewer.md](../../../docs/designs/lineage-stage-c2-html-svg-viewer.md)) plugs an offline, deterministic interactive HTML+SVG viewer (`format = "html"`) directly onto Stage C.1's `ExportGraph` and `lineage/export` contracts, targeting 15–120 node slices (hard cap 200 nodes / 800 edges) where Mermaid becomes unreadable, Graphviz DOT is static, and the Neovim Tree Panel duplicates converging subtrees.

Constraints:
- Zero new Cargo dependencies; preserve Rust 1.82 MSRV and `< 100 KB` release binary delta over Stage C.1.
- Preserve `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` (additive capability advertisement only).
- 100% offline `file://` execution: zero `fetch()`, zero `localStorage`/cookies, zero external fonts/CDNs, zero browser layout engines (`dagre`/`elkjs`), and zero Playwright/Chromium/Graphviz in CI.
- Strict byte-identical determinism across CLI, RPC, Linux, macOS, and Windows: integer `i32` SVG coordinates, `BTreeMap`/sorted `Vec` traversal only, zero wall-clock timestamps, zero host workspace folder names, and zero `CARGO_PKG_VERSION` in HTML bodies.

## Goals / Non-Goals

**Goals:**
- Consume `&ExportGraph` and `&ExportOptions` exclusively in `engine/src/export/{layout.rs, html.rs}` with zero bypass to raw `&SliceEnvelope`.
- Lay out up to 200 nodes and 800 edges in `< 50ms` with integer `i32` coordinates, zero AABB overlaps (node-node, node-gutter, corridor-card, arrowhead-card), `Both` focal waist pinning, bounded dummy waypoints (`<= 600`), and a shared `O(k log k)` interval lane packer for right cycle gutter + left overflow long-span gutter routed strictly via inter-rank horizontal corridors.
- Emit a strict-CSP single-file HTML document using real sibling assets (`engine/src/export/assets/{viewer.css, viewer.js}` embedded via `include_str!`), dual Unicode-escaped JSON islands (`#lineage-meta` + `#lineage-snippets`), zero snippets in SVG/attributes, sequential `palette[file_idx % 12]` file accents, OR-composed semantic filters + click-node causal cone, inverted synchronous `file://` 3-step `Copy path:line`, OS dark/light + toolbar toggle, and `"Save Without Snippets"` via pre-boot `pristineRoot` clone.
- Keep `lineage-engine` stateless (`ExportResult.content` on RPC; `stdout` / `--out` on CLI) with uniform `include_snippets: None => false` across all formats, passing `include_snippets = true` only from Neovim on bare ephemeral private-cache HTML opens.
- Manage an atomic `0700`/`0600`-from-birth content-addressed cache (`lineage-<slug>-<sha12>.html`, count-only 20 most recent files by `mtime`, `:Lineage export clean`) and headless/SSH/WSL-safe `vim.ui.open` in `lua/lineage/export_cache.lua`, with source-free layout telemetry and `:checkhealth lineage` diagnostics.

**Non-Goals:**
- Coding Stage C.2 before Stage C.1 (with `C1-1`–`C1-6`) passes all verification gates.
- Enclosing SVG per-file cluster rectangles, reverse browser-to-Neovim RPC sockets, `localStorage`/cookies, multi-line source snippets, quickfix causal-chain copy, or 7-day cache TTL expiry.

## Decisions

1. **7-Stage Pure-Rust Integer Layout (`engine/src/export/layout.rs`) with Bounded Dummy Waypoints (`<= 600`) + Shared Interval-Packed Gutters:**
   - *Choice:* Fixed card bounds (`CARD_W = 240`, `CARD_H = 72`, `H_GAP = 40`, `V_GAP = 56`, `DUMMY_W = 24`, `GUTTER_GAP = 28`, `LANE_PITCH = 14`, `CORRIDOR_STEP = 10`, `COMPONENT_GAP_Y = 72`, `ARROW_PULLBACK = 8`, `MARGIN = 32`), 0/1-node handling + WCC vertical stacking, `Both` waist pinning (fallback to standard longest-path if `root_ids` is empty/pruned or inter-root edges exist), shortest-span-first dummy waypoints up to `600`, 6 barycenter sweeps + clamped median-`x` placement, and one shared `O(k log k)` `pack_gutter_lanes` routine serving both the right-hand cycle gutter (DFS feedback back-edges only) and a left-hand overflow gutter (when `600` dummies saturate), routed strictly through inter-rank horizontal corridors.
   - *Alternatives rejected:* Dropping dummy waypoints (direct cubic Béziers cut through intermediate cards on 40–100 node graphs); using `scc::tarjan_scc().back_edges` directly (marks every edge in a multi-node SCC, collapsing cycles onto rank 0); bundling `dagre`/`elkjs` (bloats payload/binary and breaks pure-Rust `< 50ms` determinism).

2. **Real Sibling Assets (`engine/src/export/assets/{viewer.css, viewer.js}`) Embedded via `include_str!`:**
   - *Choice:* Keep CSS and ~250–350 LOC vanilla JS in real files embedded at compile time with zero runtime I/O, exporting pure core functions (`safeJsonForScript`, `computeCausalCone`, `computeNullPaths`, `evaluateFilters`, `buildSnippetFreeHtml`) behind a CommonJS `module.exports` guard for direct `node --test` execution.
   - *Alternative rejected:* Inline Rust raw string literals in `html.rs` (untestable without HTML scraping, prone to test/runtime drift).

3. **Destination-Aware Snippet Policy + Dual JSON Islands + Pre-Boot `pristineRoot` Clone for `"Save Without Snippets"`:**
   - *Choice:* `lineage-engine` resolves omitted `include_snippets: None` to `false` uniformly across all formats (CLI `stdout`/`--out`, Mermaid, DOT, and explicit-path Neovim exports). `lua/lineage/export.lua` passes `include_snippets = true` only for bare private-cache HTML opens (`:Lineage export html` without `[path]`/`--out` and without `--no-snippets`). In HTML, all snippets live exclusively inside `<script type="application/json" id="lineage-snippets">` (zero snippets in SVG `<text>`, `<title>`, or `data-*`). On JS boot line 1, `const pristineRoot = document.documentElement.cloneNode(true)` is captured; `"Save Without Snippets"` clones `pristineRoot`, clears `#lineage-snippets` to `"{}"`, stamps `Snippets removed`, re-serializes `#lineage-meta` with `safeJsonForScript` (`<`, `>`, `&`, `\u2028`, `\u2029`), prepends `<!DOCTYPE html>\n`, and revokes the `Blob` URL after `1000ms`.
   - *Alternatives rejected:* Defaulting HTML to `include_snippets = true` inside `lineage-engine` (leaks snippets on explicit file exports); naming the button `"Save Redacted Copy"` (misleads users into assuming paths/symbols/guards are scrubbed, contradicting Stage C.1 Alternative G); serializing live `document.documentElement.outerHTML` with plain `JSON.stringify` (leaks open inspector DOM and re-introduces stored `</script>` XSS).

4. **Sequential `palette[file_idx % 12]` File Colors & Coverage-Honesty Channel Separation:**
   - *Choice:* Assign curated colorblind-safe WCAG AA CSS variables (`--file-color-0..11`) sequentially by sorted `norm_path` index (`file_idx % 12`), confined strictly to a 4px left card accent bar and legend swatch. Keep Coverage-Honesty status glyphs (`●`, `◐`, `○`, `?`, `!`, guard icon) in an unclipped card header zone, clipping only middle-elided variable text with `<clipPath id="clip-N{k}">`.
   - *Alternative rejected:* `xxh3_64(norm_path) % 12` (birthday-paradox collisions in 4–8 file slices).

5. **Inverted Synchronous `file://` 3-Step Click-to-Copy:**
   - *Choice:* Check `window.isSecureContext && navigator.clipboard?.writeText` synchronously; on `file://`, invoke fixed-position `<textarea readonly>` (`focus({ preventScroll: true })`) + `document.execCommand("copy")` synchronously while browser user activation is live, cushioned by an inline pre-selected `<input readonly>` toast.
   - *Alternative rejected:* Calling `execCommand("copy")` inside `navigator.clipboard.writeText().catch(...)` (fails in Firefox/Safari because async rejection drops transient user activation); quickfix causal-chain copy (dropped as YAGNI).

6. **Stateless Engine + Atomic `0700`/`0600` Content-Addressed Neovim Cache (`lua/lineage/export_cache.lua`):**
   - *Choice:* `lineage/export` returns raw HTML in `ExportResult.content`; CLI writes `stdout`/`--out` only. Neovim isolates cache management in `lua/lineage/export_cache.lua` using `uv.fs_mkdir(..., 0700)` + `uv.fs_lstat` symlink refusal + non-Windows `uv.fs_chmod`, `lineage-<slug>-<sha12>.html` content-addressed names (`vim.fn.sha256` + `uv.fs_utime` on hit), atomic `uv.fs_open(tmp, "wx", 0600)` + `uv.fs_rename`, count-only 20-file lazy prune, `:Lineage export clean`, silent explicit-path saves (opt-in `--open`), and headless/SSH/WSL-safe `vim.ui.open`.
   - *Alternatives rejected:* Timestamped filenames + 7-day TTL eviction (churns repeated exports and adds unnecessary age logic); engine-managed cache (adds filesystem side effects to CLI/CI); auto-opening explicit `[path]` (breaks symmetry with Mermaid/DOT).

## Risks / Trade-offs

- **Gutter corridor density on extreme cyclic graphs** → Automatically expand inter-rank vertical spacing when a corridor requires multiple horizontal tracks; assert zero segment-vs-card-AABB intersections in `engine/tests/export_html.rs`.
- **`blob:` download behavior under strict `<meta>` CSP on `file://` across browsers** → Delay `URL.revokeObjectURL(url)` by `1000ms`, omit invalid `frame-ancestors` from `<meta>` CSP, and verify `"Save Without Snippets"` with zero console errors in both Firefox and Chromium during manual sign-off.
- **Mixed-version C.1 <-> C.2 deployments** → Stage C.2 Lua checks exact `"html"` membership in `EngineCapabilities.export_formats` (failing fast with `:Lineage install!`), while Stage C.1 Lua intersects `export_formats` with Lua-supported formats so older Lua never copies raw HTML to clipboard.
- **Headless / SSH Linux with `xdg-open` installed** → Pre-flight skip `vim.ui.open` when `#vim.api.nvim_list_uis() == 0` or on non-WSL/non-macOS/non-Windows Linux with empty `DISPLAY` and `WAYLAND_DISPLAY`, always notifying with the exact written file path.

## Migration Plan

1. Implement, verify, and freeze Stage C.1 ([docs/designs/lineage-stage-c1-graph-exporters.md](../../../docs/designs/lineage-stage-c1-graph-exporters.md) with folds `C1-1`–`C1-6`) first.
2. Implement Stage C.2 engine changes (`schema.rs`, `transport.rs`, `cli.rs`, `main.rs`, `export/{mod.rs, layout.rs, html.rs}`, `export/assets/{viewer.css, viewer.js}`) and pass `cargo test` + optional `node --test` + `< 100 KB` binary delta check.
3. Implement Stage C.2 Neovim changes (`config.lua`, `export_cache.lua`, `export.lua`, `commands.lua`, `plugin/lineage.lua`, `debug.lua`, `health.lua`) and pass headless Neovim 0.10/0.11 E2E tests plus `stylua --check`.
4. Complete the 4-artifact Firefox & Chromium `file://` manual visual sign-off gate before publishing a new `engine-v*` release and plugin tag. Rollback is safe because `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` remain backward-compatible and Stage C.1 Mermaid/DOT behavior is unaffected.

## Open Questions

None — all Stage C.2 design decisions and Stage C.1 pre-freeze folds (`C1-1`–`C1-6`) are locked.
