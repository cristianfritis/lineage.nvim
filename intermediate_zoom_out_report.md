# Stage 8: Intermediate Zoom Out Report (`openspec/changes/lineage-v1-foundation/`)

Reviewer: `egm-zoom-out`  
Standard: `zoom-out-standard.md`  
Focus: Auditing additions and mitigations surfaced in `comprehension_report.md` and `critic_report.md` against scope creep and premature machinery.

---

## Findings

1. **[Major] Keep Security Sandboxing Minimal (Avoid Breaking `pnpm`/`Nix`/Symlinked Workspaces)**
   - **Affected Scope:** `critic_report.md` #2; `design.md` (L47–53); `specs/language-adapters/spec.md` (L44–49); `specs/packaging-and-observability/spec.md` (L21–27)
   - **Rationale:** `lineage-engine` is a local editor sidecar running with the user's own OS permissions. A strict symlink-escape allowlist will break `pnpm` stores, Nix `/nix/store`, Bazel, and symlinked monorepos, and a log/dump redaction parser is unnecessary overhead.
   - **Minimal Fix:** Reject non-regular files (`!meta.is_file()`) and pseudo-filesystems (`/proc`, `/sys`, device nodes); guard only static manifest relative joins (`tsconfig.json` `paths`/`baseUrl`) against `..` traversal outside the workspace root while trusting URIs returned by the user's attached LSP; keep rotating logs source-free (paths, symbol names, timings, state tags only).

2. **[Major] Avoid Piling `mi_collect(true)` + Hysteresis Onto Premature RSS Polling in M0–M1**
   - **Affected Scope:** `critic_report.md` #5; `specs/core-ir-and-slicer/spec.md` (L36–45); `design.md` (L13); `tasks.md` (Task 1.7)
   - **Rationale:** Adding `mi_collect(true)` FFI calls and a hysteresis/cooldown state machine piles more allocator-specific complexity onto an 85% RSS trigger before real-repo benchmarks prove it is needed.
   - **Minimal Fix:** Defer OS RSS polling, `ts_set_allocator`, `mi_collect(true)`, and hysteresis out of M0–M1. Rely on `FileIrCache`'s byte-weighted LRU cap, bound the dirty-buffer map (e.g., max 32 buffers / 16 MB LRU), and keep the `1 MB` / `4096`-byte pre-parse guards. Add allocator tuning only if M1 benchmarks exceed the cap.

3. **[Major] Replace `expansion_token` Session/Continuation Machinery with Warm-Cache Re-Slicing in M3**
   - **Affected Scope:** `critic_report.md` #3, #9; `specs/neovim-ui/spec.md` (L40–44); `specs/hybrid-lsp-protocol/spec.md` (L14–19)
   - **Rationale:** Neither a TTL server session store nor an opaque continuation token serializing budgets and ancestor-cycle sets is necessary when Tier-1 `FileIrCache` is already warm.
   - **Minimal Fix:** Drop `expansion_token` and expansion RPCs from M0. In M3, expanding a truncated boundary node in the Tree Panel issues a normal bounded `lineage/slice` request rooted at that node's `(uri, range, direction)` and splices/deduplicates the returned nodes in Lua.

4. **[Major] Cap Static Manifest Resolution (Avoid Rebuilding `tsc`/`cargo`)**
   - **Affected Scope:** `critic_report.md` #6; `specs/language-adapters/spec.md` (L44–49); `tasks.md` (Task 2.2)
   - **Rationale:** Full `tsconfig.json` `"extends"` chains + `"references"` graphs reinvent `tsc` when Phase 2 LSP `definition` already resolves complex project graphs.
   - **Minimal Fix:** Key cached manifests by the nearest-ancestor manifest directory (walking up from the queried file), support basic `paths`/`baseUrl` (plus at most single-level relative `"extends"`), and let Phase 2 LSP handle `"references"` graphs. Keep disk `stat()` `(mtime, len)` checks and synthetic `<module>` top-level lowering.

5. **[Major] Keep Comprehension/Critic Clarifications Concise Without Cementing Premature Machinery**
   - **Affected Scope:** `comprehension_report.md` #4–6, #8–9; `critic_report.md` #9; `specs/core-ir-and-slicer/spec.md`; `specs/language-adapters/spec.md`; `specs/hybrid-lsp-protocol/spec.md`
   - **Minimal Fixes:**
     - Drop Tier-2 `OverlayCache` from M0/M1 (LSP servers already cache their indexes; add only if M1 benchmarks show Phase 2 needs it); define `SliceArena` in one sentence in `design.md`.
     - Collapse `ClusterNode` and `FrontierNode` into one `FrontierNode { reason: depth_cap | fanout_cap | node_cap | timeout }` deferred to M3.
     - Drop 1-step generic type substitution; inspect only the outermost nullable wrapper (`T | null`, `T | undefined`, `Option<T>`, `Optional[T]`), classify bare `T` as `Unknown: external_untyped`, and let the M1 `<= 25%` gate decide whether substitution is needed in M2.
     - Keep `hybrid-lsp-protocol/spec.md` behavioral; keep concrete JSON field schemas in `design.md` for active milestones only.
     - Keep closures/`await` and cold-scan isolation to one conservative sentence each (any `await`, aliasing call, or closure boundary resets `obj.field` to `Unknown`; ripgrep fallback parses candidates ephemerally without inserting into `FileIrCache`).

6. **[Major] Scope `lineage-v1-foundation` to M0 + M1 (Before the Empirical M1 Gate)**
   - **Affected Scope:** `proposal.md`, `design.md`, `tasks.md`
   - **Rationale:** Nearly half the comprehension and critic findings concern M2–M4 features sitting behind the unresolved M1 `<= 25%` empirical gate (reinforcing `zoom_out_report.md` #1). Split `lineage-v1-foundation` to M0 + M1 and defer M2–M4 to follow-on OpenSpec changes once M1 passes.

7. **[Nit] Keep Valid Fixes Minimal**
   - **Alternatives Considered (`critic_report.md` #1):** Add a concise 4–5 bullet summary in `design.md` rather than duplicating all 9 entries from `docs/designs/lineage-engine-v1.md`.
   - **Crash Recovery (`critic_report.md` #4):** Max-3-restarts cap, reset `b:changedtick` map + reject pending callbacks on exit, and a 2-line Lua `> 1 MB` pre-check before `lineage/syncBuffer`.
   - **Multi-Client LSP (`critic_report.md` #7):** Filter attached clients by `supports_method` and query the first non-circuit-broken client; deduplicate Phase 1/Phase 2 origins by `(file_id, range)`.
   - **Hover Keymaps & Layout (`comprehension_report.md` #2, #3):** Define `<CR>` (jump under cursor), drop `gn`/`gp` from M1 Hover if unused (`:Lineage pin` belongs to M3), anchor float once on open (`NW`/`SW`), and append Phase 2 at `[k+1]..[9]` without pre-reserving blank lines.
