## ADDED Requirements

### Requirement: Interactive Two-Phase Hover with Prefixed Keymaps and Stable Anchoring
`:Lineage hover` (and `<Plug>(LineageHover)`) SHALL open a floating window (`enter_on_open = true`, configurable) that preserves standard Vim counts (`3j`) and motions (`j`, `k`, `/`, `n`, `N`, `f`, `t`), binding user-remappable defaults `<CR>` (jump to item under cursor), `g1`–`g9` (direct numbered jump), `gu` (toggle unverified boundaries), and `q`/`<Esc>` (close and restore cursor), and SHALL never overwrite user `<leader>l*` mappings on plugin load.

#### Scenario: Typing a Vim count inside focused hover does not trigger a jump
- **WHEN** the hover float is focused and the user presses `3j`
- **THEN** the cursor moves down 3 lines inside the hover float instead of jumping to origin `[3]`

#### Scenario: Phase 2 appends without renumbering Phase 1 items, duplicating origins, or flipping anchor
- **WHEN** Phase 1 renders `[1]..[k]` (capped at 5 when open cross-file boundaries exist) anchored once (`NW`/`SW`) and Phase 2 completes within 250ms with cross-file origins
- **THEN** `[1]..[k]` keep their exact indices and line positions, Phase 2 deduplicates against Phase 1 by `(file_id, range)`, appends new origins at `[k+1]..[9]`, and grows without flipping the float across the cursor line

### Requirement: Line-1 Partial-Coverage Qualifiers and Simple-Value Compact Peek
The hover Line-1 verdict strip SHALL qualify coverage whenever any upstream path terminated at `Unknown`, hit a budget/barrel cap, or timed out in Phase 2, SHALL display `Unverified` (never raw `Unknown`) in TypeScript/JavaScript, and SHALL render a compact 1–3 line peek for `Present` variables with zero absence origins, zero danger sinks, and zero unverified boundaries.

#### Scenario: Unverified external boundary prevents an unqualified NonNull badge
- **WHEN** a variable has no proven null origins locally but calls an unverified external library
- **THEN** Line 1 displays a partial qualifier (for example, `user: NonNull (partial — 1 unverified [gu])`) and pressing `gu` expands the unverified boundary with its machine-readable reason

#### Scenario: Simple constant displays compact peek
- **WHEN** the user triggers hover on `const count = 0` with zero origins, zero danger sinks, and zero unverified boundaries
- **THEN** hover renders a compact 1–3 line summary instead of empty analytical sections

### Requirement: Native Jumplist Integration and Lineage Jump Stack
Every direct redirection from Hover SHALL record the origin position in Neovim's native jumplist (`normal! m'`) and push onto the Lineage Jump Stack so both `<C-o>` and `:Lineage back` return to the origin cursor.

#### Scenario: Redirecting to a cross-file null origin and returning
- **WHEN** the user presses `g1` (or `<CR>` on `[1]`) in hover to jump to `auth.ts:42` and then runs `:Lineage back` (or presses `<C-o>`)
- **THEN** the cursor returns to the exact origin file and position
