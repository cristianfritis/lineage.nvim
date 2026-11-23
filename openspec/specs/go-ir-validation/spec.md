# go-ir-validation Specification

## Purpose
TBD - created by archiving change lineage-m2-python-rust-adapters. Update Purpose after archive.
## Requirements
### Requirement: Go `(val, err)` Core IR Specification Validation
`lineage-engine` SHALL include an automated integration test suite (`engine/tests/ir_go_validation.rs`) constructing Pruned SSI + `GuardFact` graphs that model Go zero-value initialization, `(val, err)` and comma-ok relational `Implies` narrowing, and `defer`-mutated named returns with zero modifications to `CoreIR` or `SlicerEngine`.

#### Scenario: Correlated `(val, err)` guard narrows `val` via `Implies`
- **WHEN** a multi-output `Call` produces `(val, err)` with `Implies(StateIn(err, Nil), StateIn(val, Present))` and a branch checks `if err != nil { return }`
- **THEN** `SlicerEngine` narrows `val` to `Present` in the surviving continuation without an SMT solver

#### Scenario: Zero-value initialization and `defer` named-return mutation
- **WHEN** a Go IR graph models an uninitialized pointer/slice/map (`Nil`) vs a value type (`Present`), or a named return mutated inside `defer` (`Unknown`)
- **THEN** `SlicerEngine` reports `Absent` for uninitialized reference types, `Present` for value types, and `Unknown` for `defer`-mutated named returns

