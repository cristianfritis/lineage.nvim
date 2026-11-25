## ADDED Requirements

### Requirement: Interactive Two-Phase Hover with Prefixed Keymaps and Stable Anchoring
`:Lineage hover` (and `<Plug>(LineageHover)`) SHALL open a floating window (`enter_on_open = true`, configurable) that preserves standard Vim counts (`3j`) and motions (`j`, `k`, `/`, `n`, `N`, `f`, `t`), binding user-remappable defaults `<CR>` (jump to item under cursor), `g1`–`g9` (direct numbered jump), `gu` (toggle unverified boundaries), and `q`/`<Esc>` (close and restore cursor), and SHALL never overwrite user `<leader>l*` mappings on plugin load.

#### Scenario: Typing a Vim count inside focused hover does not trigger a jump
- **WHEN** the hover float is focused and the user presses `3j`
- **THEN** the cursor moves down 3 lines inside the hover float instead of jumping to origin `[3]`

#### Scenario: Phase 2 appends without renumbering Phase 1 items, duplicating origins, or flipping anchor
- **WHEN** Phase 1 renders `[1]..[k]` (capped at 5 when open cross-file boundaries exist) anchored once (`NW`/`SW`) and Phase 2 completes within 250ms with cross-file origins
- **THEN** `[1]..[k]` keep their exact indices and line positions, Phase 2 deduplicates against Phase 1 by `(file_id, range)`, appends new origins at `[k+1]..[9]`, and grows without flipping the float across the cursor line

### Requirement: Line-1 Partial-Coverage Qualifiers and Simple-Value Compact Peek
The hover Line-1 verdict strip SHALL qualify coverage whenever any upstream path terminated at `Unknown`, hit a budget/barrel cap, or timed out in Phase 2, SHALL display `Unverified` (never raw `Unknown`) in TypeScript/JavaScript, and SHALL render a compact 1–3 line peek for `Present` variables with zero absence origins, zero danger sinks, and zero unverified boundaries.

#### Scenario: Unverified external boundary prevents an unqualified NonNull badge
- **WHEN** a variable has no proven null origins locally but calls an unverified external library
- **THEN** Line 1 displays a partial qualifier (for example, `user: NonNull (partial — 1 unverified [gu])`) and pressing `gu` expands the unverified boundary with its machine-readable reason

#### Scenario: Simple constant displays compact peek
- **WHEN** the user triggers hover on `const count = 0` with zero origins, zero danger sinks, and zero unverified boundaries
- **THEN** hover renders a compact 1–3 line summary instead of empty analytical sections

### Requirement: Native Jumplist Integration and Lineage Jump Stack
Every direct redirection from Hover SHALL record the origin position in Neovim's native jumplist (`normal! m'`) and push onto the Lineage Jump Stack so both `<C-o>` and `:Lineage back` return to the origin cursor.

#### Scenario: Redirecting to a cross-file null origin and returning
- **WHEN** the user presses `g1` (or `<CR>` on `[1]`) in hover to jump to `auth.ts:42` and then runs `:Lineage back` (or presses `<C-o>`)
- **THEN** the cursor returns to the exact origin file and position

### Requirement: Multi-Arg `:Lineage export` Command, Collision-Free Triggers, and Safe Delivery
`plugin/lineage.lua`, `lua/lineage/commands.lua`, `lua/lineage/export.lua`, `lua/lineage/config.lua`, `lua/lineage/ui/tree.lua`, and `lua/lineage/ui/hover.lua` SHALL upgrade `:Lineage` to `nargs = "*"` (preserving all existing subcommands, `:Lineage install!`, and `:Lineage! install`), SHALL support order-independent flag parsing on `:Lineage export <format> [path] [flags]` (`--direction`, `--exclude-tests`, `--null-only`, `--no-lsp`, `--snippets`, `--no-snippets`, `--out <path>`, rejecting simultaneous `[path]` + `--out`), SHALL expose `<Plug>(LineageExport)`, `<Plug>(LineageExportMermaid)`, `<Plug>(LineageExportDot)` and `config.export = { default_format = "mermaid", lsp_timeout_ms = 500, picker = true }`, SHALL bind `e` in Tree Panel (advertised as `e:export`, zero collision with `nowait` `g`) and `ge` in Hover opening a `vim.ui.select` picker over the intersection of `EngineCapabilities.export_formats` and Lua-supported formats when `export.picker == true`, SHALL resolve export context strictly as `Focused Tree > Focused Hover > Cursor` (Focus/Pin never hijacks cursor export) passing `parent_query_id`/`parent_node_id` on Tree expansion, SHALL check exact format membership inside `client.ensure_started(...)` (plus `-32601` fallback) to fail fast with `:Lineage install!` on older binaries, SHALL copy to clipboard `+` (falling back to `"` with `vim.log.levels.WARN` when `has("clipboard") == 0`) when no path is given, and SHALL refuse to overwrite existing explicit file paths unless `:Lineage export!` is invoked.

#### Scenario: Pressing e in Tree Panel exports the active slice without colliding with g
- **WHEN** a user has the Tree Panel focused with an expanded frontier and presses `e` (or runs `:Lineage export mermaid`)
- **THEN** `g` grouping remains unaffected, `lua/lineage/export.lua` exports the active Tree `query_id` snapshot, writes valid Mermaid to system clipboard `+` (or `"` with `WARN` if clipboard is unavailable), and notifies with node/edge counts and completeness badges

#### Scenario: Explicit file export refuses overwrite without bang and older engine fails fast
- **WHEN** a user runs `:Lineage export dot existing.dot` without `!`, or runs `:Lineage export mermaid` against a pre-Stage-C `lineage-engine` lacking `export_formats`
- **THEN** Neovim refuses to overwrite `existing.dot` and instructs the user to pass `:Lineage export!`, and on an older binary fails fast instructing the user to run `:Lineage install!`

### Requirement: Neovim HTML Export Commands, Flags, Picker, and Mixed-Version Capability Gate
`lineage.nvim` SHALL register `<Plug>(LineageExportHtml)` in `plugin/lineage.lua` (preserving `:Lineage install!` and `:Lineage! install`), SHALL expose `config.export.html = { open = true, cache_max_files = 20 }` and allow `export.default_format = "html"`, SHALL advertise `HTML (browser)` in the `e` / `ge` format picker whenever `"html"` is present in `EngineCapabilities.export_formats`, SHALL validate exact format membership (`vim.tbl_contains(caps.export_formats, format)`) inside `client.ensure_started(...)` (failing fast with `:Lineage install!` guidance when connected to a Stage C.1 binary), SHALL pass `include_snippets = true` only for bare `:Lineage export html` private-cache opens (unless `--no-snippets` is given) while saving explicit `:Lineage export html <path>` / `--out <path>` silently with `include_snippets = false` by default (opt-in `--open` and `--snippets`, refusing overwrite without `:Lineage export!`, rejecting simultaneous `[path]` + `--out` and `--open` + `--no-open`), and SHALL dispatch `:Lineage export clean` before format parsing (with tab-completion and rejecting export flags).

#### Scenario: Explicit file path saves HTML silently without snippets unless --open or --snippets is passed
- **WHEN** a user runs `:Lineage export html docs/auth-flow.html`
- **THEN** `lineage.nvim` sends `include_snippets = nil` (resolving to `false` in engine), refuses overwrite if the file exists without `:Lineage export!`, writes the file silently without calling `vim.ui.open`, and notifies the user with the written path and `snippets OFF`

#### Scenario: Upgrading Lua ahead of lineage-engine fails fast on missing html capability
- **WHEN** a user runs `:Lineage export html` while connected to a Stage C.1 `lineage-engine` advertising `export_formats = ["mermaid", "dot"]`
- **THEN** `lua/lineage/export.lua` detects `"html"` is absent from `capabilities.export_formats` after `ensure_started` and notifies the user to run `:Lineage install!` without hanging or sending an invalid RPC

#### Scenario: Running :Lineage export clean removes only cached HTML exports
- **WHEN** a user runs `:Lineage export clean`
- **THEN** `lua/lineage/commands.lua` dispatches `clean` before format parsing, deletes only regular non-symlink `lineage-*.html` files in `stdpath("cache")/lineage/exports/`, leaves non-matching files and symlinks untouched, and notifies the user with removed file count and bytes reclaimed
