## Context

M0+M1 established the Pruned SSI Core IR, NDJSON stdio transport, TypeScript adapter, and Interactive Two-Phase Hover; M2 added Python and Rust adapters plus Go `(val, err)` Core IR validation. All three Tier-1 languages now slice in `< 20ms` (p95) with `< 30 MB` peak RSS.

M3 completes the v1 interactive workflows specified in `docs/designs/lineage-engine-v1.md`:
1. **Inline Focus Mode & Cross-Buffer Pinning** (`lua/lineage/ui/focus.lua`)
2. **Interactive Tree Panel** (`lua/lineage/ui/tree.lua`)
3. **2-Step LSP Call Hierarchy + `references` Fallback** (`lua/lineage/lsp_compat.lua`)
4. **Bounded Parallel Rust Ripgrep + Tree-sitter Standalone Caller Discovery** (`engine/src/resolver/ripgrep.rs`)

## Goals / Non-Goals

**Goals:**
- Compute `focus_spans` and `tree_rows` purely in Rust (`engine/src/slicer/projectors.rs`) over the bounded `SliceEnvelope` graph so Lua remains a thin renderer and CLI/Neovim outputs match.
- Keep `SliceEnvelope` strictly additive under `schema_version: 1` (`focus_spans` and `tree_rows` optional with `#[serde(default)]`).
- Run standalone "From Here" caller discovery in Rust via `ignore::WalkParallel` + boundary-checked `memchr::memmem` + Tree-sitter call-site verification under hard quotas (`min(4, cpus/2)` threads, `<= 5,000` files walked, `<= 300` files parsed, `<= 2s` deadline) without evicting Tier 1 hot cache entries.
- Support 2-step `textDocument/prepareCallHierarchy` -> `callHierarchy/incomingCalls` in `lsp_compat.lua` with automatic fallback to `textDocument/references` (and standalone ripgrep when unattached or truncated).
- Never block Neovim's main loop on edits during Focus Mode: mark `[stale]` immediately on `TextChanged` and refresh on debounced idle or explicit refresh.

**Non-Goals:**
- No Sugiyama 2D ASCII/Unicode DAG renderer, DOT/Mermaid/HTML+SVG export, or Kitty/Ghostty image preview (deferred to v1.1/v1.2).
- No cross-platform prebuilt binary downloader (`install.lua`, `checksums.json`, release CI — deferred to M4).
- No synthetic LSP `didOpen`/`didClose` or background `bufload`.

## Decisions

1. **Pure Rust Projections (`FocusSpan`, `TreeRow`) + `lineage/expand` RPC:**
   - Rust computes `focus_spans` (line ranges + semantic role + text badge `[NULL]`/`[OK]`/`[?]`) and `tree_rows` (indented rows with `node_id`, `depth`, `file_id`, `range`, `label`, `badge`, `is_cycle`, `is_frontier`, `expansion_token`, `is_absent`, `group_key`) so Lua does zero graph traversal or sorting.
   - Lazy expansion of `FrontierNode`/`ClusterNode` sends `lineage/expand { expansion_token, uri, position, direction, exclude_tests }` and splices returned child `tree_rows` into the split.
   - *Alternative rejected:* Computing tree grouping, cycle detection, and frontier slicing in Lua — rejected to preserve the strict Rust/Lua boundary and CLI parity.

2. **Cold-Scan Cache Isolation in `ripgrep.rs`:**
   - Candidate files scanned during standalone "From Here" caller discovery are parsed with a local `tree_sitter::Parser` on the worker thread and NOT inserted into `FileIrCache` (Tier 1 LRU), preventing a 300-file scan from evicting the user's hot working set.
   - TS/JS files additionally apply a quoted-specifier prefilter when searching for an exported symbol from a known module stem, skipping files that neither import nor mention the identifier.

3. **2-Step Call Hierarchy with `vtsls` Unopened-File Fallback:**
   - `lsp_compat.lua` attempts `textDocument/prepareCallHierarchy` -> `callHierarchy/incomingCalls` when supported by the attached client. If `prepareCallHierarchy` is unsupported, returns `null`/empty, or fails on unopened files (per `vtsls` behavior), it falls back transparently to `textDocument/references` before delegating to Rust's bounded ripgrep scan.

4. **Theme-Derived Extmarks & Stale-on-Edit Lifecycle in Focus Mode:**
   - Unrelated lines in the focused buffer are dimmed via extmarks linked to `Comment` (`LineageFocusDim`), while relevant lines get semantic highlight groups (`LineageStateAbsent`, `LineageStatePresent`, `LineageStateUnknown`) paired with ASCII/Unicode virtual-text badges.
   - `TextChanged`/`TextChangedI` immediately flips the mode indicator to `[stale]` without re-querying on every keystroke; a debounced `400ms` timer (or manual `:Lineage focus`) triggers re-slicing.

## Risks / Trade-offs

- **[Risk] Large monorepo ripgrep scan exceeding 2s or memory ceiling** → Mitigation: Hard `AtomicUsize` counters for walked files (`<= 5,000`) and parsed files (`<= 300`), plus `Instant` deadline (`2s`) returning `WalkState::Quit`, emitting `TruncationReason::WallClockBudget` / `MaxNodes` so the Tree Panel displays `[partial]`.
- **[Risk] Extmark leak when switching buffers or closing splits** → Mitigation: Dedicated `vim.api.nvim_create_namespace("lineage_focus")` cleared cleanly on `:Lineage clear`, `BufWipeout`, and before every re-render.
- **[Risk] Synchronized Tree preview window stealing focus or failing on deleted buffers** → Mitigation: Preview float opens with `enter = false`, `focusable = false`, and guards every update with `nvim_win_is_valid` / `nvim_buf_is_valid`.
