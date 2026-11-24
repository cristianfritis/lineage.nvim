## Why

With M0–M3 complete, `lineage.nvim` implements its full v1 analysis engine (TypeScript/TSX/JS, Python, Rust, Go IR validation), hybrid LSP bridge, bounded ripgrep caller discovery, Two-Phase Hover, Inline Focus Mode, and Interactive Tree Panel—but users still must manually compile `lineage-engine` from source, there is no automated cross-platform release pipeline, and user-facing documentation (`README.md`, `:help lineage`) is missing. M4 completes v1 with zero-friction binary installation, verified release packaging, CI hardening, and complete documentation.

## What Changes

- **Prebuilt Binary Installer & Checksum Verification (`lua/lineage/install.lua`, `checksums.json`):**
  - Pin exact release version and per-target SHA-256 digests in `checksums.json` inside the repository.
  - Download prebuilt binaries directly from versioned GitHub Release URLs (zero GitHub REST API calls), verify SHA-256 via platform CLI (`sha256sum`, `shasum -a 256`, PowerShell `Get-FileHash`) with a pure LuaJIT `bit` SHA-256 fallback (never `vim.fn.sha256()`), stop any running sidecar before atomic rename into `stdpath('data')/lineage/bin/lineage-engine-v<semver>`, and fall back to `cargo build --release` when no prebuilt target matches and Cargo + C/C++ toolchain are available.
  - Expose `:Lineage install` (and `require("lineage.install").install()` for `lazy.nvim` `build` hooks) plus surface install status in `:checkhealth lineage`.
- **Cross-Platform Release Pipeline & CI Hardening (`.github/workflows/release.yml`, `.github/workflows/ci.yml`, `engine/Cargo.toml`):**
  - Add `.github/workflows/release.yml` triggered on `v*` tags to build static Linux `musl` (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` with `mimalloc` allocator) and macOS (`aarch64-apple-darwin`, `x86_64-apple-darwin`) release binaries, emit SHA-256 digests, and publish GitHub Release artifacts + `checksums.json`.
  - Harden `.github/workflows/ci.yml` to test Neovim `0.10.x`, `0.11.x`, and `nightly`, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `stylua --check`, and all headless Neovim tests (including installer verification tests).
- **Documentation & Vim Help (`README.md`, `doc/lineage.txt`):**
  - Add `README.md` covering installation (`lazy.nvim` build hook & Cargo fallback), `:Lineage` commands, `<Plug>` mappings, Hover/Focus/Tree keymaps, coverage-honesty badges (`NonNull`, `Nullable`, `Absent`, `Unverified`, `[partial]`), troubleshooting (`:checkhealth lineage`, `:Lineage status`), and explicit v1 scope boundaries.
  - Add `doc/lineage.txt` with standard Vim help tags (`*lineage.txt*`, `*lineage-commands*`, `*lineage-config*`, `*lineage-keymaps*`, `*lineage-health*`).
- **Verification & Formatting Polish:**
  - Add headless Neovim tests `tests/test_install.lua` covering pure-LuaJIT SHA-256 digest computation, `checksums.json` parsing/mismatch rejection, target-triple detection, and atomic install/rollback semantics.
  - Ensure `cargo fmt --check` and `stylua --check plugin lua tests` pass cleanly across the entire repository.

## Capabilities

### New Capabilities
- `binary-installer`: Version-pinned prebuilt binary downloader, SHA-256 verifier (platform CLI + pure LuaJIT `bit` SHA-256), sidecar-safe atomic binary swap into `stdpath('data')/lineage/bin/`, and `cargo build --release` fallback.
- `release-pipeline-and-docs`: Automated cross-platform GitHub Release workflow (Linux `musl` with `mimalloc` + macOS universal targets), hardened Neovim `0.10`/`0.11`/`nightly` CI matrix, `README.md`, and Vim help (`doc/lineage.txt`).

### Modified Capabilities
- `observability-and-cli`: Extend `:checkhealth lineage` to report binary install source/version and recommend `:Lineage install` when the sidecar binary is missing.

## Impact

- **Neovim Lua Modules:** Adds `lua/lineage/install.lua`; updates `lua/lineage/init.lua`, `lua/lineage/health.lua`, `lua/lineage/config.lua`, and `plugin/lineage.lua` (adds `:Lineage install`).
- **Engine Configuration:** Adds optional `mimalloc` global allocator feature/dependency for `musl` targets in `engine/Cargo.toml` and `engine/src/main.rs`.
- **Packaging & CI:** Adds `checksums.json`, `.github/workflows/release.yml`; updates `.github/workflows/ci.yml`.
- **Docs & Tests:** Adds `README.md`, `doc/lineage.txt`, and `tests/test_install.lua`.
