pub mod dot;
pub mod html;
pub mod ir;
pub mod layout;
pub mod mermaid;
pub mod sanitize;

use std::path::PathBuf;
use std::time::Instant;

use crate::export::ir::{ExportGraph, DEFAULT_MAX_EDGES, DEFAULT_MAX_NODES, MAX_LABEL_CHARS};
use crate::protocol::schema::{
    ExportFormat, ExportResult, ExportSourceMode, Phase2Status, SliceEnvelope, TruncationReason,
    SCHEMA_VERSION,
};

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub workspace_root: Option<PathBuf>,
    pub null_only: bool,
    pub exclude_tests: bool,
    pub include_snippets: bool,
    pub absolute_paths: bool,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_label_chars: usize,
    pub lsp_attached: bool,
    pub lsp_timeout_ms: u64,
    pub phase2_status: Phase2Status,
    pub source_mode: ExportSourceMode,
    pub phase2_wait_ms: u64,
    pub static_reason: Option<String>,
}

impl ExportOptions {
    #[allow(clippy::too_many_arguments)]
    pub fn for_format(
        format: ExportFormat,
        workspace_root: Option<PathBuf>,
        null_only: bool,
        include_snippets: Option<bool>,
        absolute_paths: bool,
        lsp_attached: bool,
        lsp_timeout_ms: u64,
        phase2_status: Phase2Status,
        source_mode: ExportSourceMode,
        phase2_wait_ms: u64,
        static_reason: Option<String>,
    ) -> Self {
        Self {
            format,
            workspace_root,
            null_only,
            exclude_tests: false,
            // Uniform default across all Stage C.1 formats unless explicitly overridden.
            include_snippets: include_snippets.unwrap_or(false),
            absolute_paths,
            max_nodes: DEFAULT_MAX_NODES,
            max_edges: DEFAULT_MAX_EDGES,
            max_label_chars: MAX_LABEL_CHARS,
            lsp_attached,
            lsp_timeout_ms,
            phase2_status,
            source_mode,
            phase2_wait_ms,
            static_reason,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportOutput {
    pub result: ExportResult,
    pub graph: ExportGraph,
    pub render_ms: u64,
}

pub fn render_export(
    envelope: &SliceEnvelope,
    opts: &ExportOptions,
    trace_id: &str,
) -> ExportOutput {
    let start = Instant::now();
    let initial_graph = ExportGraph::build(envelope, opts);

    let mut html_telemetry: Option<(u64, u64, layout::LayoutMetrics)> = None;
    let (content, final_graph, warnings) = match opts.format {
        ExportFormat::Mermaid => mermaid::render_mermaid(envelope, &initial_graph, opts),
        ExportFormat::Dot => (
            dot::render_dot(&initial_graph, opts),
            initial_graph,
            Vec::new(),
        ),
        ExportFormat::Html => {
            let layout_start = Instant::now();
            let svg_layout = layout::compute_layout(&initial_graph, opts);
            let layout_ms = layout_start.elapsed().as_millis() as u64;
            let ser_start = Instant::now();
            let rendered = html::render_html(&initial_graph, &svg_layout, opts);
            let serialize_ms = ser_start.elapsed().as_millis() as u64;
            html_telemetry = Some((layout_ms, serialize_ms, svg_layout.metrics));
            (rendered, initial_graph, Vec::new())
        }
    };

    let render_ms = start.elapsed().as_millis() as u64;
    let node_count = final_graph.nodes.len();
    let edge_count = final_graph.edges.len();
    let truncated_nodes = final_graph.truncated_nodes;
    let truncated_edges = final_graph.truncated_edges;
    let truncation: Vec<TruncationReason> = final_graph.completeness.truncation.clone();
    let completeness_badges = final_graph.completeness.badges.clone();
    let output_bytes = content.len();

    if let Some((layout_ms, serialize_ms, metrics)) = html_telemetry {
        tracing::info!(
            trace_id = %trace_id,
            query_id = envelope.query_id,
            format = "html",
            source_mode = ?opts.source_mode,
            phase2_status = ?opts.phase2_status,
            phase2_wait_ms = opts.phase2_wait_ms,
            layout_ms = layout_ms,
            serialize_ms = serialize_ms,
            node_count = node_count,
            edge_count = edge_count,
            rank_count = metrics.rank_count,
            dummy_nodes_added = metrics.dummy_nodes_added,
            dummy_cap_hit = metrics.dummy_cap_hit,
            feedback_back_edges = metrics.feedback_back_edges,
            overflow_edges = metrics.overflow_edges,
            crossing_sweeps = metrics.crossing_sweeps,
            svg_width = metrics.svg_width,
            svg_height = metrics.svg_height,
            output_bytes = output_bytes,
            truncated_nodes = truncated_nodes,
            truncated_edges = truncated_edges,
            "html graph export completed"
        );
    } else {
        tracing::info!(
            trace_id = %trace_id,
            query_id = envelope.query_id,
            format = ?opts.format,
            source_mode = ?opts.source_mode,
            phase2_status = ?opts.phase2_status,
            phase2_wait_ms = opts.phase2_wait_ms,
            render_ms = render_ms,
            node_count = node_count,
            edge_count = edge_count,
            output_bytes = output_bytes,
            truncation = ?truncation,
            "graph export completed"
        );
    }

    let result = ExportResult {
        schema_version: SCHEMA_VERSION,
        format: opts.format,
        content,
        source_mode: opts.source_mode,
        phase2_status: opts.phase2_status,
        node_count,
        edge_count,
        truncated_nodes,
        truncated_edges,
        truncation,
        completeness_badges,
        warnings,
    };

    ExportOutput {
        result,
        graph: final_graph,
        render_ms,
    }
}
