## ADDED Requirements

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
