## Purpose
Define the automated cross-platform GitHub Release workflow, hardened multi-version Neovim CI matrix, and user-facing documentation (`README.md` and `doc/lineage.txt`).

## Requirements

### Requirement: Cross-Platform Release CI and Multi-Version Neovim Test Matrix
The repository SHALL include `.github/workflows/release.yml` building static Linux `musl` (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` with `mimalloc`) and macOS (`aarch64-apple-darwin`, `x86_64-apple-darwin`) binaries on version tags and publishing `checksums.json`, and `.github/workflows/ci.yml` running Rust tests, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `stylua --check`, and headless Neovim tests across `0.10.x`, `0.11.x`, and `nightly`.

#### Scenario: Release workflow targets static musl with mimalloc and macOS binaries
- **WHEN** a `v*` release tag is pushed to GitHub
- **THEN** `.github/workflows/release.yml` builds all 4 target triples, computes SHA-256 checksums, and attaches the binaries and `checksums.json` to the GitHub Release

#### Scenario: CI workflow enforces Rust and Lua formatting and tests across Neovim versions
- **WHEN** a pull request or push runs `.github/workflows/ci.yml`
- **THEN** CI runs `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `stylua --check`, and headless Neovim tests on `0.10.x`, `0.11.x`, and `nightly`

### Requirement: Complete User Documentation and Vim Help Tags
The repository SHALL ship `README.md` and `doc/lineage.txt` documenting installation (`lazy.nvim` build hook and Cargo fallback), all `:Lineage` subcommands (`hover`, `focus`, `pin`, `clear`, `to`, `from`, `both`, `back`, `install`, `status`, `log`, `debug-dump`), `<Plug>` mappings, Hover/Focus/Tree keymaps, coverage-honesty indicators (`NonNull`, `Nullable`, `Absent`, `Unverified`, `[partial]`), troubleshooting (`:checkhealth lineage`, `:Lineage status`), and explicit v1 scope boundaries.

#### Scenario: Vim help file exposes standard help tags
- **WHEN** a user runs `:help lineage.txt` or `:help lineage-commands` after `:helptags`
- **THEN** `doc/lineage.txt` contains matching `*lineage.txt*`, `*lineage-commands*`, `*lineage-config*`, `*lineage-keymaps*`, and `*lineage-health*` help tags
