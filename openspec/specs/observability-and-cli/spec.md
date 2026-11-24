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
