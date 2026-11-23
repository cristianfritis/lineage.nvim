## ADDED Requirements

### Requirement: Interactive Tree Panel with Grouping, Filtering, Preview, and Lazy Expansion
`lineage.nvim` SHALL provide `:Lineage to`, `:Lineage from`, and `:Lineage both` (plus Hover pivots `gt`, `gf`, `gn`) opening a collapsible split that renders engine-projected `tree_rows`, supports grouping (`Root Origin` / `Nullability`), null/absence-only filtering, pre-budget test-file toggling, cycle and `[partial]` truncation badges, synchronized non-focusable floating code preview, jump redirection (native jumplist + Lineage Jump Stack), and lazy expansion of `FrontierNode` / `ClusterNode` entries via `lineage/expand`.

#### Scenario: Opening Tree Panel from Hover or command renders bounded rows and preview
- **WHEN** the user runs `:Lineage to` or `:Lineage from` (or presses `gt`/`gf` in Hover)
- **THEN** a split opens displaying indented `tree_rows` with file/line labels, state badges, cycle/`[partial]` indicators, and a synchronized floating preview of the selected row without stealing focus

#### Scenario: Toggling test exclusion re-queries before budget caps
- **WHEN** the user toggles test-file exclusion inside the Tree Panel
- **THEN** `lineage.nvim` re-queries `lineage-engine` with `exclude_tests = true` so test files are filtered before `max_nodes` or `max_fanout` caps apply

#### Scenario: Expanding a FrontierNode fetches additional hops lazily
- **WHEN** the user triggers expansion on a `FrontierNode` row carrying an `expansion_token`
- **THEN** `lineage.nvim` sends `lineage/expand` and splices the returned child `tree_rows` into the Tree Panel in place
