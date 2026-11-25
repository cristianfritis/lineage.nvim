# lineage.nvim

Demand-driven, flow-sensitive interprocedural nullability and dataflow slicing for Neovim (`>= 0.10`).

`lineage.nvim` pairs a local Rust sidecar (`lineage-engine`) with your attached LSP clients (`vtsls`/`ts_ls`, `pyright`/`ty`, `rust-analyzer`, `gopls`) to answer **"where can this value become null/absent, and where does it flow?"** across TypeScript/TSX/JavaScript, Python, Rust, and Go using Pruned SSI IR, Two-Phase Hover, Inline Focus Mode, and an Interactive Tree Panel.

## Requirements

- Neovim `>= 0.10`
- `curl` or `wget` (for prebuilt binary download)
- **Fallback / source build:** Rust toolchain (`>= 1.82`, `cargo`) and a C/C++ compiler for Tree-sitter grammars

## Installation

### `lazy.nvim` (recommended)

```lua
{
  "cristianfritis/lineage.nvim",
  build = function()
    require("lineage.install").install()
  end,
  config = function()
    require("lineage").setup({
      -- All options below show their defaults:
      binary_path = nil,
      version = nil,
      build_from_source = false,
      hover = {
        enter_on_open = true,
        border = "rounded",
        max_width = 80,
        max_height = 24,
        icons = { unicode = true },
        keymaps = {
          jump_cursor = "<CR>",
          jump_prefix = "g",
          toggle_unverified = "gu",
          close = { "q", "<Esc>" },
        },
      },
      lsp = { timeout_ms = 150 },
      tree = {
        height = 14,
        keymaps = { export = "e" },
      },
      export = {
        default_format = "mermaid",
        lsp_timeout_ms = 500,
        picker = true,
        html = {
          open = true,
          cache_max_files = 20,
        },
      },
      exclude_tests = false,
    })

    vim.keymap.set("n", "<leader>lh", "<Plug>(LineageHover)", { desc = "Lineage Hover" })
    vim.keymap.set("n", "<leader>lf", "<Plug>(LineageFocus)", { desc = "Lineage Focus" })
    vim.keymap.set("n", "<leader>lp", "<Plug>(LineagePin)", { desc = "Lineage Pin" })
    vim.keymap.set("n", "<leader>lt", "<Plug>(LineageTo)", { desc = "Lineage Tree (To)" })
    vim.keymap.set("n", "<leader>lF", "<Plug>(LineageFrom)", { desc = "Lineage Tree (From)" })
    vim.keymap.set("n", "<leader>lb", "<Plug>(LineageBack)", { desc = "Lineage Back" })
    vim.keymap.set("n", "<leader>le", "<Plug>(LineageExport)", { desc = "Lineage Export" })
  end,
}
```

### How Binary Installation Works

- Running `:Lineage install` (or `:Lineage install!` / `require("lineage.install").install({ force = true })`) reads `checksums.json` (`engine_tag`, `repo`, `targets`) pinned inside the plugin repository and downloads the matching prebuilt binary directly from the decoupled `engine-v*` GitHub Release URL (no GitHub REST API calls).
- Every download is SHA-256 verified via `sha256sum`, `shasum -a 256`, or PowerShell `Get-FileHash`, with a binary-safe pure LuaJIT `bit` SHA-256 fallback. Existing release binaries are also SHA-256 verified before returning `already_installed`.
- Before swapping the verified binary into `stdpath('data')/lineage/bin/lineage-engine-<engine_tag>`, any running sidecar is stopped and the file is atomically renamed into place.
- If no prebuilt target or published digest matches, `build_from_source = true` is set, or a prebuilt HTTP/network download fails while `cargo` is on `PATH`, `install.lua` falls back to `cargo build --release --manifest-path <plugin_root>/engine/Cargo.toml` and writes a `<dest>.source_build` provenance marker (removed when a verified prebuilt binary is later installed). SHA-256 mismatches always abort with an error.

### Manual Source Build

```bash
cd engine && cargo build --release
```

## Commands

| Command | Description |
|---|---|
| `:Lineage` / `:Lineage hover` | Open Two-Phase Hover dataflow card at cursor |
| `:Lineage focus` | Toggle Inline Focus Mode for symbol under cursor |
| `:Lineage pin` | Pin Inline Focus Mode across buffer switches |
| `:Lineage clear` | Clear Focus / Pin highlights and autocmds |
| `:Lineage to` | Open Interactive Tree Panel: Data Flow To Here |
| `:Lineage from` | Open Interactive Tree Panel: Data Flow From Here |
| `:Lineage both` | Open Interactive Tree Panel: bidirectional flow |
| `:Lineage back` | Jump back in the Lineage jump stack |
| `:Lineage export <fmt> [path] [flags]` | Export graph (`mermaid` / `dot` / `html`) to clipboard, private browser cache, or file (`!` to overwrite; `--open`/`--no-open`) |
| `:Lineage export clean` | Remove cached HTML exports from `stdpath('cache')/lineage/exports/` |
| `:Lineage install` | Download and verify `lineage-engine` (or build with Cargo) |
| `:Lineage install!` | Force re-install of pinned `lineage-engine` binary |
| `:Lineage status` | Display sidecar version, cache, grammars, and log path |
| `:Lineage log` | Display recent sidecar stderr ring-buffer output |
| `:Lineage debug-dump` | Dump lowered Pruned SSI IR for the current buffer |

## `<Plug>` Mappings

- `<Plug>(LineageHover)`
- `<Plug>(LineageFocus)`
- `<Plug>(LineagePin)`
- `<Plug>(LineageTo)`
- `<Plug>(LineageFrom)`
- `<Plug>(LineageBack)`
- `<Plug>(LineageExport)`
- `<Plug>(LineageExportMermaid)`
- `<Plug>(LineageExportDot)`
- `<Plug>(LineageExportHtml)`

## UI Keymaps

### Two-Phase Hover

| Key | Action |
|---|---|
| `<CR>` | Jump to origin item on cursor line |
| `g1`..`g9` | Jump to numbered origin `[1]`..`[9]` |
| `gu` | Toggle **Unverified Boundaries** section |
| `gs` | Switch to Inline Focus Mode |
| `gp` | Pin Inline Focus Mode |
| `gt` / `gf` / `gn` | Open Tree Panel (`to` / `from` / `both`) |
| `ge` | Export slice graph (format picker or default format) |
| `q` / `<Esc>` | Close hover and restore cursor |

### Inline Focus Mode

- Dims unrelated lines (`LineageFocusDim`) and highlights flow lines with inline badges.
- Automatically marks `[stale]` and debounces re-slicing (`400ms`) on edits.
- Statusline component: `require("lineage.ui.focus").statusline()` (returns e.g. `[Lineage: FOCUS sym]` or `[Lineage: PIN sym [stale]]`).

### Interactive Tree Panel

| Key | Action |
|---|---|
| `<CR>` | Expand frontier node (`[+]`) or jump to location |
| `o` | Expand frontier or toggle fold |
| `e` | Export active Tree slice graph (format picker or default format) |
| `g` | Toggle grouping (`Root Origin` / `Nullability`) |
| `n` | Toggle **Null-only** filter |
| `t` | Toggle **Exclude tests** filter |
| `m` | Cycle slice direction (`to` -> `from` -> `both`) |
| `q` / `<Esc>` | Close tree and floating preview |

## Coverage-Honesty Indicators

`lineage.nvim` explicitly separates proven states from unverified dynamic boundaries:

| Indicator | Meaning |
|---|---|
| `● NonNull` | Value is proven non-null / present on reachable paths |
| `◐ Nullable` | Value may be null/absent along at least one path |
| `○ Absent` (`[NULL]`) | Value is directly null/absent/uninitialized |
| `? Unverified` (`[?]`) | Reached an unresolved dynamic/external boundary |
| `[partial]` | Tree view was capped by configured traversal/expansion budgets |

## Troubleshooting

1. Run `:checkhealth lineage` to inspect Neovim version, `lineage-engine` binary path/source, sidecar ping latency, attached LSP `offset_encoding`, loaded grammars, cache bytes, EWMA/circuit-breaker state, and rotating log path.
2. Run `:Lineage status` or `:Lineage log` inside Neovim.
3. Reproduce slices or export diagrams outside Neovim with the standalone CLI:
   ```bash
   lineage-engine dump-ir path/to/file.ts
   lineage-engine dump-cfg --dot path/to/file.ts
   lineage-engine slice path/to/file.ts:42:10 --format json --trace
   lineage-engine export path/to/file.ts:42:10 --format mermaid --direction both --no-lsp
   lineage-engine export path/to/file.ts:42:10 --format dot --out flow.dot
   lineage-engine export path/to/file.ts:42:10 --format html --direction both --out flow.html
   ```

## v1 Scope Boundaries

- **Supported languages:** TypeScript / TSX / JavaScript, Python, Rust, and Go.
- **Prebuilt binaries:** Linux `musl` (`x86_64`, `aarch64`) and macOS (`arm64`, `x86_64`). Windows and other targets use the automatic `cargo build --release` fallback.
- **No Mason registry entry in v1** and **no background auto-updates** or startup network traffic.
- **Bounded analysis:** Buffers above 1 MB skip live sync; cross-file traversal operates within bounded caller/depth budgets and surfaces unresolved edges as `Unverified` / `[partial]`.
