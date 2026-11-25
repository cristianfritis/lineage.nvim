## Purpose
Provide a 100% offline, deterministic, strict-CSP single-file HTML+SVG interactive graph viewer (`format = "html"`) over Stage C.1's `ExportGraph` IR, with pure-Rust layered-DAG SVG layout, destination-aware snippet isolation, `"Save Without Snippets"`, content-addressed `0700`/`0600` Neovim cache management, and headless/SSH/WSL-safe browser opening.

## Requirements

### Requirement: Pure-Rust Deterministic Layered-DAG SVG Layout
`engine/src/export/layout.rs` SHALL consume Stage C.1's canonical `&ExportGraph` and `&ExportOptions` exclusively (never bypassing to `&SliceEnvelope`) using zero new Cargo crates and integer `i32` coordinates exclusively, SHALL lay out graphs up to 200 nodes and 800 edges in `< 50ms` with byte-identical determinism across platforms, SHALL enforce zero bounding-box overlaps (node-node, node-gutter, gutter-corridor segment vs. card AABB, and arrowhead-card via `ARROW_PULLBACK`), SHALL preserve strict top-to-bottom rank ordering (`rank[u] < rank[v]`) for all forward edges (including forward edges of multi-node cycles), SHALL pin primary `root_ids` at a shared focal waist on `Both` slices (falling back deterministically to standard longest-path if `root_ids` is empty/pruned or contains inter-root DAG edges), SHALL insert shortest-span-first dummy waypoints up to a hard cap of `600`, SHALL pack both right-hand DFS feedback back-edges (`is_back_edge && u != v`) and left-hand overflow long-span forward edges (when `600` dummies saturate) via a shared `O(k log k)` interval lane packer routed strictly through inter-rank horizontal corridors, SHALL route self-loops (`u == v`) with explicit non-overlapping clearance, and SHALL handle weakly connected components (WCC vertical stacking) as well as 0-node (non-zero `viewBox`) and 1-node degenerate graphs.

#### Scenario: Multi-node cycle preserves top-to-bottom forward flow and routes only feedback back-edges to right gutter
- **WHEN** `layout::compute_layout` runs on a normalized `ExportGraph` containing a 3-node cycle `A -> B -> C -> A` where `C -> A` is flagged `is_back_edge = true`
- **THEN** `rank[A] < rank[B] < rank[C]`, forward edges `A -> B` and `B -> C` flow top-to-bottom across ranks, only `C -> A` routes via an inter-rank corridor into the right-hand cycle gutter, and zero card or corridor overlaps occur

#### Scenario: Both slice anchors focal cursor node at waist with deterministic fallback
- **WHEN** `layout::compute_layout` runs on a `Both` slice with upstream origins feeding `root_ids` and downstream consumers leaving `root_ids`
- **THEN** all upstream nodes sit at ranks `< focal_rank`, `root_ids` sit at `focal_rank`, all downstream nodes sit at ranks `> focal_rank`, and if `root_ids` is empty or pruned by `--null-only` the layout falls back cleanly to standard longest-path without panicking or inverting edges

#### Scenario: Exceeding 600 dummy waypoints routes remaining long-span edges via left overflow gutter
- **WHEN** `layout::compute_layout` runs on an adversarial 200-node graph whose multi-rank forward edges require more than `600` dummy waypoints
- **THEN** `dummy_nodes_added <= 600`, `dummy_cap_hit == true`, overflow long-span edges route through interval-packed left-hand gutter lanes via inter-rank corridors with zero card-edge intersections, and layout finishes in `< 50ms`

#### Scenario: 0-node and 1-node graphs produce valid non-zero viewBox
- **WHEN** `layout::compute_layout` runs on an empty (0-node) or trivial (1-node) `ExportGraph`
- **THEN** `SvgLayout` returns a valid non-zero `viewBox` (`640 x 240`) with `empty_state = true` or `trivial_single = true` and zero NaN or division-by-zero geometry hazards

### Requirement: Strict-CSP, XSS-Safe, Deterministic Single-File HTML+SVG Serialization
`engine/src/export/html.rs` SHALL assemble a single self-contained offline HTML5 document (`<!DOCTYPE html>`) embedding `engine/src/export/assets/viewer.css` and `engine/src/export/assets/viewer.js` via `include_str!`, SHALL enforce `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; object-src 'none';">` (with zero `fetch()`, zero `localStorage`/cookies, zero external fonts/CDNs, and no invalid `frame-ancestors` in `<meta>`), SHALL emit zero wall-clock timestamps, zero host workspace folder names, and zero `CARGO_PKG_VERSION` strings in the HTML body so identical `ExportGraph` inputs produce byte-identical output across CLI and RPC, SHALL XML-escape and strip ASCII control characters on all SVG/HTML text while Unicode-escaping `<`, `>`, `&`, `U+2028`, and `U+2029` inside `<script type="application/json" id="lineage-meta">` (`viewer_schema_version = 1`) and `<script type="application/json" id="lineage-snippets">`, SHALL never emit raw snippets into SVG `<text>`, `<title>`, or `data-*` attributes, and SHALL render Coverage-Honesty glyphs and state pills in an unclipped card header zone with `<clipPath>` + UTF-8 middle-elision on variable text and sequential colorblind-safe `palette[file_idx % 12]` file indicator dots.

#### Scenario: Untrusted repository symbols and snippets cannot break out of script tags or inject HTML
- **WHEN** `html::render_html` serializes an `ExportGraph` whose symbol, label, `guard_text`, or snippet contains `</script><img onerror=alert(1)>`, quotes, ampersands, ASCII control bytes, and `U+2028`/`U+2029`
- **THEN** the output contains a strict `<meta>` CSP without `frame-ancestors`, zero unescaped `</script>` or raw `<img` tags, zero raw snippets inside SVG markup or attributes, and valid JSON islands

#### Scenario: Identical ExportGraph produces byte-identical HTML across CLI and RPC
- **WHEN** the same `ExportGraph` and `ExportOptions` are rendered through `lineage-engine export --format html` and the `lineage/export` NDJSON RPC
- **THEN** the resulting HTML bytes are identical, contain zero timestamps or host workspace folder names, and assign deterministic sequential `color_idx = file_idx % 12` values over sorted sanitized paths

### Requirement: Modular Offline Interactive Viewer and Leak-Free "Save Without Snippets"
`engine/src/export/assets/viewer.js` SHALL construct all dynamic DOM elements via `textContent` and `createElement`/`createElementNS` (never `innerHTML`), SHALL capture `const pristineRoot = document.documentElement.cloneNode(true)` on line 1 of browser boot before any DOM mutation, SHALL provide pan/zoom, `Fit View` / `Fit Active` (`f`) / `Center Focus` (initial framing fits `<= 25` nodes and centers primary focal node at `1.0` scale for `> 25` nodes), OR-composed `Null-only`/`Guards`/`Unverified` focus-dimming toggles with live counts (disabling zero-count filters, running causal-path BFS for `Null-only` matching `ir.rs`, and showing locked pills for export-time `--null-only`/`--exclude-tests`), clickable scrollable file legend, click-node transitive causal cone with `<g id="active-overlay">` edge elevation and collapsed-by-default side inspector with direct causal link hops (`Esc`/canvas click clears selection; `Reset`/`r` clears all), OS-adaptive `@media (prefers-color-scheme: dark)` theming with an in-memory Dark/Light toggle, an inverted synchronous `file://`-safe 3-step `Copy path:line` helper (trying `navigator.clipboard.writeText` only in secure contexts, running synchronous fixed `<textarea readonly>` + `execCommand("copy")` first on `file://`, and falling back to an inline pre-selected `<input readonly>` toast), and a **`Save Without Snippets`** button (hidden/disabled when `#lineage-snippets` is already `{}`) that clones `pristineRoot`, sets `#lineage-snippets` to `"{}"`, stamps `Snippets removed` and `meta.snippets_removed = true` via `safeJsonForScript`, prepends `<!DOCTYPE html>\n`, triggers a `Blob` download (`*-no-snippets.html`), and revokes the object URL after `1000ms`.

#### Scenario: "Save Without Snippets" strips snippets cleanly without leaking live DOM state or stored XSS
- **WHEN** a user inspects a snippet-bearing node whose symbol contains `</script><img onerror=alert(1)>`, pans/filters the canvas, and clicks `Save Without Snippets`
- **THEN** `buildSnippetFreeHtml(pristineRoot)` produces a standalone HTML document starting with `<!DOCTYPE html>`, with `#lineage-snippets` equal to `"{}"`, zero open inspector text or pan/zoom/filter state baked in, `Snippets removed` header badge stamped, and `</script>` Unicode-escaped inside `#lineage-meta`

#### Scenario: Clicking Copy path:line under file:// preserves user gesture activation
- **WHEN** a user clicks `Copy path:line` in the side inspector while viewing the HTML artifact from a `file://` origin (`window.isSecureContext === false`)
- **THEN** the viewer executes the synchronous fixed-position `<textarea readonly>` + `document.execCommand("copy")` path directly inside the click handler (without awaiting a failing Promise first), and if blocked displays an inline pre-selected `<input readonly>` toast with a copy hint

### Requirement: Destination-Aware Snippet Safety, Content-Addressed Private Cache, and Headless/SSH-Safe Opening
`lineage-engine` SHALL keep `include_snippets: None => false` uniformly across all formats (`mermaid`, `dot`, `html`) and remain stateless by returning HTML in `ExportResult.content`. `lua/lineage/export_cache.lua` SHALL own `stdpath("cache")/lineage/exports/` with `0700` directory creation, `uv.fs_lstat` symlink refusal, and non-Windows `uv.fs_chmod`, SHALL name cached files `lineage-<sanitized-slug>-<sha12>.html` via `vim.fn.sha256` (refreshing `mtime` via `uv.fs_utime` on cache hit, writing atomically via `uv.fs_open(tmp, "wx", 0600)` + `uv.fs_rename` on miss), SHALL lazily retain only the 20 most recent non-symlink regular `^lineage%-.*%.html$` files by `mtime` (plus cleaning orphaned `%.tmp$` files older than 1 hour), SHALL support `:Lineage export clean` returning removed file count and reclaimed bytes, and SHALL pre-flight skip `vim.ui.open` when `#vim.api.nvim_list_uis() == 0` or on non-WSL/non-macOS/non-Windows Linux with empty `DISPLAY` and `WAYLAND_DISPLAY`, wrapping `vim.ui.open` in `pcall` and always notifying with the exact written file path.

#### Scenario: Bare HTML export writes 0600 content-addressed cache file and deduplicates repeats
- **WHEN** a user runs `:Lineage export html` twice on an unchanged slice on desktop Linux/macOS
- **THEN** the first run creates a `0700` cache directory and atomic `0600`-from-birth `lineage-<slug>-<sha12>.html` file with `include_snippets = true` and opens it via `vim.ui.open`, and the second run reuses the same file while updating its `mtime` without consuming another slot in the 20-file cap

#### Scenario: Headless SSH Linux skips vim.ui.open without corrupting the terminal
- **WHEN** a user runs `:Lineage export html` over SSH on Linux with neither `DISPLAY` nor `WAYLAND_DISPLAY` set
- **THEN** `lua/lineage/export_cache.lua` writes the `0600` cached HTML file, skips `vim.ui.open` before invoking `xdg-open`, and notifies the user with the exact file path
