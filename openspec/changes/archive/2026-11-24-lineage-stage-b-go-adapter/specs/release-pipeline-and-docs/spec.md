## MODIFIED Requirements

### Requirement: Complete User Documentation and Vim Help Tags
The repository SHALL ship `README.md` and `doc/lineage.txt` referencing canonical repo `cristianfritis/lineage.nvim`, documenting Go alongside TypeScript/TSX/JavaScript, Python, and Rust (including `gopls` support), synchronized `config.lua` defaults (`binary_path`, `version = nil`, `build_from_source = false`), decoupled `engine-v*` / `v*` installation (`:Lineage install`, `:Lineage install!`, network Cargo fallback + `source_build` provenance), all `:Lineage` subcommands, `<Plug>` mappings, Hover/Focus/Tree keymaps, coverage-honesty indicators (`NonNull`, `Nullable`, `Absent`, `Unverified`, `[partial]`), troubleshooting (`:checkhealth lineage`, `:Lineage status`, `:Lineage log`), and explicit v1.1 scope boundaries (with `doc/tags` gitignored).

#### Scenario: Vim help file exposes standard help tags and canonical repository slug
- **WHEN** a user runs `:help lineage.txt` or `:help lineage-commands` after `:helptags doc`
- **THEN** `doc/lineage.txt` contains matching `*lineage.txt*`, `*lineage-commands*`, `*lineage-config*`, `*lineage-keymaps*`, and `*lineage-health*` tags and references `cristianfritis/lineage.nvim`
