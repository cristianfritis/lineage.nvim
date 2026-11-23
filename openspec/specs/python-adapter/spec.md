# python-adapter Specification

## Purpose
TBD - created by archiving change lineage-m2-python-rust-adapters. Update Purpose after archive.
## Requirements
### Requirement: Statically Linked Python Grammar and Core Lowering
`lineage-engine` SHALL statically link `tree-sitter-python` and register a `PythonAdapter` supporting `.py` and `.pyi` files with two-granularity lazy parsing (`FileSkeleton` + function-scoped Pruned SSI lowering) and deterministic `ERROR`/`MISSING` recovery (`Unknown: tree_sitter_error_node`).

#### Scenario: Python `is None`, `assert`, and truthiness asymmetry
- **WHEN** a Python function checks `if x is not None:`, `assert x is not None`, or `if x:` on `x: str | None`
- **THEN** `is not None` and `assert x is not None` narrow `x` to `Present` in the continuation, while the `else` branch of `if x:` remains `MaybeAbsent`

#### Scenario: Python walrus operator, default `= None`, comprehensions, and `raise`
- **WHEN** a Python function uses `if (u := get_user()) is not None:`, a parameter `limit=None`, a filtered comprehension `[u for u in users if u is not None]`, or a branch ending in `raise ValueError()`
- **THEN** `u` is narrowed after the walrus guard, `limit=None` starts `MaybeAbsent`, comprehension filters narrow the element binding, and `raise` terminates its branch so dead `None` paths do not leak

#### Scenario: Broken Python syntax lowers valid sibling statements
- **WHEN** a `.py` buffer contains a mid-edit syntax error in one statement
- **THEN** the broken subtree lowers to `Unknown` (`tree_sitter_error_node`) while sibling valid statements remain sliceable without panicking

### Requirement: Python Shallow Mutation, `.pyi` Outer-Wrapper Extraction, Stdlib Summaries, and Static Imports
`PythonAdapter` SHALL apply conservative 1-level `obj.field` kill/gen rules, extract outermost `Optional[T]` / `T | None` wrappers from `.pyi` declaration stubs without lowering stub bodies, provide a curated stdlib summary table (`dict.get`, `re.search`, `re.match`, `next`, etc.), and resolve relative imports plus `pyproject.toml` / `src/` layouts statically.

#### Scenario: `.pyi` outer-wrapper extraction proves `MaybeAbsent`
- **WHEN** a call resolves to a `.pyi` stub returning `Optional[T]` or `T | None`
- **THEN** `PythonAdapter` extracts `MaybeAbsent` directly from the outer signature without instantiating generics

#### Scenario: Static `pyproject.toml` and relative import resolution
- **WHEN** a Python module imports a symbol via `.helpers` or from a package rooted under `src/` configured in `pyproject.toml` without an attached LSP
- **THEN** `PythonAdapter` resolves the target `.py` file statically in Phase 1 and caches workspace metadata once per root

