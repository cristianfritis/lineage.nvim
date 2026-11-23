## ADDED Requirements

### Requirement: Pluggable `LanguageAdapter` Contract and Statically Linked JS/TS Grammars
`lineage-engine` SHALL isolate language-specific parsing, lowering, absence lattice definitions, built-in stdlib summaries, test-file classification, and static import resolution behind a pluggable adapter interface and SHALL statically link TypeScript, TSX, and JavaScript grammars.

#### Scenario: TSX and JSX files parse with their matching grammar without syntax errors
- **WHEN** a query targets a `.tsx` file or a `.js`/`.jsx`/`.mjs`/`.cjs` file
- **THEN** `.tsx` dispatches to the TSX grammar and `.js`/`.jsx`/`.mjs`/`.cjs` dispatches to the JavaScript grammar without spurious JSX `ERROR` nodes

#### Scenario: Broken buffer syntax lowers sibling statements deterministically
- **WHEN** a buffer contains a mid-edit syntax error producing syntax error or missing nodes
- **THEN** the adapter emits an `Unknown` (`tree_sitter_error_node`) for the broken subtree while still lowering valid sibling statements

### Requirement: TypeScript and JavaScript Absence, Narrowing, and Danger-Sink Semantics
The TypeScript/JavaScript adapter SHALL distinguish `null` from `undefined`, model truthiness asymmetry, desugar `?.`, `??`, `??=`, and `!`, model discriminated unions and type predicates (`x is T`), classify unguarded member access, function calls, and element indexing on `Absent`/`MaybeAbsent` values as intraprocedural danger sinks, and apply conservative kill/gen rules for shallow 1-level `obj.field`, closures, and `async`/`await`.

#### Scenario: TypeScript distinguishes `null` vs `undefined` and truthiness asymmetry
- **WHEN** a TypeScript function checks `if (x !== null)` on `x: string | null | undefined` or checks `if (x)` with an `else` branch
- **THEN** `x !== null` retains `undefined` as a possible absence state, and the `else` branch of `if (x)` remains `MaybeAbsent` rather than `DefinitelyAbsent`

#### Scenario: Shallow 1-level `obj.field`, closures, and `async`/`await` follow conservative kill/gen rules
- **WHEN** `obj.field` is narrowed to `Present` and followed by an unrelated call `log("ok")`, a call passing `obj` (`mutate(obj)`), an `await` suspension point, or a nested closure capture
- **THEN** `log("ok")` preserves `obj.field`; `mutate(obj)` and `await` downgrade `obj.field` to `Unknown` (never `Absent`); immutable single-assignment captures inherit outer narrowing while reassigned captures downgrade to `Unknown`; and `await expr` transparently unwraps `Promise<T>` return provenance

### Requirement: Declaration-Only Outer-Wrapper Signature Extraction and Built-In Stdlib Summaries
When a call resolves to an external `.d.ts` declaration stub, the adapter SHALL parse only the target declaration signature span (never lowering external bodies and never scraping LSP `hover` text), inspecting only the outermost nullable wrapper (`T | null`, `T | undefined`, `T | null | undefined` -> `MaybeAbsent`; bare uninstantiated `T` -> `Unknown: external_untyped`), backed by a built-in JS/TS stdlib summary table (`Array.find`, `Map.get`, `RegExp.exec`, `querySelector`).

#### Scenario: Generic outer-wrapper return proves `MaybeAbsent` without generic instantiation
- **WHEN** a TypeScript call `items.find(fn)` resolves to `Array<T>.find(...): T | undefined`
- **THEN** the adapter extracts `MaybeAbsent` (`undefined`) directly from the outermost union wrapper without instantiating `T`

#### Scenario: Bare uninstantiated generic return falls back to `Unknown`
- **WHEN** an external `.d.ts` signature returns a bare uninstantiated type parameter `T`
- **THEN** the call result is classified as `Unknown` (`external_untyped`) rather than fabricating a null origin

### Requirement: Bounded Static `tsconfig.json` and Barrel Import Resolution
The TypeScript/JavaScript adapter SHALL include a zero-LSP static import resolver keyed by the nearest-ancestor `tsconfig.json` (supporting JSONC comments/trailing commas, `paths`/`baseUrl`, single-level relative `"extends"`, and `index.ts` barrels) with a Phase 1 cap on wildcard `export *` fan-out (max 2 barrel hops, max 10 wildcard siblings, 10ms sub-budget).

#### Scenario: Standalone cross-file hover resolves `tsconfig.json` path aliases without LSP
- **WHEN** a TypeScript file imports `getUser` from `@app/users` (mapped in the nearest-ancestor `tsconfig.json` to `src/users/index.ts`) with no LSP server attached
- **THEN** `lineage-engine` resolves `@app/users` statically to `src/users/index.ts` and slices `getUser` in Phase 1

#### Scenario: Wildcard mega-barrel caps Phase 1 work and defers cleanly
- **WHEN** static resolution encounters an `export *` barrel exceeding 2 hops, 10 wildcard siblings, or 10ms
- **THEN** Phase 1 stops barrel expansion immediately and defers the symbol to Phase 2 LSP `definition` (or emits `Unknown: barrel_fanout_cap` when standalone)
