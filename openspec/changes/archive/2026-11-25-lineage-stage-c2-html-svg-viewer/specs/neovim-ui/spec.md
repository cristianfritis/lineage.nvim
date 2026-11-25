## ADDED Requirements

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
