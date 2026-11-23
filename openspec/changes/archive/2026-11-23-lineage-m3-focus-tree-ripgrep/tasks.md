## 1. Engine Schema, Projections & `lineage/expand` RPC

- [x] 1.1 Add `ignore` and `memchr` dependencies to `engine/Cargo.toml`.
- [x] 1.2 Extend `engine/src/protocol/schema.rs` additively (`schema_version: 1`) with `SliceDirection::Both`, `FocusSpan`, `TreeRow`, `ExpandParams`, `ExpandResult`, `LspResolveKind::CallHierarchyIncoming`, and `LspResolveKind::References`.
- [x] 1.3 Implement pure Rust `project_focus_spans` and `project_tree_rows` in `engine/src/slicer/projectors.rs` (including grouping keys, nullability flags, cycle badges, and `FrontierNode`/`ClusterNode` `expansion_token`) and wire `lineage/expand` in `engine/src/protocol/transport.rs`.

## 2. Bounded Standalone Ripgrep + Tree-sitter Caller Discovery

- [x] 2.1 Implement `engine/src/resolver/ripgrep.rs` using `ignore::WalkParallel`, word-boundary `memchr::memmem`, language-extension filters, TS/JS quoted-specifier prefiltering, hard quotas (`min(4, cpus/2)` threads, `<= 5,000` files walked, `<= 300` files parsed, `<= 2s` deadline returning `WalkState::Quit`), Tree-sitter call-site verification, and cold-scan cache isolation.
- [x] 2.2 Wire standalone caller discovery into `SlicerEngine::slice_query` for `SliceDirection::From` and `SliceDirection::Both` when LSP is unattached or truncated, emitting `[partial]` truncation metadata when caps fire.

## 3. 2-Step LSP Call Hierarchy & `references` Fallback

- [x] 3.1 Extend `lua/lineage/lsp_compat.lua` to handle `callHierarchyIncoming` (`textDocument/prepareCallHierarchy` -> `callHierarchy/incomingCalls`) with automatic fallback to `textDocument/references` when unsupported or returning `null`/empty (covering `vtsls` unopened-file behavior).

## 4. Inline Focus Mode & Cross-Buffer Pinning

- [x] 4.1 Implement `lua/lineage/ui/focus.lua` rendering `focus_spans` via dedicated extmark namespace (`LineageFocusDim` linked to `Comment`, semantic state highlights + ASCII/Unicode badges, active-mode/pin indicator, immediate `[stale]` transition on `TextChanged`/`TextChangedI`, and debounced idle refresh).
- [x] 4.2 Wire `:Lineage focus`, `:Lineage pin`, `:Lineage clear`, `<Plug>(LineageFocus)`, `<Plug>(LineagePin)`, and Hover pivots `gs` / `gp` in `plugin/lineage.lua`, `lua/lineage/init.lua`, `lua/lineage/ui/hover.lua`, and `lua/lineage/ui/highlights.lua`.

## 5. Interactive Tree Panel

- [x] 5.1 Implement `lua/lineage/ui/tree.lua` rendering collapsible `tree_rows` in a split with `To`/`From`/`Both` modes, grouping toggles (`Root Origin` / `Nullability`), null-only filter, pre-budget test-file toggle, cycle and `[partial]` badges, synchronized non-focusable floating code preview, jump redirection (native jumplist + Lineage Jump Stack), and lazy `FrontierNode`/`ClusterNode` expansion via `lineage/expand`.
- [x] 5.2 Wire `:Lineage to`, `:Lineage from`, `:Lineage both`, `<Plug>(LineageTo)`, `<Plug>(LineageFrom)`, and Hover pivots `gt` / `gf` / `gn` in `plugin/lineage.lua`, `lua/lineage/init.lua`, `lua/lineage/config.lua`, and `lua/lineage/ui/hover.lua`.

## 6. Verification & Tests

- [x] 6.1 Add `engine/tests/ripgrep_callers.rs` testing standalone TS/Python/Rust caller discovery, quoted-specifier prefiltering, cap truncation, and cache isolation.
- [x] 6.2 Add headless Neovim tests `tests/test_focus.lua`, `tests/test_tree.lua`, and extend `tests/test_lsp_compat.lua` for `prepareCallHierarchy` -> `references` fallback.
- [x] 6.3 Run `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release`, and all headless Neovim Lua tests.
