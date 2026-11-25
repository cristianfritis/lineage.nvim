## ADDED Requirements

### Requirement: Complete `SliceGraph` Producer Emission, Phase 2 Merging, and URI Normalization
`engine/src/slicer/` and `engine/src/cache/memory.rs` SHALL harden `SliceGraph` under `SCHEMA_VERSION = 1` using additive `#[serde(default)]` fields only: `FlowNode` SHALL carry `is_guard`, `guard_text`, `is_danger_sink`, and `danger_kind` (preserved through copy-chain contraction and read directly by `project_focus_spans` instead of `"narrow "` prefix heuristics); `FlowEdge` SHALL carry `edge_side: Option<EdgeSide>` (`Upstream | Downstream`); `HoverRisk` SHALL carry `node_id: Option<String>`; `record_inst_node` SHALL emit def-use edges for `ProjectField`, `ProjectIndex`, and `UnwrapVariant`; `eval_inst` SHALL emit cross-file callee `RawSliceNode`s with real `file_id` and `cross_file_call` edges (`edge_side = Upstream`) during `To` slices; `pipeline.rs` SHALL populate `FlowEdge.cycle_group_id` via `scc::tarjan_scc` and `FlowEdge.is_back_edge` via a deterministic 3-color DFS feedback-arc pass; `project_tree_rows` SHALL track the active DFS recursion stack (`in_stack`) rather than a global visited set so diamond fan-in is never mislabeled `[cycle]`; `execute_phase2` SHALL parameterize `phase2_ceiling_ms` (and propagate `lsp_timeout_ms` to per-call LSP timeout) and merge resolved Phase 2 LSP/ripgrep nodes and edges directly into `ctx.pipeline_graph`; and `uri_to_path` SHALL percent-decode UTF-8 escape sequences (`%20`, etc.) and normalize Windows `file:///C:/...` drive paths.

#### Scenario: Diamond convergence is not mislabeled as a cycle in Tree or SliceGraph
- **WHEN** two distinct branches flow into a shared variable in a diamond topology (with zero recursive loop)
- **THEN** `project_tree_rows` renders both paths without `[cycle]`, and `SliceGraph` emits `is_back_edge = false` on all diamond edges

#### Scenario: Phase 2 LSP resolution and cross-file projections appear in SliceGraph
- **WHEN** a `To` slice traverses `ProjectField`/`ProjectIndex`/`UnwrapVariant` and resolves a cross-file callee or Phase 2 LSP definition
- **THEN** `SliceGraph` contains the projection edges, the cross-file node with its real `file_id`, merged Phase 2 nodes/edges with updated `absence`, and typed `is_guard`/`is_danger_sink`/`HoverRisk.node_id` metadata

#### Scenario: Percent-encoded and Windows file URIs normalize cleanly against workspace root
- **WHEN** `uri_to_path` decodes `file:///home/user/my%20repo/src/a.ts` or `file:///C:/Users/dev/repo/src/a.ts`
- **THEN** `uri_to_path` returns the decoded filesystem path (`.../my repo/src/a.ts` or `C:/Users/dev/repo/src/a.ts`) so `strip_prefix(workspace_root)` succeeds reliably
