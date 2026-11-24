## MODIFIED Requirements

### Requirement: Built-In Observability, Health Checks, and Standalone CLI Debug Commands
`lineage.nvim` and `lineage-engine` SHALL propagate `trace_id` across all async hops, write source-free rotating logs (paths, symbol names, timings, and state tags only), provide `:checkhealth lineage`, `:Lineage status`, `:Lineage log`, and `:Lineage debug-dump` inside Neovim, surface `:Lineage install` recovery guidance in `:checkhealth lineage` when the sidecar binary is missing, and expose standalone CLI commands (`dump-ir`, `dump-cfg --dot`, `slice --trace`) in `lineage-engine`.

#### Scenario: Running `:checkhealth lineage` reports runtime diagnostics
- **WHEN** a user executes `:checkhealth lineage`
- **THEN** Neovim displays Neovim version (`>= 0.10`), binary path and protocol match, sidecar ping latency, attached LSP clients with `offset_encoding` and method capabilities, loaded grammars, tracked LRU cache bytes vs configured cap, per-client EWMA and circuit-breaker state, and rotating log path

#### Scenario: Running `:checkhealth lineage` when binary is missing suggests `:Lineage install`
- **WHEN** a user executes `:checkhealth lineage` and no `lineage-engine` binary is found
- **THEN** Neovim reports an error recommending `:Lineage install` (or `cargo build --release` in `engine/`)

#### Scenario: Standalone CLI reproduces a slice outside Neovim
- **WHEN** a developer runs `lineage-engine slice src/auth.ts:42:10 --format json --trace`
- **THEN** `lineage-engine` prints the deterministic `SliceEnvelope` JSON and step-by-step lattice/guard transitions without requiring Neovim or a live LSP server
