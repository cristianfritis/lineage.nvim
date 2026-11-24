## Context

Milestones M0–M3 delivered the complete v1 engine and Neovim UI (`lineage-engine` with TypeScript/TSX/JS, Python, Rust adapters, Go `(val, err)` IR validation, NDJSON RPC transport, hybrid LSP bridge with 2-step call hierarchy, bounded ripgrep caller discovery, Two-Phase Hover, Inline Focus Mode, and Interactive Tree Panel).

Milestone M4 finishes v1 by making installation zero-friction, verifiable, and repeatable across Linux and macOS, hardening CI across Neovim versions, and adding user-facing documentation (`README.md` and `doc/lineage.txt`).

## Goals / Non-Goals

**Goals:**
- Provide a self-contained binary installer (`lua/lineage/install.lua`) pinned to `checksums.json` inside the repo that downloads prebuilt release archives/binaries from versioned GitHub Release URLs without touching the GitHub REST API.
- Verify every downloaded binary with SHA-256 using platform CLI tools (`sha256sum`, `shasum -a 256`, PowerShell `Get-FileHash`) and a pure LuaJIT `bit` SHA-256 implementation as fallback (never `vim.fn.sha256()`).
- Ensure safe upgrades on active editors: stop any running sidecar before atomically renaming the verified temp file into `stdpath('data')/lineage/bin/lineage-engine-v<semver>` and applying executable permissions (`chmod 0755`).
- Automatically fall back to `cargo build --release` when no prebuilt target matches (e.g. Windows or unsupported arches) and `cargo` + C/C++ compiler exist.
- Build static Linux `musl` (`x86_64`, `aarch64` using `mimalloc` for musl allocation performance) and macOS (`arm64`, `x86_64`) binaries via `.github/workflows/release.yml`, generating `checksums.json`.
- Ship concise, complete documentation in `README.md` and Vim help (`doc/lineage.txt`).

**Non-Goals:**
- No Mason registry submission in v1 (deferred to v1.2).
- No Windows `x86_64-pc-windows-msvc` prebuilt binary in M4 release CI unless added post-v1; Windows uses the `cargo build --release` fallback path cleanly.
- No auto-updating or background network traffic on plugin load — downloads occur only on explicit `:Lineage install` or package manager `build` hook.

## Decisions

1. **Direct Versioned GitHub Release URLs + Repo-Pinned `checksums.json`:**
   - `checksums.json` stores `{ "version": "0.1.0", "repo": "lineage-nvim/lineage.nvim", "targets": { "<triple>": { "asset": "...", "sha256": "..." } } }`.
   - `install.lua` constructs `https://github.com/<repo>/releases/download/v<version>/<asset>` directly, avoiding GitHub API rate limits and unauthenticated 60 req/hr failures in CI/corporate networks.
   - *Alternative rejected:* Querying GitHub Releases REST API (`/releases/latest`) — rejected due to unauthenticated rate limits and non-deterministic version skew between plugin Lua code and engine binary.

2. **SHA-256 Verification Without `vim.fn.sha256()`:**
   - `vim.fn.sha256()` operates on Vim strings (truncating at NUL bytes in binaries), making it unsafe for executable verification.
   - `install.lua` first checks platform CLI utilities (`sha256sum`, `shasum -a 256`, `powershell Get-FileHash`), and falls back to an embedded pure-LuaJIT `bit` SHA-256 streaming implementation reading raw bytes via `io.open(path, "rb")`.
   - *Alternative rejected:* Requiring external `sha256sum` unconditionally — rejected to guarantee verification works on minimal containers and Windows.

3. **`mimalloc` Global Allocator on `musl` Targets:**
   - Static Linux `musl` builds suffer significant allocator contention under parallel Tree-sitter/ripgrep workloads with musl's default `mallocng`.
   - Gate `mimalloc` behind `#[cfg(all(target_env = "musl", feature = "mimalloc"))]` (or optional dependency enabled in `release.yml` for musl targets) so local dev builds on glibc/macOS keep minimal compile overhead while musl release binaries get fast multi-threaded allocation.

4. **Atomic Swap + Sidecar Stop Semantics:**
   - Downloads write to `<dest>.tmp.<pid>`. Only after SHA-256 matches does `install.lua` call `require("lineage.client").stop()`, `chmod 0755` (on Unix), and `vim.uv.fs_rename(tmp, dest)` (removing `<dest>.tmp.<pid>` on any failure).
   - Version-keyed filenames (`lineage-engine-v<semver>`) prevent in-place overwrite conflicts across plugin upgrades.

## Risks / Trade-offs

- **[Risk] `checksums.json` contains placeholder hashes before the first tagged `v0.1.0` GitHub Release is published** → Mitigation: `install.lua` treats missing/placeholder (`"TBD"` / empty) digests for a target as unavailable prebuilt and transparently falls back to `cargo build --release` (with clear messaging) while still failing hard on non-placeholder digest mismatches.
- **[Risk] Pure LuaJIT SHA-256 performance on ~10–15 MB binary** → Mitigation: Prefer native `sha256sum` / `shasum -a 256` whenever available; use the pure LuaJIT implementation only as fallback and test it thoroughly on known NIST test vectors in `tests/test_install.lua`.
- **[Risk] Neovim `nightly` CI flakiness** → Mitigation: Run `0.10.x` and `0.11.x` as required CI checks and mark `nightly` with `continue-on-error: true` so upstream Neovim breakage is surfaced early without blocking releases.
