## MODIFIED Requirements

### Requirement: Versioned `SliceEnvelope` and Idempotent `GraphDelta` Wire Contract
All slice responses SHALL use a versioned `SliceEnvelope` (`schema_version: 1`) carrying `query_id`, `trace_id`, file table, optional canonical graph (opt-in via `include_graph`, omitted on default `< 50ms` hover responses), engine-computed projections (`hover_card`, and optional `focus_spans` and `tree_rows` when requested), truncation reasons, and query stats; Phase 2 updates SHALL stream as idempotent `lineage/sliceDelta` notifications, and lazy frontier/cluster expansion SHALL be served via `lineage/expand`.

#### Scenario: Phase 2 delta arrives for a stale `query_id`
- **WHEN** a `lineage/sliceDelta` notification arrives whose `query_id` does not match the active hover `query_id`
- **THEN** `lineage.nvim` discards the delta immediately without mutating any window or buffer

#### Scenario: Tree and Focus projections are additive and omitted when not requested
- **WHEN** a client sends `lineage/slice` without requesting `focus_spans` or `tree_rows`
- **THEN** `SliceEnvelope` remains backwards-compatible under `schema_version: 1` and omits those optional projection fields

### Requirement: Budgeted Parallel Location-Only LSP Bridge Without Synthetic `didOpen`
The LSP bridge SHALL query only structured location endpoints (`definition`, `typeDefinition`, `implementation` capped at 5 targets, above which emit `Unknown: polymorphic_cap`, plus 2-step `prepareCallHierarchy` -> `callHierarchy/incomingCalls` with automatic fallback to `textDocument/references` for "Data Flow From Here"), SHALL never call `textDocument/hover`, `bufload`, `:badd`, `:edit`, or synthetic `didOpen`/`didClose`, SHALL select the first non-circuit-broken attached LSP client supporting each method, and SHALL batch sibling unresolved symbols per BFS depth level with a `150ms` default per-request timeout inside the `250ms` Phase 2 ceiling.

#### Scenario: Per-client EWMA and circuit breaker isolate unresponsive LSP servers
- **WHEN** an attached LSP client exceeds 3 consecutive request timeouts while a second attached client responds within budget
- **THEN** the circuit breaker opens only for the timed-out `(lsp_client_name, workspace_root)` (probing half-open after 30s) while the healthy client continues serving Phase 2 normally

#### Scenario: Call hierarchy falls back cleanly to `textDocument/references` when unsupported or empty
- **WHEN** a `from` query invokes `prepareCallHierarchy` on an attached LSP server that does not support call hierarchy or returns `null` (such as `vtsls` on unopened files)
- **THEN** `lsp_compat.lua` falls back automatically to `textDocument/references` without raising an error or sending synthetic `didOpen`
