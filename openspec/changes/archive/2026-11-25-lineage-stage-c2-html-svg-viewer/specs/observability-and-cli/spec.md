## ADDED Requirements

### Requirement: Standalone CLI HTML Export Contract
`engine/src/cli.rs` and `engine/src/main.rs` SHALL add `Html` to `ExportCliFormat` (keeping `SliceCliFormat` unchanged so `lineage-engine slice --format html` is rejected at parse time), SHALL default `lineage-engine export ... --format html` to `include_snippets = false` (opt-in via `--snippets`), SHALL write rendered HTML to `stdout` when `--out` is omitted or to `--out <path>` when provided, and SHALL never auto-open a browser or touch Neovim's cache directory.

#### Scenario: CLI HTML export writes deterministic snippet-free HTML to --out without opening a browser
- **WHEN** a developer runs `lineage-engine export src/auth.ts:42:8 --format html --direction both --no-lsp --out /tmp/flow.html`
- **THEN** `lineage-engine` writes deterministic, snippet-free HTML to `/tmp/flow.html`, does not spawn a browser, and does not create or prune files in Neovim's cache directory

### Requirement: Source-Free HTML Layout Telemetry and Cache/Opener Health Diagnostics
`engine/src/export/` SHALL emit a source-free `tracing::info!` span for every `format = "html"` export containing `layout_ms`, `serialize_ms`, `node_count`, `edge_count`, `rank_count`, `dummy_nodes_added`, `dummy_cap_hit`, `feedback_back_edges`, `overflow_edges`, `crossing_sweeps`, `svg_width`, `svg_height`, `output_bytes`, `truncated_nodes`, and `truncated_edges` (zero snippets, labels, symbols, or URIs). `lua/lineage/debug.lua:show_status` SHALL display `open_status` and `cache_pruned` on `lineage/export` telemetry entries, and `lua/lineage/health.lua:check` SHALL report `capabilities.export_formats` (flagging missing `"html"` with `:Lineage install!`), export cache directory existence, `0700` permissions (on non-Windows), cached file count, total bytes, and browser-open display readiness.

#### Scenario: Running :checkhealth lineage reports HTML capability, private cache health, and display readiness
- **WHEN** a user runs `:checkhealth lineage` after Stage C.2 is installed
- **THEN** Neovim displays `capabilities.export_formats` (warning with `:Lineage install!` if `"html"` is absent), `stdpath("cache")/lineage/exports` permission and file-count/byte status, and whether the environment supports browser opening or degrades to path-only output
