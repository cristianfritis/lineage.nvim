## 1. Prebuilt Binary Installer & Checksum Verification

- [x] 1.1 Create `checksums.json` at the repo root pinning `version`, `repo`, and target triple entries (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `aarch64-apple-darwin`, `x86_64-apple-darwin`).
- [x] 1.2 Implement `lua/lineage/install.lua` with platform/architecture target detection, direct versioned GitHub Release URL download (`curl`/`wget`), SHA-256 verification (platform CLI `sha256sum`/`shasum`/`Get-FileHash` plus binary-safe pure LuaJIT `bit` SHA-256 fallback, never `vim.fn.sha256()`), sidecar stop + `chmod 0755` + atomic `fs_rename` into `stdpath('data')/lineage/bin/lineage-engine-v<semver>`, and `cargo build --release` fallback.
- [x] 1.3 Wire `:Lineage install` in `plugin/lineage.lua` and `lua/lineage/init.lua`, and update `lua/lineage/health.lua` to surface `:Lineage install` guidance when the sidecar binary is missing.

## 2. Release Pipeline, `musl` `mimalloc` & CI Hardening

- [x] 2.1 Add optional `mimalloc` dependency/feature gated for `musl` targets in `engine/Cargo.toml` and `engine/src/main.rs`.
- [x] 2.2 Create `.github/workflows/release.yml` to build static Linux `musl` (`x86_64`, `aarch64` with `--features mimalloc`) and macOS (`arm64`, `x86_64`) binaries on `v*` tags, generate `checksums.json`, and publish GitHub Release assets.
- [x] 2.3 Update `.github/workflows/ci.yml` to include Neovim `nightly` (non-blocking) alongside `0.10.x` and `0.11.x` and verify all Lua and Rust checks.

## 3. Documentation & Vim Help

- [x] 3.1 Create `README.md` covering installation (`lazy.nvim` build hook & Cargo fallback), `:Lineage` commands, `<Plug>` mappings, Hover/Focus/Tree keymaps, coverage-honesty indicators, troubleshooting (`:checkhealth lineage`, `:Lineage status`), and explicit v1 scope boundaries.
- [x] 3.2 Create `doc/lineage.txt` with standard Vim help tags (`*lineage.txt*`, `*lineage-commands*`, `*lineage-config*`, `*lineage-keymaps*`, `*lineage-health*`).

## 4. Verification, Formatting & Tests

- [x] 4.1 Add `tests/test_install.lua` testing pure LuaJIT `bit` SHA-256 (NIST vectors + binary NUL bytes), `checksums.json` validation & mismatch rejection, target detection, and sidecar-safe atomic install semantics.
- [x] 4.2 Run `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release`, `stylua --check` (if installed), and all headless Neovim Lua tests (`tests/test_*.lua`).
