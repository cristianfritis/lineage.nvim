## ADDED Requirements

### Requirement: Full-Duplex JSON-RPC 2.0 NDJSON Transport and Cancellation
`lineage-engine` and `lineage.nvim` SHALL communicate via single-line UTF-8 JSON-RPC 2.0 NDJSON over stdio with non-colliding ID namespaces (`"c-<n>"` for Neovim client requests, `"e-<n>"` for engine reverse-RPC requests), `stdout` isolation (logs to a source-free rotating file, panics to `stderr`), bidirectional `$/cancelRequest`, and main-loop safe callback scheduling in Lua.

#### Scenario: Cursor movement cancels in-flight Phase 2 and Neovim LSP requests
- **WHEN** the user triggers a jump or moves the cursor while Phase 2 reverse-LSP requests are in flight
- **THEN** `lineage.nvim` sends `$/cancelRequest` for the superseded `query_id`, `lineage-engine` cancels the worker token, and Lua cancels pending Neovim LSP requests

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

### Requirement: Sidecar Crash Recovery, Dirty-Buffer Sync, and Neovim 0.10/0.11 Compatibility
`lineage.nvim` SHALL cap automatic sidecar restarts at 3 attempts, SHALL clear its synced `b:changedtick` map and reject pending callbacks on sidecar exit, SHALL skip `lineage/syncBuffer` when a dirty buffer exceeds 1 MB on the Lua side, SHALL branch Neovim 0.10 vs. 0.11+ LSP APIs behind a compatibility layer, and `lineage-engine` SHALL translate all UTF-8/UTF-16/UTF-32 LSP positions centrally.

#### Scenario: Sidecar restart re-syncs dirty buffers without stale disk reads
- **WHEN** `lineage-engine` exits unexpectedly while a buffer has unsaved edits and restarts on the next hover
- **THEN** `lineage.nvim` resets its synced `b:changedtick` table and re-sends `lineage/syncBuffer` before slicing

#### Scenario: Unsaved multi-byte UTF-8 edits align with LSP positions
- **WHEN** a user edits a line containing multi-byte UTF-8 characters without saving and triggers hover
- **THEN** `lineage-engine` slices the synced dirty buffer text and translates coordinates accurately for the attached client's `offset_encoding`

### Requirement: Deterministic Standalone Resolver Test Seam
`lineage-engine` SHALL isolate external LSP lookups behind a transport-agnostic provider interface supporting pure-static execution and recorded-fixture replay (with simulated latencies and timeouts) for deterministic CI testing outside Neovim.

#### Scenario: Recorded LSP fixtures test Phase 2 timeout and circuit breaker without Neovim
- **WHEN** a test invokes `lineage-engine` with a fixture provider configured for 3 consecutive timeouts followed by a 20ms reply after cooldown
- **THEN** Phase 1 returns within budget, the circuit breaker trips and recovers in half-open mode deterministically, and zero Neovim or external LSP processes are required
