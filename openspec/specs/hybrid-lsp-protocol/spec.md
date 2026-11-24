## ADDED Requirements

### Requirement: Full-Duplex JSON-RPC 2.0 NDJSON Transport and Cancellation
`lineage-engine` and `lineage.nvim` SHALL communicate via single-line UTF-8 JSON-RPC 2.0 NDJSON over stdio with non-colliding ID namespaces (`"c-<n>"` for Neovim client requests, `"e-<n>"` for engine reverse-RPC requests), `stdout` isolation (logs to a source-free rotating file, panics to `stderr`), centralized monotonic `query_id` allocation across Hover/Focus/Tree in `lua/lineage/client.lua`, a dedicated `init_waiters` queue for concurrent `ensure_started` callers (drained on `initialize`, `M.stop()`, and early process exit), inline `(query_id, oneshot::Sender)` reverse-RPC tracking in `TransportState.pending_reverse` so `$/cancelRequest` and `shutdown` abort blocked reverse-RPC waits immediately without tripping EWMA timeouts, an RAII drop guard ensuring `finish_query(qid)` runs on all exits, and main-loop safe callback scheduling in Lua.

#### Scenario: Cursor movement cancels in-flight Phase 2 and Neovim LSP requests without tripping EWMA
- **WHEN** the user triggers a jump or moves the cursor while Phase 2 reverse-LSP requests are in flight
- **THEN** `lineage.nvim` sends `$/cancelRequest` for the superseded `query_id`, `lineage-engine` cancels the worker token and drops matching `pending_reverse` senders (`Disconnected`, not `Timeout`), EWMA does not record a timeout, and Lua cancels pending Neovim LSP requests

#### Scenario: Concurrent startup callers all resume on initialize completion or early exit
- **WHEN** two Neovim UI modules invoke `client.ensure_started` concurrently while `initialize` is in flight (or the sidecar exits early)
- **THEN** both callbacks are queued in `init_waiters` and invoked cleanly with success or error without hanging

### Requirement: Versioned `SliceEnvelope` and Idempotent `GraphDelta` Wire Contract
All slice responses SHALL use a versioned `SliceEnvelope` (`schema_version: 1`) carrying `query_id`, `trace_id`, file table, optional canonical graph (opt-in via `include_graph`), engine-computed projections (`hover_card`, and optional `focus_spans` and `tree_rows` when requested), truncation reasons, and query stats; `transport.rs` SHALL flush the Phase 1 `RpcResponse` onto `stdout` *before* dispatching Phase 2 via non-blocking `try_send` onto a dedicated bounded Phase 2 worker pool (2–4 threads, drained before closing `out_tx` on shutdown) that streams `lineage/sliceDelta` notifications; Phase 2 resolution SHALL correlate `LspResolveItemResult` back to `LspResolveItem.kind` by index/key, prune resolved `unknown_boundaries` (clearing `ExternalUntyped` only when zero `ExternalUntyped` entries remain), project `CallHierarchyIncoming` callers into `tree_rows` (with `tree.lua` subscribing to `on_delta`), preserve unicode hover symbol headers while updating `status_badge` and `is_trivial_peek`, and clear prior buffer extmarks on re-focus/re-pin while cancelling active queries in `focus.lua:clear()`.

#### Scenario: Phase 1 response arrives before Phase 2 delta and hover applies updates cleanly
- **WHEN** `:Lineage hover` runs on a buffer with an attached LSP and unresolved cross-file calls
- **THEN** `lineage-engine` writes the Phase 1 `RpcResponse` first so the float opens in `< 50ms`, and `lineage/sliceDelta` arrives afterward to append `[k+1..9]`, update `status_badge` and `verdict_line`, and remove resolved symbols from `unknown_boundaries`

#### Scenario: Phase 2 delta arrives for a stale `query_id`
- **WHEN** a `lineage/sliceDelta` notification arrives whose `query_id` does not match the active hover `query_id`
- **THEN** `lineage.nvim` discards the delta immediately without mutating any window or buffer

#### Scenario: `:Lineage from` populates Tree Panel when an LSP is attached
- **WHEN** the user executes `:Lineage from` with an attached LSP server
- **THEN** Phase 2 correlates `CallHierarchyIncoming` results (or falls back to bounded `discover_callers`), projects callers into `tree_rows` inside `lineage/sliceDelta`, and `tree.lua` renders them in the Tree Panel

### Requirement: Budgeted Parallel Location-Only LSP Bridge Without Synthetic `didOpen`
The LSP bridge SHALL query only structured location endpoints (`definition`, `typeDefinition`, `implementation` capped at 5 targets, plus 2-step `prepareCallHierarchy` -> `callHierarchy/incomingCalls` with fallback to `textDocument/references`), SHALL never call `textDocument/hover`, `bufload`, `:badd`, `:edit`, or synthetic `didOpen`/`didClose`, SHALL convert engine UTF-8 byte columns to each client's `offset_encoding` (`utf-16`/`utf-32`/`utf-8`) in `lsp_compat.lua` using Neovim `0.10`/`0.11+`-compatible `vim.str_utfindex` (reading unloaded file lines directly from disk and checking `item.offset_encoding` to prevent double-conversion), SHALL pass `(workspace_root, lsp_client_name)` internally into `SymbolResolver::resolve_batch`, SHALL forward `circuit_open_clients` and `max_timeout_ms` on `LspResolveBatchParams` plus `lsp_timeout_ms` on `SliceParams`, SHALL select `lsp_client_name` via capability-aware `lsp_compat.pick_client` and populate `client_name` on timed-out items, and SHALL honor EWMA adaptive timeouts bounded by configured `lsp.timeout_ms`.

#### Scenario: Per-client EWMA and circuit breaker isolate unresponsive LSP servers
- **WHEN** an attached LSP client exceeds 3 consecutive request timeouts while a second attached client responds within budget
- **THEN** `LspResolveBatchParams.circuit_open_clients` includes the timed-out client, `lsp_compat.pick_client` skips it while the healthy secondary client continues serving Phase 2 normally, and the tripped client probes half-open after 30s

#### Scenario: Reverse-RPC translates UTF-8 byte columns to UTF-16 on multi-byte lines across Neovim 0.10 and 0.11
- **WHEN** `lineage-engine` sends a reverse-RPC `LspResolveItem` on a line containing multi-byte UTF-8 characters (loaded or unloaded buffer) to a UTF-16 LSP client on Neovim `0.10.4` or `0.11.0`
- **THEN** `lsp_compat.lua` reads the line without `bufload` and converts the UTF-8 byte column using the version-appropriate `vim.str_utfindex` signature before calling `client:request`

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
