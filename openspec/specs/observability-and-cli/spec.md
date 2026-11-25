## Purpose
Provide runtime observability, `:checkhealth lineage` diagnostics, in-editor debug commands, and standalone CLI tools for `lineage.nvim` and `lineage-engine`.

## Requirements

### Requirement: Protocol Handshake, Binary Discovery, and Skew Fail-Fast
On startup, `lineage.nvim` SHALL locate `lineage-engine` in precedence order (`opts.binary_path` -> version-keyed `stdpath('data')/lineage/bin/lineage-engine-v<semver>` -> local dev `engine/target/release/lineage-engine` -> `PATH`) and SHALL execute an `initialize` handshake exchanging `protocol_version` (`1`), `engine_version`, and capabilities, stopping the sidecar and notifying the user on mismatch.

#### Scenario: Incompatible engine binary fails fast with upgrade guidance
- **WHEN** `lineage.nvim` connects to a `lineage-engine` binary reporting a mismatched `protocol_version`
- **THEN** `lineage.nvim` stops the sidecar and displays an error naming the expected and received protocol versions

### Requirement: Built-In Observability, Health Checks, and Standalone CLI Debug Commands
`lineage.nvim` and `lineage-engine` SHALL propagate `trace_id` across all async hops (including `lineage/expand` as an internal Rust argument for tracing logs), initialize engine tracing at default level `info` (overridable via `RUST_LOG`) emitting source-free structured per-query events (`trace_id`, `query_id`, `method`, `uri`, `phase1_ms`, `phase2_ms`, `truncation`, `unknown_count`) to a daily rotating log file, expose additive live current-PID process `rss_bytes` on `lineage/status` (using single-PID `sysinfo` memory refresh), maintain a bounded 100-entry client telemetry ring in `lua/lineage/client.lua`, glob `<log_path>*` (selecting the newest `rolling::daily` `.YYYY-MM-DD` file, tailing up to 200 lines) alongside `stderr` in `:Lineage log`, display `engine_tag`, binary provenance (`release` vs `source_build`), `rss_bytes`, call-hierarchy/references LSP capabilities, and `:Lineage install` recovery guidance in `:checkhealth lineage` and `:Lineage status`, and expose standalone CLI commands (`dump-ir`, `dump-cfg --dot`, `slice --trace`) in `lineage-engine`.

#### Scenario: Running `:checkhealth lineage` reports runtime diagnostics, engine_tag, call-hierarchy capabilities, and OS RSS
- **WHEN** a user executes `:checkhealth lineage`
- **THEN** Neovim displays Neovim version (`>= 0.10`), binary path with `engine_tag` and provenance (`release` vs `source_build`), protocol match, sidecar ping latency, attached LSP clients with `offset_encoding` and method capabilities (`definition`, `typeDefinition`, `implementation`, `prepareCallHierarchy`, `references`), loaded grammars, tracked LRU cache bytes vs configured cap, live OS `rss_bytes`, per-client EWMA and circuit-breaker state, and rotating log path

#### Scenario: Running `:Lineage log` resolves daily-rotated log files alongside stderr
- **WHEN** a user runs `:Lineage log` after slicing queries with default `RUST_LOG`
- **THEN** Neovim globs `<log_path>*` to open the newest `lineage-engine.log.YYYY-MM-DD` file (falling back to exact `log_path`) and displays up to 200 recent source-free structured entries alongside any `stderr` ring-buffer lines

#### Scenario: Running `:checkhealth lineage` when binary is missing suggests `:Lineage install`
- **WHEN** a user executes `:checkhealth lineage` and no `lineage-engine` binary is found
- **THEN** Neovim reports an error recommending `:Lineage install` (or `cargo build --release` in `engine/`)

#### Scenario: Standalone CLI reproduces a slice outside Neovim
- **WHEN** a developer runs `lineage-engine slice src/auth.ts:42:10 --format json --trace`
- **THEN** `lineage-engine` prints the deterministic `SliceEnvelope` JSON and step-by-step lattice/guard transitions without requiring Neovim or a live LSP server

### Requirement: Unified Standalone CLI Export Command and Format Enum Split
`engine/src/cli.rs` and `engine/src/main.rs` SHALL split `CliFormat` into `SliceCliFormat { Json, Text, Dot }` and `ExportCliFormat { Mermaid, Dot }` (rejecting invalid format/subcommand pairings at parse time), SHALL add `Both` to `CliDirection`, SHALL add `Commands::Export` accepting `<file>:<line>:<col>` or `--from-envelope <path|->`, `--format`, `--direction`, `--workspace-root` (defaulting to nearest `.git` ancestor or current working directory—never `file.parent()`), `--exclude-tests`, `--null-only`, `--no-lsp`, `--lsp-fixture`, tri-state `--snippets`/`--no-snippets` (`conflicts_with`, resolving to `Option<bool>`), `--absolute-paths`, and `--out <path>` (stdout when omitted), and SHALL update `Commands::Slice` so `--lsp-fixture` enables Phase 2 and `--format dot` delegates to `export::render_export`.

#### Scenario: Standalone CLI export resolves nearest workspace root and rejects invalid formats
- **WHEN** a developer runs `lineage-engine export src/auth.ts:42:8 --format mermaid --direction both --no-lsp` inside a git repository
- **THEN** `lineage-engine` resolves `--workspace-root` to the git root (not `src/`), emits deterministic Mermaid to stdout, and rejects `lineage-engine export ... --format json` at CLI parse time

### Requirement: Source-Free Export Observability and Hardened NDJSON Client Transport
`engine/src/export/` SHALL emit a source-free `tracing::info!` span for every CLI and RPC export (`trace_id`, `query_id`, `format`, `source_mode`, `phase2_status`, `phase2_wait_ms`, `render_ms`, `node_count`, `edge_count`, `output_bytes`, `truncation`—zero snippets, labels, or URIs). `lua/lineage/client.lua` SHALL expose `M.capabilities()`, clear `state.capabilities` and `state.buf` on `stop()` and process exit, accumulate incoming NDJSON pipe chunks in a chunk array (`table.concat` on `"\n"`) rather than quadratic string concatenation, reject pending request callbacks cleanly on `vim.json.decode` errors, and record `lineage/export` entries (`format`, `source_mode`, `phase2_status`, `rtt_ms`, `output_bytes`) in `telemetry_ring`. `lua/lineage/debug.lua:show_status` SHALL format `lineage/export` entries cleanly instead of printing `phase1=nilms`.

#### Scenario: Large multi-chunk export response and decode errors are handled without stalls or hangs
- **WHEN** `lineage-engine` returns a multi-chunk `lineage/export` NDJSON response (or a corrupted JSON frame)
- **THEN** `lua/lineage/client.lua` buffers chunks without O(N²) reallocation, resolves the callback and records export telemetry formatted cleanly in `:Lineage status`, or rejects the pending callback cleanly on decode error instead of hanging

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
