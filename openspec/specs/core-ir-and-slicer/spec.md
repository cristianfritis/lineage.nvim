## ADDED Requirements

### Requirement: Two-Granularity Lazy Parsing and Query-Scoped AST Disposal
`lineage-engine` SHALL parse source files on demand into a lightweight file skeleton (imports, exports, top-level function byte ranges, and content hashes) and SHALL pass `&FileSkeleton` into `LanguageAdapter::lower_function` across TypeScript, Python, and Rust adapters so lowering reuses cached imports without re-parsing the full file on every uncached function, while Rust import extraction SHALL parse nested braced `use` declarations and glob `use` declarations (`local_name = "*"`, `imported_name = "*"`) cleanly. Parsed syntax trees SHALL be dropped unconditionally when lowering finishes.

#### Scenario: Cold hover on a large file lowers only the enclosing function without double-parsing
- **WHEN** a hover query targets a symbol inside a 50-line function within a 3,000-line file
- **THEN** `lineage-engine` builds the file skeleton once, passes `&FileSkeleton` to `lower_function` to lower only the enclosing function into SSI IR, drops the syntax tree, and responds within the `< 50ms` (p95) Phase 1 budget

#### Scenario: Editing one function preserves cached IR of sibling functions
- **WHEN** function A is edited in a file while function B's byte slice and content hash remain unchanged
- **THEN** function A is re-lowered on next query and function B's cached SSI IR and summary remain valid

#### Scenario: Nested braced and glob Rust `use` declarations resolve imported bindings
- **WHEN** a Rust file contains nested braced imports (`use std::{collections::{HashMap, HashSet}, io};`) or glob imports (`use crate::helpers::*;`)
- **THEN** `extract_use_decl` records all leaf imported bindings and glob wildcards (`local_name = "*"`, `imported_name = "*"`) without truncating on the first brace

### Requirement: Pruned SSI Core IR and Bounded Relational Guard Algebra
The core IR SHALL represent lowered functions in Pruned SSI form (`Phi` merges, `Pi` branch-edge refinements, field/index/variant projections, and multi-output calls with `Normal | EarlyExit` exit edges) and SHALL constrain branch guard facts to `StateIn(Place, BitMask)`, bounded `Implies(FactA, FactB)`, and `Unreachable`.

#### Scenario: Dead branches after terminating statements do not pollute post-branch state
- **WHEN** a branch assigns `null` or `undefined` and terminates via `return` or `throw`
- **THEN** the post-branch merge excludes the unreachable branch and preserves the surviving narrowed state

#### Scenario: Correlated guard narrows a sibling variable via `Implies`
- **WHEN** a multi-output call or discriminated union establishes `Implies(StateIn(tag, Present), StateIn(val, Present))` (or `Implies(StateIn(err, Nil), StateIn(val, Present))`) and a branch guards on the witness
- **THEN** the continuation after the guard narrows `val` to present without invoking an SMT solver

### Requirement: Structural Separation of `Unknown` and `Absent` in a 2-Axis Product Lattice
The absence lattice (`engine/src/ir/lattice.rs` and `engine/src/protocol/schema.rs`) SHALL represent every value with two orthogonal axes—proven adapter-native states and machine-readable unverified boundary reasons including `UnknownReason::ExternalUninstalled`—so `Unknown` never collapses into `MaybeAbsent` at join points; SHALL support clearing a resolved `UnknownReason` bit via a helper (`without_unknown`) only when zero matching entries remain in `updated_boundaries`; SHALL never silently default `AbsenceValue::bottom()`, missed cursor lookups (`lookup_at_pos`), missing SSA values, or unresolved `CallTarget::Local` calls to `NonNull` / `Present`; SHALL ensure `project_focus_spans` and `project_tree_rows` classify `is_absent` strictly from `maybe_null || maybe_undefined` so pure `Unknown` / `bottom()` nodes render `[?]` (`Unverified`) rather than `[NULL]`; and SHALL populate and propagate 1-hop `FnSummary.param_flow_to_return` across TypeScript, Python, and Rust lowerers and `slicer/mod.rs`.

#### Scenario: Joining Present and Unknown does not produce a nullable warning
- **WHEN** branch 1 assigns a proven present value and branch 2 calls an unverified external function (`external_untyped`)
- **THEN** the merged state retains proven `Present` alongside the `external_untyped` boundary reason and is excluded from proven null-origin and danger-sink lists

#### Scenario: Joining Absent and Unknown preserves the witnessed absence origin
- **WHEN** branch 1 assigns `null` and branch 2 calls an unverified external function
- **THEN** the merged state preserves the witnessed `null` origin alongside the unverified boundary reason

#### Scenario: Unresolved local call, empty SSA value, or pure Unknown renders Unverified in Hover, Focus, and Tree
- **WHEN** a slice encounters `AbsenceValue::bottom()`, a missed `lookup_at_pos` cursor position, an unresolved `CallTarget::Local` closure/method, or a pure `Unknown` node
- **THEN** Hover renders `Unverified` with a machine-readable `UnknownReason` (never unqualified `NonNull`), and Focus/Tree project `[?]` (`LineageStateUnknown`) rather than `[NULL]` (`LineageStateAbsent`)

#### Scenario: 1-hop parameter flow propagates nullable caller arguments through pass-through callees
- **WHEN** a caller passes a `Nullable` value to a local or statically resolved helper whose `FnSummary.param_flow_to_return` includes that parameter index
- **THEN** `slicer/mod.rs` propagates the caller argument's `Nullable` state to the call result instead of returning a static `Present` summary

### Requirement: Bounded Memory Cache, Disk Staleness Validation, and Pre-Parse/Path Guards
`lineage-engine` SHALL enforce a recalibrated byte-weighted LRU cache cap of `clamp(3% host RAM, 64 MB, 140 MB)` (calibrated to keep total OS RSS `<= 300 MB`) storing strictly file-local parametric function summaries, SHALL replace hand-rolled per-adapter workspace config `Mutex<HashMap>` caches (`TsConfigCache`, `PyProjectCache`, `CargoWorkspaceCache`) with `moka::sync::Cache::new(64)`, SHALL remove unused `SliceArena` dead code, SHALL bound in-memory dirty-buffer overlays (max 32 buffers / 16 MB LRU), SHALL validate cached unopened disk files via file metadata (`mtime`, length) on traversal, SHALL reject non-regular files, `/proc`/`/sys` paths, files exceeding 1 MB, and files with lines exceeding 4096 bytes, and SHALL guard static manifest relative joins against `..` escape outside the workspace root.

#### Scenario: Large, minified, or non-regular files are skipped without hanging
- **WHEN** a traversal encounters a FIFO/named pipe, a `/proc` path, or a file exceeding 1 MB or containing a line longer than 4096 bytes
- **THEN** `lineage-engine` skips parsing without hanging or spiking memory and emits an `Unknown` boundary tagged `file_too_large` or `non_regular_file`

#### Scenario: External disk edit invalidates cached unopened file IR
- **WHEN** an unopened file previously cached in the LRU is modified on disk (for example via `git checkout`)
- **THEN** the metadata check detects the changed `mtime` or length and re-parses the file on the next query

### Requirement: SCC Fixed-Point Termination and 5-Stage Deterministic Reduction Pipeline
Interprocedural slicing SHALL terminate on direct and mutual recursion via Tarjan SCC and bounded monotone worklist iteration (max 8 iterations before widening to `Unknown`, emitting a structural back-edge rather than duplicating canonical nodes), SHALL execute a deterministic 5-stage reduction pipeline with mode-aware wall-clock budgets (`50ms` for Phase 1 Hover, `300ms` for initial `Tree`/`FromHere`), SHALL capture `stats.phase1_ms` after the reduction pipeline and all projections complete, and SHALL record `SliceStats.uninstalled_external_calls` (`#[serde(default)]`) alongside reconciled external-call counters across `Stdlib`, `Imported`, and `Dynamic`.

#### Scenario: Recursive call cycle converges and emits a structural back-edge
- **WHEN** function `f` calls `g` and `g` calls `f` while tracing origins of a return value
- **THEN** the SCC worklist converges within 8 iterations, preserves base-case origins, and emits a structural back-edge rather than duplicating canonical nodes

#### Scenario: Test-file exclusion does not starve production node budget
- **WHEN** a slice runs with test files excluded on a symbol referenced across test and production files
- **THEN** test files are filtered before debiting `max_nodes` or `max_fanout`

#### Scenario: Initial Tree and FromHere queries use the 300ms pipeline budget and report end-to-end phase1_ms
- **WHEN** a client requests `direction = "from"` with `include_tree = true` and `discover_callers` runs for 70ms
- **THEN** the reduction pipeline uses the `300ms` mode budget instead of aborting at `50ms`, and `stats.phase1_ms` reflects the total duration including reduction and tree projection
