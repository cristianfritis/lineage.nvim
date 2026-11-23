## ADDED Requirements

### Requirement: Inline Focus Mode and Cross-Buffer Variable Pinning
`lineage.nvim` SHALL provide `:Lineage focus`, `:Lineage pin`, `:Lineage clear` (and Hover pivots `gs` / `gp`), rendering engine-projected `focus_spans` via extmarks that dim unrelated lines using `Comment`-linked highlights, highlight relevant statements paired with text badges (`[NULL]`, `[OK]`, `[?]`), and display an unobtrusive active-mode/pin indicator.

#### Scenario: Activating Focus Mode dims unrelated lines and tags slice statements
- **WHEN** the user runs `:Lineage focus` (or presses `gs` from Hover) on a variable inside a function
- **THEN** lines outside the slice dim with `LineageFocusDim` (linked to `Comment`), slice statements receive semantic highlights and text-paired state badges, and an active-mode indicator confirms Focus Mode is active

#### Scenario: Editing a focused buffer marks the slice stale without blocking keystrokes
- **WHEN** Focus Mode is active and the user edits text in the buffer (`TextChanged` / `TextChangedI`)
- **THEN** the indicator immediately transitions to `[stale]` without synchronous re-slicing on each keystroke, refreshing on debounced idle or manual `:Lineage focus`

#### Scenario: Pinning a symbol persists focus context across buffer switches
- **WHEN** the user runs `:Lineage pin` (or presses `gp` from Hover) and switches to another buffer
- **THEN** the pinned symbol state remains active until cleared via `:Lineage clear`
