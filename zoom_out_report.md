# Stage 5: Initial Zoom Out Report (`openspec/changes/lineage-v1-foundation/`)

Reviewers: `egm-zoom-out` + `egm-inventive-catalyst`  
Standard: `zoom-out-standard.md`  
Context: `docs/designs/lineage-engine-v1.md`, `concepts_brief.md`

---

## Part 1: Skeptical Architect (`egm-zoom-out`)

### Context Sufficiency & Root Problem Check
Passed. `proposal.md` and `design.md` establish the objective, root problem, empirical rationale, and non-goals.

### Findings

1. **[Major] Monolithic 5-Milestone Change Bundle Spans an Unresolved `M1` Architectural Gate**
   - **Affected Scope:** `proposal.md` (L15–22, L32), `design.md` (L16, L49, L53, L64), `tasks.md` (L1–42)
   - **Rationale:** `design.md` defines an `M1` empirical measurement gate (`<= 25%` external generic `Unknown` rate on TypeScript repos) that determines whether `M2` must change the LSP bridge strategy. Bundling all five milestones (`M0`–`M4`), all three languages, all three UI modes, ripgrep fallback, and release packaging into one OpenSpec change undercuts that vertical-slice risk mitigation. Scope this change to `M0` + `M1` (core engine + TypeScript Hover + measurement gate) and defer `M2`–`M4` to follow-on OpenSpec changes once `M1` validates the assumptions.

2. **[Major] Premature Optimization & Front-Loaded Infrastructure in Milestone `M0`**
   - **Affected Scope:** `tasks.md` (Tasks 1.1, 1.3, 1.4, 1.7, 1.8, 1.9), `design.md` (Decisions 1, 3), `specs/core-ir-and-slicer/spec.md` (L36–46)
   - **Rationale:** `M0` front-loads Tier-2 symbol-anchored stale-while-revalidate `OverlayCache`, OS RSS polling + `ts_set_allocator`, dual priority lanes (even though background scans do not exist until `M3`), `ignore`/`grep-searcher`/`grep-regex`/`memchr` crates in `Cargo.toml`, and `focus_spans`/`tree_rows` projectors (which are `M3` deliverables). Keep `M0` minimal: single-lane worker + Tier-1 bounded LRU + `hover_card` projection + simple per-request LSP timeouts; add `M3` crates/lanes/projections when needed.

3. **[Major] YAGNI Abstractions for Post-v1 Non-Goals (`go.mod`, `OutParam`, `Exceptional` Edges)**
   - **Affected Scope:** `proposal.md` (L10), `design.md` (L22–23, L31, L37), `specs/core-ir-and-slicer/spec.md` (L15)
   - **Rationale:**
     - `go.mod` resolver appears in `proposal.md`/`design.md` even though Go is a post-v1 Non-Goal.
     - `OutParam` call output slots are for C/C# (post-v1); none of TS, Python, Rust, or Go have out-parameters. Plain multi-output `outputs: Vec<ValueId>` covers single returns and Go `(val, err)`.
     - `Exceptional` call exit edges contradict the Non-Goal excluding interprocedural uncaught exceptions; intraprocedural `throw`/`raise` is already handled by `EarlyExit` / `Unreachable`.

4. **[Major] Reinventing the Wheel: Pure LuaJIT `bit` SHA-256 Implementation**
   - **Affected Scope:** `design.md` (Decision 7, L45), `specs/packaging-and-observability/spec.md` (L4), `tasks.md` (Task 5.1, L39)
   - **Rationale:** Every supported OS ships a native SHA-256 CLI (`sha256sum`, `shasum -a 256`, PowerShell `Get-FileHash`). Writing a bespoke pure-LuaJIT `bit` SHA-256 adds unnecessary code and 32-bit signed-integer bug surface; fail with an actionable error or fall back to `cargo build --release` if no native CLI exists.

5. **[Major] Altitude Drops (Implementation Weeds) in Capability Specs and Proposal**
   - **Affected Scope:** `proposal.md` (L7–13), `specs/core-ir-and-slicer/spec.md` (L26, L37), `specs/hybrid-lsp-protocol/spec.md` (L33, L40), `specs/packaging-and-observability/spec.md` (L4)
   - **Rationale:** Capability specs hardcode Rust field signatures, internal cache names, allocator crates (`mimalloc`/`ts_set_allocator`), `line-index`, Lua filenames (`lua/lineage/lsp_compat.lua`), Rust trait names, and thread formulas. Move implementation choices to `design.md`/`tasks.md` and keep `specs/*/spec.md` and `proposal.md` at behavioral/capability altitude.

6. **[Nit] Redundant Full `graph` Payload on Sub-50ms Hover Responses**
   - **Affected Scope:** `specs/hybrid-lsp-protocol/spec.md` (L15), `design.md` (Decision 5, L39–41)
   - **Rationale:** Always serializing and decoding the full canonical `graph: { nodes, edges }` alongside `hover_card` adds unnecessary wire payload and Lua `vim.json.decode` main-loop overhead. Make `graph` opt-in for CLI `--format json`, `:Lineage debug-dump`, and future graph consumers.

---

## Part 2: Inventive Catalyst (`egm-inventive-catalyst`)

1. **[Major] Query-Scoped AST Retention vs. Per-Function AST Disposal**
   - **Affected Scope:** `specs/core-ir-and-slicer/spec.md` (L3–12), `design.md` (L13)
   - **False Compromise:** Dropping the Tree-sitter `Tree` after each single function lowering forces repeated full-file re-parses when one query hops across `fn_a -> fn_b -> fn_c` inside the same 3,000-line file.
   - **Resolution:** Hold up to 2–4 parsed Tree-sitter `Tree` handles inside the ephemeral Tier-3 `SliceArena` and drop them unconditionally when the query finishes (zero ASTs persist in Tier-1 RAM between queries).

2. **[Major] Ephemeral Tier-3 `SliceArena` vs. Stateful Lazy `FrontierNode` Expansion**
   - **Affected Scope:** `specs/neovim-ui/spec.md` (L39–44), `specs/core-ir-and-slicer/spec.md` (L36–38), `tasks.md` (L9–11)
   - **False Compromise:** Dropping Tier-3 `SliceArena` on query completion conflicts with lazy `FrontierNode`/`ClusterNode` expansion "without recomputing the entire graph from scratch."
   - **Resolution:** Make `expansion_token` a stateless, client-held continuation token encoding `(uri, symbol_id, range)`, direction, remaining budget, and compact ancestor-cycle hash set, echoed back to run a bounded sub-slice against warm Tier-1/Tier-2 caches and returning an idempotent `GraphDelta`.

3. **[Major] Unbounded Static Wildcard Barrel Traversal vs. `< 50ms` Phase 1 Latency**
   - **Affected Scope:** `specs/language-adapters/spec.md` (L44–49), `design.md` (L36–38)
   - **False Compromise:** Static `index.ts` barrel resolution has no cap; resolving through wildcard mega-barrels (`export * from ...`) can blow `< 50ms` Phase 1 and churn Tier-1 LRU.
   - **Resolution:** Resolve explicit named re-exports via `FileSkeleton` and cap wildcard `export *` fan-out in Phase 1 (e.g., max 2 barrel hops, max 10 siblings, or 10ms sub-budget), deferring overflow to Phase 2 LSP `definition` or `Unknown: barrel_fanout_cap`.

4. **[Nit] Tier-2 `OverlayCache` Line-Range Diffing vs. Reusing Tier-1 Function Content Hashes**
   - **Affected Scope:** `design.md` (L27–29), `specs/core-ir-and-slicer/spec.md` (L10–12), `specs/hybrid-lsp-protocol/spec.md` (L32–37)
   - **False Compromise:** Tracking edited line ranges and shifting cached positions duplicates invalidation machinery already solved by Tier-1 `function_content_hash`.
   - **Resolution:** Stamp Tier-2 `OverlayCache` entries with the enclosing function's `function_content_hash` (caller side) and target `stat()`/`changedtick` (callee side), dropping line-range diffing and coordinate shifting.
