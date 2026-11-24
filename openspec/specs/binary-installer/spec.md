## Purpose
Provide a zero-friction, version-pinned prebuilt binary installer (`lua/lineage/install.lua`) with SHA-256 checksum verification, sidecar-safe atomic binary swap, and Cargo build fallback.

## Requirements

### Requirement: Version-Pinned Prebuilt Binary Download and SHA-256 Verification
`lineage.nvim` SHALL provide `lua/lineage/install.lua` (exposed via `:Lineage install` and `require("lineage.install").install()`) that reads `checksums.json` from the plugin repository, downloads the matching platform binary directly from the versioned GitHub Release URL (without calling the GitHub REST API), and verifies its SHA-256 digest via platform CLI (`sha256sum`, `shasum -a 256`, or PowerShell `Get-FileHash`) or a pure LuaJIT `bit` SHA-256 fallback (never `vim.fn.sha256()`).

#### Scenario: Downloaded binary fails SHA-256 verification
- **WHEN** a downloaded binary's computed SHA-256 digest does not match the pinned digest in `checksums.json`
- **THEN** `install.lua` deletes the temporary file, does not touch the destination path, and reports a checksum mismatch error

#### Scenario: Pure LuaJIT SHA-256 computes binary-safe digests including NUL bytes
- **WHEN** platform CLI hash utilities are unavailable and `install.lua` hashes a file containing arbitrary binary bytes (including `0x00`)
- **THEN** the pure LuaJIT `bit` SHA-256 implementation returns the exact standard lowercase hex SHA-256 digest

### Requirement: Sidecar-Safe Atomic Binary Swap and Cargo Build Fallback
Before replacing or installing into `stdpath('data')/lineage/bin/lineage-engine-v<semver>`, `install.lua` SHALL stop any running `lineage-engine` sidecar and atomically rename the verified temporary file into place; when no matching prebuilt target or published digest is available, `install.lua` SHALL fall back to running `cargo build --release` if `cargo` is present, or surface a clear installation error otherwise.

#### Scenario: Running sidecar is stopped before atomic install
- **WHEN** `lineage-engine` is running and the user installs a verified binary into `stdpath('data')/lineage/bin/lineage-engine-v<semver>`
- **THEN** `install.lua` stops the running sidecar, sets executable permissions, and atomically renames the verified temp file to the versioned target path

#### Scenario: Unsupported platform falls back to Cargo release build
- **WHEN** the current OS/architecture has no published prebuilt entry in `checksums.json` and `cargo` is executable
- **THEN** `install.lua` invokes `cargo build --release --manifest-path <plugin_root>/engine/Cargo.toml` and copies the compiled binary into the versioned bin directory
