## ADDED Requirements

### Requirement: Two-Granularity Lazy Parsing and Query-Scoped AST Disposal
`lineage-engine` SHALL parse source files on demand into a lightweight file skeleton (imports, exports, top-level function byte ranges, and content hashes) and SHALL lower only the enclosing function(s) (or synthetic `<module>` scope for top-level bindings) touched by a query into Pruned SSI IR. Parsed syntax trees SHALL be retained at most for the duration of the active query (capped at 4 active trees) and dropped unconditionally when the query completes.

#### Scenario: Cold hover on a large file lowers only the enclosing function
- **WHEN** a hover query targets a symbol inside a 50-line function within a 3,000-line file
- **THEN** `lineage-engine` builds the file skeleton, lowers only the enclosing function into SSI IR, drops the syntax tree when the query completes, and responds within the `< 50ms` (p95) Phase 1 budget

#### Scenario: Editing one function preserves cached IR of sibling functions
- **WHEN** function A is edited in a file while function B's byte slice and content hash remain unchanged
- **THEN** function A is re-lowered on next query and function B's cached SSI IR and summary remain valid

### Requirement: Pruned SSI Core IR and Bounded Relational Guard Algebra
The core IR SHALL represent lowered functions in Pruned SSI form (`Phi` merges, `Pi` branch-edge refinements, field/index/variant projections, and multi-output calls with `Normal | EarlyExit` exit edges) and SHALL constrain branch guard facts to `StateIn(Place, BitMask)`, bounded `Implies(FactA, FactB)`, and `Unreachable`.

#### Scenario: Dead branches after terminating statements do not pollute post-branch state
- **WHEN** a branch assigns `null` or `undefined` and terminates via `return` or `throw`
- **THEN** the post-branch merge excludes the unreachable branch and preserves the surviving narrowed state

#### Scenario: Correlated guard narrows a sibling variable via `Implies`
- **WHEN** a multi-output call or discriminated union establishes `Implies(StateIn(tag, Present), StateIn(val, Present))` (or `Implies(StateIn(err, Nil), StateIn(val, Present))`) and a branch guards on the witness
- **THEN** the continuation after the guard narrows `val` to present without invoking an SMT solver

### Requirement: Structural Separation of `Unknown` and `Absent` in a 2-Axis Product Lattice
The absence lattice SHALL represent every value with two orthogonal axes—proven adapter-native states and machine-readable unverified boundary reasons—so `Unknown` never collapses into `MaybeAbsent` at join points.

#### Scenario: Joining Present and Unknown does not produce a nullable warning
- **WHEN** branch 1 assigns a proven present value and branch 2 calls an unverified external function (`external_untyped`)
- **THEN** the merged state retains proven `Present` alongside the `external_untyped` boundary reason and is excluded from proven null-origin and danger-sink lists

#### Scenario: Joining Absent and Unknown preserves the witnessed absence origin
- **WHEN** branch 1 assigns `null` and branch 2 calls an unverified external function
- **THEN** the merged state preserves the witnessed `null` origin alongside the unverified boundary reason

### Requirement: Bounded Memory Cache, Disk Staleness Validation, and Pre-Parse/Path Guards
`lineage-engine` SHALL enforce a byte-weighted LRU cache ceiling of `clamp(5% host RAM, 128 MB, 300 MB)` storing strictly file-local parametric function summaries (no baked-in cross-file callee results), SHALL bound in-memory dirty-buffer overlays (max 32 buffers / 16 MB LRU), SHALL validate cached unopened disk files via file metadata (`mtime`, length) on traversal, SHALL reject non-regular files, `/proc`/`/sys` paths, files exceeding 1 MB, and files with lines exceeding 4096 bytes, and SHALL guard static manifest relative joins against `..` escape outside the workspace root.

#### Scenario: Large, minified, or non-regular files are skipped without hanging
- **WHEN** a traversal encounters a FIFO/named pipe, a `/proc` path, or a file exceeding 1 MB or containing a line longer than 4096 bytes
- **THEN** `lineage-engine` skips parsing without hanging or spiking memory and emits an `Unknown` boundary tagged `file_too_large` or `non_regular_file`

#### Scenario: External disk edit invalidates cached unopened file IR
- **WHEN** an unopened file previously cached in the LRU is modified on disk (for example via `git checkout`)
- **THEN** the metadata check detects the changed `mtime` or length and re-parses the file on the next query

### Requirement: SCC Fixed-Point Termination and 5-Stage Deterministic Reduction Pipeline
Interprocedural slicing SHALL terminate on direct and mutual recursion via Tarjan SCC and bounded monotone worklist iteration (max 8 iterations before widening to `Unknown`, emitting a structural back-edge rather than duplicating canonical nodes) and SHALL execute a deterministic 5-stage reduction pipeline (pre-budget test filtering, zero-cost copy-chain contraction into `ContractedChain`, deterministic best-first scheduling, orthogonal caps `max_depth=5`/`max_nodes=150`/`max_fanout=12`/wall-clock budget emitting `FrontierNode`, and pure post-slice view projections).

#### Scenario: Recursive call cycle converges and emits a structural back-edge
- **WHEN** function `f` calls `g` and `g` calls `f` while tracing origins of a return value
- **THEN** the SCC worklist converges within 8 iterations, preserves base-case origins, and emits a structural back-edge rather than duplicating canonical nodes

#### Scenario: Test-file exclusion does not starve production node budget
- **WHEN** a slice runs with test files excluded on a symbol referenced across test and production files
- **THEN** test files are filtered before debiting `max_nodes` or `max_fanout`
