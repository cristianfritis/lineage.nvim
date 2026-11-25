## Why

After Stage C.1 ([docs/designs/lineage-stage-c1-graph-exporters.md](../../../docs/designs/lineage-stage-c1-graph-exporters.md)) introduces Mermaid and Graphviz DOT exporters on top of the Interactive Tree Panel, medium-to-large nullability investigations (15–120 nodes, up to 200 nodes) still lack an offline interactive graph view: Mermaid becomes cramped past ~20–25 nodes, DOT requires a local Graphviz install and is static, and the Tree Panel duplicates converging subtrees in multi-file diamonds, recursive cycles, and `Both` slices. Stage C.2 adds a 100% offline, deterministic interactive HTML+SVG viewer directly on top of Stage C.1's frozen `ExportGraph` IR and `lineage/export` RPC with zero new Cargo crates and zero breaking wire or schema changes.

## What Changes

- **Sequencing Prerequisite:** Stage C.1 (including pre-freeze folds `C1-1`–`C1-6` in [docs/designs/lineage-stage-c1-graph-exporters.md](../../../docs/designs/lineage-stage-c1-graph-exporters.md)) must be implemented, verified, and frozen before Stage C.2 implementation begins.
- **Additive Engine Export Format (`format = "html"`):**
  - Register `ExportFormat::Html` (`"html"`), advertise `"html"` in `EngineCapabilities.export_formats`, and add `Html` to `ExportCliFormat` (keeping `SliceCliFormat` unchanged so `slice --format html` stays invalid).
  - Add pure-Rust integer-coordinate layered-DAG SVG layout (`engine/src/export/layout.rs`) consuming `&ExportGraph` exclusively (`<= 200` nodes, `<= 800` edges, `< 50ms`, zero AABB overlaps, `Both` focal waist pinning, bounded `<= 600` dummy waypoints, and a shared `O(k log k)` interval lane packer for right-hand cycle back-edges and left-hand overflow long-span edges routed through inter-rank horizontal corridors).
  - Add single-file offline HTML+SVG serializer (`engine/src/export/html.rs`) embedding sibling assets (`engine/src/export/assets/{viewer.css, viewer.js}` via `include_str!`) with strict `<meta>` CSP, dual Unicode-escaped JSON islands (`#lineage-meta` and `#lineage-snippets`), zero raw snippets in SVG/attributes, zero wall-clock timestamps or host workspace folder names, sequential colorblind-safe `palette[file_idx % 12]` file accent strips, OR-composed `Null-only`/`Guards`/`Unverified` filters with live counts, click-node causal cone focus-dimming, inverted synchronous `file://` 3-step `Copy path:line`, OS-adaptive dark/light CSS with toolbar toggle, and `"Save Without Snippets"` via pre-boot `pristineRoot` clone.
- **Stateless Engine & Destination-Aware Privacy Defaults:**
  - Keep `lineage-engine` stateless (`ExportResult.content` over NDJSON RPC; `stdout` or `--out <path>` in CLI, never auto-opening a browser or touching Neovim's cache) with uniform `include_snippets: None => false` across all formats.
  - Pass `include_snippets = true` from `lua/lineage/export.lua` only on bare ephemeral private-cache HTML opens (`:Lineage export html` with no explicit path, unless `--no-snippets` is passed); explicit `:Lineage export html <path>` / `--out <path>` saves silently with `include_snippets = false` by default (opt-in `--open` / `--snippets`).
- **Neovim Private Cache, Commands, `<Plug>` & Observability:**
  - Add `lua/lineage/export_cache.lua` managing `stdpath("cache")/lineage/exports/` (`0700` dir with symlink rejection, atomic `0600`-from-birth content-addressed `lineage-<slug>-<sha12>.html` files, count-only 20-file lazy prune by `mtime`, `:Lineage export clean`, and headless/SSH/WSL-safe `vim.ui.open`).
  - Add `--open` / `--no-open`, `:Lineage export clean`, `<Plug>(LineageExportHtml)`, `export.html = { open = true, cache_max_files = 20 }`, exact format membership check in `EngineCapabilities.export_formats`, source-free HTML layout telemetry in `tracing::info!`, and `:Lineage status` / `:checkhealth lineage` cache and opener diagnostics.

## Capabilities

### New Capabilities
- `html-svg-viewer`: Pure-Rust layered-DAG SVG coordinate layout, strict-CSP offline single-file HTML+SVG serialization, interactive vanilla JS viewer (`viewer.js`/`viewer.css`), destination-aware snippet isolation and `"Save Without Snippets"`, content-addressed `0700`/`0600` Neovim cache (`export_cache.lua`), and headless/SSH/WSL-safe browser launching.

### Modified Capabilities
- `observability-and-cli`: Extend `ExportCliFormat` with `Html` (`stdout`/`--out`, never auto-open, default `--no-snippets`), emit source-free HTML layout/serialization `tracing::info!` metrics, and extend `:Lineage status` and `:checkhealth lineage` with export cache health (`0700` permissions, file count, bytes), `export_formats`, and browser-open readiness.
- `neovim-ui`: Extend `:Lineage export` with `html` delivery (bare private-cache auto-open vs. silent explicit-path save), `:Lineage export clean`, `--open` / `--no-open`, `<Plug>(LineageExportHtml)`, `export.html` config defaults, `HTML (browser)` picker entry, and exact format membership check against `EngineCapabilities.export_formats`.

## Impact

- **Rust Engine (`engine/`):** `src/protocol/{schema.rs, transport.rs}`, `src/{cli.rs, main.rs}`, `src/export/{mod.rs, layout.rs, html.rs}`, `src/export/assets/{viewer.css, viewer.js}`, `tests/export_html.rs`, `tests/viewer_js.test.mjs`. Zero new Cargo crates; Rust 1.82 MSRV preserved; `< 100 KB` release binary delta over Stage C.1.
- **Neovim Plugin (`lua/`, `plugin/`, `tests/`):** `lua/lineage/{config.lua, export_cache.lua, export.lua, commands.lua, debug.lua, health.lua}`, `plugin/lineage.lua`, `tests/test_export_html.lua`.
- **Docs & Specs:** `README.md`, `doc/lineage.txt`, and OpenSpec capabilities (`html-svg-viewer`, `observability-and-cli`, `neovim-ui`; `core-ir-and-slicer` untouched).
- **Backward Compatibility:** `PROTOCOL_VERSION = 1` and `SCHEMA_VERSION = 1` remain backward-compatible; Stage C.2 Lua fails fast with `:Lineage install!` against Stage C.1 binaries lacking `"html"` in `export_formats`.
