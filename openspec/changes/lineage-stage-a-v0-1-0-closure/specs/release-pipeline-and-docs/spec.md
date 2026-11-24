## MODIFIED Requirements

### Requirement: Cross-Platform Release CI and Multi-Version Neovim Test Matrix
The repository SHALL include `.github/workflows/release.yml` triggered on `engine-v*` tags and `workflow_dispatch` (`dry_run: boolean`) that validates `engine/Cargo.toml` version parity on tag pushes (skipping tag-prefix parity on `workflow_dispatch` and deriving `version` from `Cargo.toml`), builds static Linux `musl` (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` with `--features mimalloc`) and macOS (`aarch64-apple-darwin`, `x86_64-apple-darwin`) binaries, executes post-build binary smoke checks (`file`, static-musl-safe `(ldd "$BIN" 2>&1 || true)` check, `lipo -archs` on macOS, plus `--version` and a CLI `dump-ir`/`slice` query on native runners), emits deterministic `checksums.json` (with `version`, `engine_tag`, `repo`, and `targets`), and publishes GitHub Release assets with `make_latest: false` (skipped when `dry_run == true`); SHALL provide `scripts/sync_checksums.sh <engine-tag>` to download, validate (4 64-hex digests, matching repo and `engine_tag`), and atomically write repo-root `checksums.json` before the maintainer commits to `main` and tags `v*`; and SHALL harden `.github/workflows/ci.yml` to trigger on `push` (`branches: [main]` and `tags: ["v*"]`) and `pull_request`, run `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo check --features mimalloc`, `stylua --check`, fixture-isolated headless Neovim tests (`tests/test_install.lua` decoupled from live `"TBD"` hashes + `tests/test_two_phase_lsp.lua`), and `checksums.json` validation (rejecting `"TBD"` hashes on `v*` tags).

#### Scenario: Decoupled engine-v* tag builds, smoke-tests, and publishes non-latest release
- **WHEN** an `engine-v0.1.0` tag is pushed to `cristianfritis/lineage.nvim`
- **THEN** `.github/workflows/release.yml` verifies `Cargo.toml == 0.1.0`, builds all 4 target triples, passes static-musl-safe `ldd` and native-runner CLI smoke checks, emits deterministic `checksums.json` with `"engine_tag": "engine-v0.1.0"`, and publishes assets with `make_latest: false`

#### Scenario: Workflow dispatch dry_run verifies all 4 builds without publishing a release
- **WHEN** a maintainer triggers `.github/workflows/release.yml` via `workflow_dispatch` with `dry_run = true` from `main`
- **THEN** tag-prefix parity is skipped, all 4 targets compile and pass smoke checks without creating or mutating a GitHub Release

#### Scenario: Real hashes committed to checksums.json do not break CI installer tests and v* tags reject TBD
- **WHEN** real 64-hex SHA-256 digests are committed to repo-root `checksums.json` on `main` and `.github/workflows/ci.yml` runs on `main` or a `v*` tag
- **THEN** `tests/test_install.lua` passes using temporary fixtures for `"TBD"` fallback tests without attempting live network downloads, and `v*` tag validation runs via `on.push.tags: ["v*"]` confirming zero `"TBD"` entries

### Requirement: Complete User Documentation and Vim Help Tags
The repository SHALL ship `README.md` and `doc/lineage.txt` referencing canonical repo `cristianfritis/lineage.nvim`, synchronized `config.lua` defaults (`binary_path`, `version = nil`, `build_from_source = false`), decoupled `engine-v*` / `v*` installation (`:Lineage install`, `:Lineage install!`, network Cargo fallback + `source_build` provenance), all `:Lineage` subcommands, `<Plug>` mappings, Hover/Focus/Tree keymaps, coverage-honesty indicators (`NonNull`, `Nullable`, `Absent`, `Unverified`, `[partial]`), troubleshooting (`:checkhealth lineage`, `:Lineage status`, `:Lineage log`), and explicit `v0.1.0` / v1 scope boundaries (with `doc/tags` gitignored).

#### Scenario: Vim help file exposes standard help tags and canonical repository slug
- **WHEN** a user runs `:help lineage.txt` or `:help lineage-commands` after `:helptags doc`
- **THEN** `doc/lineage.txt` contains matching `*lineage.txt*`, `*lineage-commands*`, `*lineage-config*`, `*lineage-keymaps*`, and `*lineage-health*` tags and references `cristianfritis/lineage.nvim`
