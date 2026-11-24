## MODIFIED Requirements

### Requirement: Version-Pinned Prebuilt Binary Download and SHA-256 Verification
`lineage.nvim` SHALL use `checksums.json` (`version`, `engine_tag`, `repo: "cristianfritis/lineage.nvim"`, and `targets`) as the single source of truth shared by `lua/lineage/install.lua` and `lua/lineage/client.lua`, SHALL construct direct versioned GitHub Release URLs from `engine_tag` (`https://github.com/<repo>/releases/download/<engine_tag>/<asset>`, falling back to `v<version>` only for legacy manifests) without calling the GitHub REST API, SHALL enforce explicit connect/total timeouts on download commands (`curl --connect-timeout 15 --max-time 120`, `wget --timeout=15`), SHALL verify SHA-256 via platform CLI (`sha256sum`, `shasum -a 256`, or PowerShell `Get-FileHash`) or the binary-safe pure LuaJIT `bit` SHA-256 fallback (never `vim.fn.sha256()`), SHALL verify the existing binary's SHA-256 before returning `already_installed` for release installs (accepting an existing binary with `<dest>.source_build` present when `!opts.force`, and removing `<dest>.source_build` when installing a verified release binary), and SHALL support `:Lineage install!` (matching `sub == "install!"` plus `bang = true` alias `:Lineage! install`) to force re-installation.

#### Scenario: Downloaded binary fails SHA-256 verification
- **WHEN** a downloaded binary's computed SHA-256 digest does not match the pinned digest in `checksums.json`
- **THEN** `install.lua` deletes the temporary file, does not touch the destination path, and aborts with a checksum mismatch error without falling back to Cargo

#### Scenario: Stale or corrupted binary at destination does not shadow a pinned release
- **WHEN** a binary already exists at `stdpath('data')/lineage/bin/lineage-engine-<engine_tag>` without `<dest>.source_build`, its SHA-256 does not match `checksums.json`, and `build_from_source` is false
- **THEN** `install.lua` rejects the stale binary instead of returning `already_installed` and downloads and atomically installs the verified release binary

#### Scenario: `:Lineage install!` forces re-installation and clears source_build marker
- **WHEN** the user executes `:Lineage install!` (or `:Lineage! install`) while a valid or `source_build` binary is already present
- **THEN** `plugin/lineage.lua` invokes `lineage.install({ force = true })`, re-installs the pinned binary atomically, and removes `<dest>.source_build` if a verified prebuilt binary was installed

#### Scenario: Pure LuaJIT SHA-256 computes binary-safe digests including NUL bytes
- **WHEN** platform CLI hash utilities are unavailable and `install.lua` hashes a file containing arbitrary binary bytes (including `0x00`)
- **THEN** the pure LuaJIT `bit` SHA-256 implementation returns the exact standard lowercase hex SHA-256 digest

### Requirement: Sidecar-Safe Atomic Binary Swap and Cargo Build Fallback
Before replacing or installing into `stdpath('data')/lineage/bin/lineage-engine-<engine_tag>`, `install.lua` SHALL stop any running `lineage-engine` sidecar and atomically rename the verified temporary file into place; when no matching prebuilt target or published digest is available, `build_from_source = true` is configured, OR a prebuilt HTTP/network download fails (`curl`/`wget` error or timeout) while `cargo` is executable on `PATH`, `install.lua` SHALL fall back to running `cargo build --release --manifest-path <plugin_root>/engine/Cargo.toml`, install atomically, and write an empty `<dest>.source_build` marker file alongside the binary.

#### Scenario: Running sidecar is stopped before atomic install
- **WHEN** `lineage-engine` is running and the user installs a verified binary into `stdpath('data')/lineage/bin/lineage-engine-<engine_tag>`
- **THEN** `install.lua` stops the running sidecar, sets executable permissions, and atomically renames the verified temp file to the `engine_tag`-keyed path

#### Scenario: Network download failure falls back to Cargo and writes source_build marker
- **WHEN** `checksums.json` pins a valid SHA-256 digest, `curl`/`wget` fails due to an offline/network error or timeout, and `cargo` is executable
- **THEN** `install.lua` notifies the user, runs `cargo build --release`, atomically installs the compiled binary, and creates `<dest>.source_build`

#### Scenario: Unsupported platform falls back to Cargo release build
- **WHEN** the current OS/architecture has no published prebuilt entry in `checksums.json` and `cargo` is executable
- **THEN** `install.lua` invokes `cargo build --release --manifest-path <plugin_root>/engine/Cargo.toml`, installs the binary atomically, and creates `<dest>.source_build`
