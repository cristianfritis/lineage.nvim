# Stage 7: Critic Review Report (`openspec/changes/lineage-v1-foundation/` `FeatureRange`)

Reviewers: `egm-design` + `egm-structural` + `egm-style`  
Standard: `feature-spec-standard.md` (design & structural)  
Context: `docs/designs/lineage-engine-v1.md`, `concepts_brief.md`

---

## Blockers

1. **[Blocker — `egm-design`, `egm-structural`] Missing Mandatory `## Alternatives Considered` Section in OpenSpec `design.md`**
   - **Affected Scope:** `design.md` (L25–46)
   - **Rationale:** Violates `feature-spec-standard.md` ("The Alternatives Rule"). `design.md` compresses 9 evaluated alternatives from `docs/designs/lineage-engine-v1.md` into 7 one-sided `Decisions` bullets (and Decisions 6–7 omit `- *Rationale:*` entirely). Add an explicit `## Alternatives Considered` section with pros, cons, and technical rejection rationales so `design.md` is self-contained.

2. **[Blocker — `egm-design`, `egm-structural`] Missing Security & Path-Sandboxing Requirements and Risk Mitigations**
   - **Affected Scope:** `design.md` (L47–53), `specs/core-ir-and-slicer/spec.md` (L36–41), `specs/language-adapters/spec.md` (L33–49), `specs/packaging-and-observability/spec.md` (L21–27)
   - **Rationale:** Violates `feature-spec-standard.md` ("Risks & Limits"). `lineage-engine` reads disk paths from `tsconfig.json` `paths`/`baseUrl`, relative imports, and LSP URIs; malicious `../../../../etc/passwd` or symlinks to `~/.ssh/id_rsa` or `/proc/self/environ` pass `metadata().is_file()`. Add canonicalized read-only path allowlists (workspace root + package/toolchain roots), reject symlink escapes and `/proc`/`/sys`, define redaction/privacy rules for `:Lineage debug-dump` and rotating logs, and document supply-chain and security risks in `design.md`.

---

## Majors

3. **[Major — `egm-design`] Architectural Contradiction: `expansion_token` vs. Ephemeral Tier-3 `SliceArena`**
   - **Affected Scope:** `specs/neovim-ui/spec.md` (L40–44), `specs/core-ir-and-slicer/spec.md` (L37), `design.md` (L28)
   - **Rationale:** Expanding `FrontierNode`/`ClusterNode` via `expansion_token` without full recomputation conflicts with dropping Tier-3 `SliceArena` on query completion. Specify stateless token encoding (coordinates, remaining budget, direction, filter flags, visited hash set) or a bounded TTL session store with `token_expired` fallback.

4. **[Major — `egm-design`] Sidecar Crash Recovery, `b:changedtick` Reset, and Lua-Side `<= 1 MB` Sync Guard**
   - **Affected Scope:** `specs/hybrid-lsp-protocol/spec.md` (L3–13, L32–37)
   - **Rationale:** Unspecified sidecar restart backoff/cap, silent stale-disk desync if Lua's `b:changedtick` map is not cleared on sidecar restart, and Neovim main-loop freeze if Lua `vim.json.encode`s a 5–20 MB dirty buffer before Rust applies its 1 MB cap. Add bounded restart backoff, full `changedtick` reset on reconnect, callback cleanup on exit, and a Lua-side `<= 1 MB` pre-check before `lineage/syncBuffer`.

5. **[Major — `egm-design`] Unbounded Dirty-Buffer Table & 85% RSS Eviction Spin-Loop**
   - **Affected Scope:** `specs/core-ir-and-slicer/spec.md` (L36–45)
   - **Rationale:** Keeping full dirty-buffer texts for dozens of loaded Neovim buffers outside Tier-1 LRU escapes the memory ceiling; evicting LRU entries under `mimalloc` does not drop OS RSS without explicit page purging (`mi_collect(true)`) and can spin-loop if RSS stays >85% when Tier 1 is empty. Account dirty-buffer bytes inside the budget, invoke allocator purge, and add hysteresis/cooldown.

6. **[Major — `egm-design`] Monorepo Manifests (`tsconfig.json` `"extends"`), Disk `stat()` Staleness, and Module/Closure Lowering**
   - **Affected Scope:** `specs/language-adapters/spec.md` (L44–49), `specs/core-ir-and-slicer/spec.md` (L3–12, L36–45)
   - **Rationale:** Caching one manifest per workspace root breaks monorepos with per-package `tsconfig.json` (`"extends"` / `"references"`), nested `pyproject.toml`, and workspace `Cargo.toml`. Also `core-ir-and-slicer/spec.md` omits `stat()` invalidation for externally modified disk files (`git checkout`), module top-level scope lowering, and nested closure capture boundaries.

7. **[Major — `egm-design`] Multi-Client LSP Fan-Out & Phase 1/Phase 2 Hover Slot Deduplication**
   - **Affected Scope:** `specs/hybrid-lsp-protocol/spec.md` (L21–30), `specs/neovim-ui/spec.md` (L10–12)
   - **Rationale:** Unspecified behavior when multiple LSP clients attach to one buffer (e.g., `vtsls` + `eslint`), unspecified handling of hidden Phase 1 origins `[6]..[N]` when Phase 2 returns 0 new origins, and missing canonical `(file_id, range)` deduplication between Phase 1 static and Phase 2 LSP origins.

8. **[Major — `egm-structural`] Cross-File Scope Contradiction on `go.mod` Static Resolver**
   - **Affected Scope:** `proposal.md` (L10), `design.md` (L23, L37), `specs/language-adapters/spec.md` (L45)
   - **Rationale:** `proposal.md` and `design.md` include `go.mod` in v1 static resolvers while `design.md` L23 declares Go post-v1 and `language-adapters/spec.md` omits `go.mod`. Remove `go.mod` from v1 scope.

9. **[Major — `egm-structural`] `SliceEnvelope` / `GraphDelta` Wire Contract Missing Dependent Fields**
   - **Affected Scope:** `specs/hybrid-lsp-protocol/spec.md` (L14–19) vs. `specs/neovim-ui/spec.md` (L40–44)
   - **Rationale:** Wire contract omits `root_ids`, `direction`, `truncation`, `stats`, `SourceNode`, projection keys (`hover_card`, `focus_spans`, `tree_rows`), `expansion_token` + expansion RPC, and `GraphDelta` payload fields.

10. **[Major — `egm-structural`] Variable Pinning Requirement Missing Behavioral Body and Scenario**
    - **Affected Scope:** `specs/neovim-ui/spec.md` (L32–37)
    - **Rationale:** Heading promises cross-buffer Variable Pinning, but the `SHALL` clause and Scenario omit `:Lineage pin`, `gp`, and cross-buffer behavior.

11. **[Major — `egm-structural`, `egm-style`] Contradictory State Notation `Present/Nil` in Correlated Guard Scenario**
    - **Affected Scope:** `specs/core-ir-and-slicer/spec.md` (L22)
    - **Rationale:** `Implies(StateIn(err, Present/Nil), StateIn(val, Present))` conflates opposite states. Fix antecedent to `StateIn(err, Nil)`.

12. **[Major — `egm-style`] Unresolved Numeric Ranges in Normative `SHALL` Requirements & Uncalibrated Claims**
    - **Affected Scope:** `design.md` (L64, L50), `specs/hybrid-lsp-protocol/spec.md` (L8, L22, L25–26, L40), `specs/core-ir-and-slicer/spec.md` (L8), `specs/language-adapters/spec.md` (L29–31)
    - **Rationale:**
      - `design.md` L64 writes `"Confirmed empirically during M1"` in past tense under Open Questions; change to `"To be measured..."`.
      - Normative `SHALL` clauses use unresolved ranges (`120–150ms`, `<= 200–500` files parsed, `2–4` parse workers); lock single concrete defaults.
      - `core-ir-and-slicer/spec.md` L8 and `hybrid-lsp-protocol/spec.md` L8 assert `"under 50ms"` without `(p95)` or warm-cache qualifiers.
      - `hybrid-lsp-protocol/spec.md` L25–26 omits the consecutive-timeout threshold to trip the circuit breaker.
      - `language-adapters/spec.md` L29–31 promises closures and `await` in title/`WHEN` but omits them in `THEN`.

---

## Nits

13. **[Nit — `egm-design`, `egm-structural`, `egm-style`]** Clarify M1 `<= 25%` metric denominator; carry `< 350ms` Tree Panel latency target into `neovim-ui/spec.md`; add a Go `(val, err)` Scenario to `language-adapters/spec.md`; clarify Windows Cargo fallback vs. prebuilt priority; align capability mapping for statically linked grammars (`proposal.md` L22 vs. `language-adapters/spec.md` L4); add brief component topology summary to `design.md`; replace anthropomorphic/filler words (`coverage honesty`, `strictly`, `self-DoS`, `proactive`, `trivially`, `unobtrusive`, `safely`, `cleanly`, `actionable`, `full`) and standardize `"Go (val, err) core IR validation suite"`.
