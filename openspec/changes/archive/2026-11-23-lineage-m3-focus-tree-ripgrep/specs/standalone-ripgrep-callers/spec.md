## ADDED Requirements

### Requirement: Bounded Parallel Standalone Caller Discovery
`lineage-engine` SHALL implement standalone "Data Flow From Here" caller discovery in Rust using `ignore::WalkParallel`, word-boundary identifier matching (`memchr::memmem`), language-extension filtering, TS/JS quoted-specifier prefiltering, and Tree-sitter call-site verification under hard quotas (`min(4, cpus/2)` threads, `<= 5,000` files walked, `<= 300` files parsed, `<= 2s` wall-clock deadline).

#### Scenario: Standalone "From Here" finds cross-file callers without an attached LSP
- **WHEN** a user runs a `from` slice on an exported function `findSession` in a multi-file repository with no LSP server attached
- **THEN** `lineage-engine` discovers calling sites across workspace files using parallel `ignore` traversal and Tree-sitter call-site verification without stalling Neovim's main loop

#### Scenario: Quota or wall-clock cap terminates scan cleanly and marks partial results
- **WHEN** a standalone caller scan hits the walked-file cap, parsed-file cap, or `2s` deadline
- **THEN** workers return `WalkState::Quit` immediately, emit a `TruncationReason` so the UI displays `[partial]`, and do not evict hot entries from `FileIrCache`
