## What this proposes

This change introduces `lineage.nvim`, a greenfield Neovim 0.10+ plugin backed by a standalone Rust sidecar binary (`lineage-engine`), to provide demand-driven, flow-sensitive, null- and data-aware interprocedural slicing without eager whole-repository indexing. `lineage-engine` parses files lazily at two granularities (a file skeleton plus function-scoped Pruned SSI IR with bounded relational guards and a 2-axis product absence lattice that keeps `Unknown` separate from `Absent`), caps memory at `clamp(5% host RAM, 128 MB, 300 MB)` via a 3-tier byte-weighted LRU cache and an 85% OS RSS safeguard, and communicates with Neovim over full-duplex NDJSON stdio with separate Hover and Background worker lanes. It ships Tier-1 language adapters for TypeScript/TSX/JS, Python, and Rust (plus a Go `(val, err)` core IR test suite), a location-only batched LSP bridge with a bounded Rust ripgrep fallback, and three Neovim UI modes (Two-Phase Hover with Line-1 coverage honesty, Inline Focus Mode, and Interactive Tree Panel). Delivery is structured across five vertical milestones (`M0`–`M4`), gating `M2` on an `M1` TypeScript measurement of external generic `Unknown` rates (`<= 25%`), and includes version-keyed checksum-verified binary distribution and observability commands.

## How the existing system works, per this document

There is no existing `lineage.nvim` or `lineage-engine` codebase; this is a greenfield repository with no prior specs under `openspec/specs/`. Currently, Neovim users rely on built-in LSP workflows (`definition`, `references`, `callHierarchy`, `hover`), which locate symbols and show static types at a point but do not trace which specific assignments, branches, guards, or cross-file returns can produce an absent/null value at the cursor. Empirical benchmarks from a sibling plugin (`refactor-neo.nvim`, `ADR-0005` and `ADR-0006`) showed that workspace-wide scanning and Tree-sitter parsing in Neovim Lua causes 7.0–8.6s wall time and 60–287ms main-loop stalls on TypeScript, and that LSP `prepareCallHierarchy` diverges across servers (for example, `vtsls` omits `item.data` on nested items and rejects `prepareCallHierarchy` on unopened files).

---

### Findings

1. **Severity**: Major
   **Affected Scope**: Hybrid LSP Bridge / Reverse-RPC flow (`design.md` L33–38; `hybrid-lsp-protocol/spec.md` L4, L12, L22; `tasks.md` L5–6, L10)
   **Discrepancy Description & Rationale**: The end-to-end Phase 2 control flow between `lineage-engine` and `lineage.nvim` is never explicitly described in `proposal.md`, `design.md`, or `hybrid-lsp-protocol/spec.md`. A naive reader only learns that `lineage-engine` calls back into Neovim to query attached LSP servers by piecing together hints (`"c-<n>"` vs `"e-<n>"` IDs, `client:cancel_request` in Lua, and `LspResolveBatchParams` / `reverse-RPC table` in `tasks.md`). State explicitly who queries LSP, what `"c-<n>"` and `"e-<n>"` represent, and how `lspResolveBatch` flows over stdio.

2. **Severity**: Major
   **Affected Scope**: Hover default keymaps `gn`, `gp`, `<CR>` (`design.md` L43; `neovim-ui/spec.md` L4; `tasks.md` L20)
   **Discrepancy Description & Rationale**: While `g1`–`g9`, `gt`, `gf`, `gs`, `gu`, and `q`/`<Esc>` are explained in `neovim-ui/spec.md`, the actions for `gn`, `gp`, and `<CR>` inside the hover float are never defined anywhere in the bundle.

3. **Severity**: Major
   **Affected Scope**: Phase 2 hover item numbering and window height (`neovim-ui/spec.md` L11–12 vs `tasks.md` L20)
   **Discrepancy Description & Rationale**: `neovim-ui/spec.md` states Phase 1 renders `[1]..[k]` (capped at 5 when open cross-file boundaries exist) and Phase 2 appends at `[k+1]..[9]`, whereas `tasks.md` L20 specifies "fixed-height Line-1 coverage honesty qualifiers, partitioned Phase 1 (`[1]..[5]`) + Phase 2 (`[6]..[9]`) append-only updates". It is unclear whether Phase 2 starts at `k+1` or always at `6`, and how "fixed-height" reconciles with appending Phase 2 items (e.g., whether vertical height is pre-reserved or grows downward without flipping anchor).

4. **Severity**: Major
   **Affected Scope**: 3-tier cache internals (`OverlayCache`, `SliceArena`, "strictly file-local parametric `FnSummary`") (`design.md` L28; `core-ir-and-slicer/spec.md` L37)
   **Discrepancy Description & Rationale**: These project-specific cache structures are named but not explained. The document does not state what `FnSummary` is parameterized over, what "strictly file-local" excludes (e.g., no baked-in cross-file callee results), what data `OverlayCache` caches and how staleness/revalidation works, or what `SliceArena` holds and its lifetime.

5. **Severity**: Major
   **Affected Scope**: Meta-node taxonomy (`ClusterNode`, `FrontierNode`, `ContractedChain`) (`design.md` L40; `core-ir-and-slicer/spec.md` L48; `neovim-ui/spec.md` L40–44)
   **Discrepancy Description & Rationale**: The bundle states that orthogonal caps (`max_depth=5`, `max_nodes=150`, `max_fanout=12`, wall-clock budget) emit `ClusterNode`/`FrontierNode`, and that both support lazy expansion via `expansion_token`, but never defines what distinguishes a `ClusterNode` from a `FrontierNode` or which cap produces each. `ContractedChain` is also referenced in the schema without being explicitly tied to the "zero-cost copy-chain contraction" stage.

6. **Severity**: Major
   **Affected Scope**: "Outer-wrapper nullability rule" and 1-step substitution (`design.md` L37; `language-adapters/spec.md` L34–42)
   **Discrepancy Description & Rationale**: Examples show `T | undefined` and `Option<Self::Item>` yielding `MaybeAbsent` and bare `T` yielding `Unknown`, but the rule itself is not defined: what constitutes an outer wrapper across TS/Python/Rust, how non-nullable concrete wrappers are classified, and what qualifies as "trivially substituted in 1 step".

7. **Severity**: Major
   **Affected Scope**: "Danger sinks" and "intraprocedural-only downstream risks" (`design.md` L12; `core-ir-and-slicer/spec.md` L30; `language-adapters/spec.md` L23; `tasks.md` L27)
   **Discrepancy Description & Rationale**: "Downstream risks" and "danger sinks" are referenced across four files without defining what operations count as danger sinks per language (e.g., member access, calls, indexing, `.unwrap()`/`.expect()`) or how hover's "intraprocedural-only" scope differs from other views.

8. **Severity**: Major
   **Affected Scope**: Kill/gen rules for closures and `async`/`await` (`language-adapters/spec.md` L29–31)
   **Discrepancy Description & Rationale**: The requirement states that "Shallow 1-level `obj.field`, closures, and `async`/`await` follow explicit kill/gen rules", and the `WHEN` clause mentions `await`, but the `THEN` clause only specifies behavior for `log("ok")` vs `mutate(obj)`. The expected behavior for `await` and closures is completely omitted.

9. **Severity**: Major
   **Affected Scope**: "Cold-scan cache isolation" (`design.md` L51; `hybrid-lsp-protocol/spec.md` L40–44; `tasks.md` L32)
   **Discrepancy Description & Rationale**: The ripgrep fallback requires "cold-scan cache isolation" so it leaves the hot LRU working set intact, but neither `core-ir-and-slicer/spec.md` nor `hybrid-lsp-protocol/spec.md` explains the mechanism (e.g., bypassing `FileIrCache` entirely with ephemeral parses vs. a separate bounded cache or admission policy).

10. **Severity**: Major
    **Affected Scope**: `M1` external generic `Unknown` rate gate (`<= 25%`) (`proposal.md` L32; `design.md` L16, L49, L64; `tasks.md` L22)
    **Discrepancy Description & Rationale**: This metric is a hard delivery gate between `M1` and `M2`, but its numerator and denominator are never defined (e.g., fraction of all queried symbols, fraction of all external call sites, or fraction of generic external call sites resolving to `Unknown: external_untyped`).

11. **Severity**: Nit
    **Affected Scope**: `"Data Flow From Here"` naming (`hybrid-lsp-protocol/spec.md` L7, L29, L43; `tasks.md` L33 vs `neovim-ui/spec.md` L40)
    **Discrepancy Description & Rationale**: `"Data Flow From Here"` is used in quotes in the protocol spec and tasks without explicitly stating that it corresponds to `:Lineage from` / `gf` (forward slicing).

12. **Severity**: Nit
    **Affected Scope**: Prohibition on `vim.fn.sha256()` (`design.md` L45; `packaging-and-observability/spec.md` L4; `tasks.md` L39)
    **Discrepancy Description & Rationale**: The bundle repeatedly forbids `vim.fn.sha256()` without a one-line rationale (e.g., it hashes Vimscript strings rather than raw binary files / truncates at NUL bytes), leaving future maintainers without the "why".

13. **Severity**: Nit
    **Affected Scope**: `"Phase-0 observability"` (`proposal.md` L13)
    **Discrepancy Description & Rationale**: `"Phase-0"` appears once in `proposal.md` and is undefined, creating minor terminology collision with milestones (`M0`–`M4`) and hover execution phases (`Phase 1` / `Phase 2`).
