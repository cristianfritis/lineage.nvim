## Why

With M0+M1 (Core IR, TS/JS, Interactive Two-Phase Hover) and M2 (Python, Rust, Go `(val, err)` IR validation) complete, developers can inspect single-symbol provenance via Hover, but still lack scoped in-buffer reading isolation and multi-hop upstream/downstream investigation across files. M3 delivers the remaining v1 interactive workflows—Inline Focus Mode with Cross-Buffer Pinning, the Interactive Tree Panel, 2-step LSP call-hierarchy resolution, and bounded parallel Rust ripgrep+Tree-sitter standalone caller discovery.

## What Changes

- **Engine Projections & Wire Schema (additive `schema_version: 1`):**
  - Add `FocusSpan` and `TreeRow` projections to `SliceProjections` (`engine/src/protocol/schema.rs`) and pure Rust projectors in `engine/src/slicer/projectors.rs` (including `To` / `From` / `Both` directions, grouping by Root Origin or Nullability, null-only filtering, cycle badges, and opaque `expansion_token` for `FrontierNode` / `ClusterNode` lazy expansion).
  - Add `lineage/expand` RPC handler for lazy frontier/cluster expansion in `engine/src/protocol/transport.rs`.
- **Bounded Standalone Caller Discovery (`engine/src/resolver/ripgrep.rs`):**
  - Add `ignore`, `memchr`, and `regex` crates to `engine/Cargo.toml` for parallel standalone "From Here" caller discovery with language-extension filters, TS/JS quoted-specifier prefiltering, hard quotas (`min(4, cpus/2)` threads, `<= 5,000` files walked, `<= 300` files parsed, `<= 2s` wall-clock deadline), Tree-sitter call-site verification, cold-scan cache isolation, and partial truncation badges.
- **2-Step LSP Call Hierarchy & `vtsls` Fallback (`lua/lineage/lsp_compat.lua` + `engine/src/protocol/schema.rs`):**
  - Extend `LspResolveKind` with `CallHierarchyIncoming` (`prepareCallHierarchy` -> `callHierarchy/incomingCalls`) and `References`, automatically falling back to `textDocument/references` or standalone ripgrep when unsupported or on `vtsls` unopened-file stops.
- **Inline Focus Mode & Cross-Buffer Pinning (`lua/lineage/ui/focus.lua`):**
  - Implement `:Lineage focus`, `:Lineage pin`, `:Lineage clear`, and Hover pivots `gs`/`gp`.
  - Dim unrelated lines via `Comment`-linked extmarks, highlight slice statements with text-paired state badges (`[NULL]`, `[OK]`, `[?]`), display an active-mode indicator, mark slices `[stale]` immediately on `TextChanged`, and refresh on debounced idle or manual refresh.
- **Interactive Tree Panel (`lua/lineage/ui/tree.lua`):**
  - Implement `:Lineage to`, `:Lineage from`, `:Lineage both`, and Hover pivots `gt`/`gf`/`gn`.
  - Render collapsible `tree_rows` in a split with grouping toggles, null/absence filter, pre-budget test-file toggle, cycle badges, `[partial]` truncation badges, synchronized floating code preview, jump redirection (pushing to native jumplist + Lineage Jump Stack), and lazy `FrontierNode`/`ClusterNode` expansion.

## Capabilities

### New Capabilities
- `standalone-ripgrep-callers`: Bounded parallel Rust caller discovery (`ignore` + boundary-checked `memchr` + Tree-sitter call-site verification) with TS/JS quoted-specifier prefiltering, hard file/parse/time quotas, cold-scan cache isolation, and `[partial]` truncation telemetry.
- `inline-focus-mode`: Neovim Inline Focus Mode and Cross-Buffer Variable Pinning (`:Lineage focus`, `:Lineage pin`, `:Lineage clear`, Hover `gs`/`gp`) with `Comment`-linked dimming, semantic extmark badges, active-mode indicator, and `[stale]`-on-edit debounced refresh lifecycle.
- `interactive-tree-panel`: Neovim collapsible Tree Panel (`:Lineage to`, `:Lineage from`, `:Lineage both`, Hover `gt`/`gf`/`gn`) consuming engine-projected `tree_rows`, supporting grouping, null filtering, pre-budget test toggle, synchronized code preview, and lazy `FrontierNode`/`ClusterNode` expansion.

### Modified Capabilities
- `hybrid-lsp-protocol`: Extend location-only LSP bridge with 2-step `prepareCallHierarchy` -> `callHierarchy/incomingCalls` (and `textDocument/references` fallback) for "Data Flow From Here", additive `focus_spans`/`tree_rows` projections in `SliceEnvelope`, and `lineage/expand` RPC.

## Impact

- **Engine Dependencies:** Adds `ignore` and `memchr` to `engine/Cargo.toml`.
- **Engine Modules:** Adds `engine/src/resolver/ripgrep.rs`; updates `engine/src/protocol/schema.rs`, `engine/src/protocol/transport.rs`, `engine/src/slicer/mod.rs`, `engine/src/slicer/projectors.rs`.
- **Neovim Lua Modules:** Adds `lua/lineage/ui/focus.lua`, `lua/lineage/ui/tree.lua`; updates `plugin/lineage.lua`, `lua/lineage/init.lua`, `lua/lineage/config.lua`, `lua/lineage/lsp_compat.lua`, `lua/lineage/ui/hover.lua`, `lua/lineage/ui/highlights.lua`.
- **Tests:** Adds `engine/tests/ripgrep_callers.rs`, `tests/test_focus.lua`, `tests/test_tree.lua`, and extends `tests/test_lsp_compat.lua`.
