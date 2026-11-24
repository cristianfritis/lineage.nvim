# Stage A Layer 2 Live LSP Verification Checklist

Run with minimal config:

```bash
nvim -u tests/dogfood_init.lua <repo-file>
```

## Environment & Versions

- [ ] Neovim version recorded (`nvim --version`): `____________________`
- [ ] TypeScript LSP (`vtsls` or `typescript-language-server`) version: `____________________`
- [ ] Python LSP (`pyright-langserver` or `ty`) version: `____________________`
- [ ] Rust LSP (`rust-analyzer`) version: `____________________`

## Indexing Warmup & Telemetry (`G6`)

For each of the 3 languages (TypeScript, Python, Rust):

1. Open a multi-file workspace file and wait for initial LSP indexing to settle.
2. Run `:checkhealth lineage` and verify:
   - `lineage-engine` binary path, `engine_tag`, and provenance (`release` or `source_build`) are reported.
   - Attached LSP client lists `offset_encoding` and capabilities (`definition`, `typeDefinition`, `implementation`, `prepareCallHierarchy`, `references`).
   - Live `rss_bytes` is reported.
3. Execute `>= 20` `:Lineage hover`, `:Lineage focus`, `:Lineage to`, and `:Lineage from` probes across cross-file symbols.
4. Inspect `:Lineage status` and `lua print(vim.inspect(require("lineage.client").get_telemetry()))`:
   - [ ] TypeScript LSP (`>= 20` probes): Phase 2 `p95 <= 250ms`, `0` parse errors, `0` restarts
   - [ ] Python LSP (`>= 20` probes): Phase 2 `p95 <= 250ms`, `0` parse errors, `0` restarts
   - [ ] Rust LSP (`>= 20` probes): Phase 2 `p95 <= 250ms`, `0` parse errors, `0` restarts

## UI & Zero-Stale-Diagnostic Pass Criteria (`G1`)

- [ ] `:Lineage hover` opens Phase 1 immediately (`< 50ms`), updates `status_badge` and appends `[k+1..]` on `lineage/sliceDelta` without renumbering `[1..k]` or dropping `<symbol>: ` prefix.
- [ ] `gu` toggles Unverified Boundaries; `g1`..`g9` / `<CR>` jumps cleanly; `:Lineage back` restores cursor.
- [ ] `:Lineage focus` / `:Lineage pin` renders `[NULL]`, `[?]`, `[OK]` accurately, clears old buffer extmarks when switching buffers, and clears all extmarks on `:Lineage clear`.
- [ ] `:Lineage to` / `:Lineage from` opens Tree Panel, receives Phase 2 callers on `from`, supports `o`, `g`, `n`, `t`, `m`, `q`.
- [ ] `:Lineage log` opens newest daily rotating log alongside `stderr` ring.
- [ ] Rapid cursor movement / closing hover mid-flight cancels Phase 2 cleanly with `0` Lua errors and `0` stale Neovim diagnostics.
