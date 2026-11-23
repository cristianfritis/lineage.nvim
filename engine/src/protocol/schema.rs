use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcId {
    Str(String),
    Num(i64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("Method not found: {method}"))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(-32603, message)
    }

    pub fn cancelled() -> Self {
        Self::new(-32800, "Request cancelled")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RpcId>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: RpcId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(id: RpcId, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: RpcId, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

impl RpcNotification {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OffsetEncoding {
    #[default]
    #[serde(rename = "utf-8", alias = "utf8")]
    Utf8,
    #[serde(rename = "utf-16", alias = "utf16")]
    Utf16,
    #[serde(rename = "utf-32", alias = "utf32")]
    Utf32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalPos {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalRange {
    pub start: CanonicalPos,
    pub end: CanonicalPos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SliceDirection {
    #[default]
    To,
    From,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub protocol_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineCapabilities {
    pub languages: Vec<String>,
    pub reverse_rpc: bool,
    pub delta_notifications: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub protocol_version: u32,
    pub schema_version: u32,
    pub engine_version: String,
    pub capabilities: EngineCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceParams {
    pub uri: String,
    pub position: LspPosition,
    #[serde(default)]
    pub offset_encoding: OffsetEncoding,
    #[serde(default)]
    pub direction: SliceDirection,
    #[serde(default)]
    pub include_graph: bool,
    #[serde(default = "default_true")]
    pub include_snippets: bool,
    #[serde(default)]
    pub exclude_tests: bool,
    #[serde(default)]
    pub query_id: Option<u64>,
    #[serde(default)]
    pub lsp_attached: bool,
    #[serde(default)]
    pub lsp_client_name: Option<String>,
    #[serde(default)]
    pub workspace_root: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBufferParams {
    pub uri: String,
    pub changedtick: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseBufferParams {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelRequestParams {
    #[serde(default)]
    pub query_id: Option<u64>,
    #[serde(default)]
    pub id: Option<RpcId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugDumpParams {
    pub uri: String,
    #[serde(default)]
    pub position: Option<LspPosition>,
    #[serde(default)]
    pub offset_encoding: OffsetEncoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    ExternalUntyped,
    DynamicDispatch,
    LspTimeout,
    LspUnattached,
    LspEmpty,
    CircuitOpen,
    HopBudgetExhausted,
    BarrelFanoutCap,
    FileTooLarge,
    NonRegularFile,
    TreeSitterErrorNode,
    PolymorphicCap,
    MaxDepth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncationReason {
    MaxDepth,
    MaxNodes,
    MaxFanout,
    WallClockBudget,
    HopBudgetExhausted,
    BarrelFanoutCap,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    SourceNode,
    ContractedChain,
    FrontierNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbsenceSummary {
    pub present: bool,
    pub maybe_null: bool,
    pub maybe_undefined: bool,
    pub unknown_reasons: Vec<UnknownReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: String,
    pub kind: NodeKind,
    pub file_id: u32,
    pub range: CanonicalRange,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub absence: AbsenceSummary,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub is_back_edge: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_group_id: Option<u32>,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub file_id: u32,
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceGraph {
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverItem {
    pub index: usize,
    pub file_id: u32,
    pub uri: String,
    pub range: CanonicalRange,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub provenance: String,
    pub is_absent_origin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverRisk {
    pub file_id: u32,
    pub uri: String,
    pub range: CanonicalRange,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnknownBoundary {
    pub file_id: u32,
    pub uri: String,
    pub range: CanonicalRange,
    pub symbol: String,
    pub reason: UnknownReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverCardProjection {
    pub symbol: String,
    pub verdict_line: String,
    pub status_badge: String,
    pub is_trivial_peek: bool,
    pub peek_lines: Vec<String>,
    pub phase1_items: Vec<HoverItem>,
    pub phase2_items: Vec<HoverItem>,
    pub guards: Vec<String>,
    pub local_risks: Vec<HoverRisk>,
    pub unknown_boundaries: Vec<UnknownBoundary>,
    pub unverified_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SliceProjections {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover_card: Option<HoverCardProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SliceStats {
    pub phase1_ms: u64,
    pub phase2_ms: u64,
    pub nodes_visited: usize,
    pub nodes_emitted: usize,
    pub contracted_chains: usize,
    pub unknown_external_generic_calls: usize,
    pub total_external_generic_calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceEnvelope {
    pub schema_version: u32,
    pub query_id: u64,
    pub trace_id: String,
    pub root_ids: Vec<String>,
    pub direction: SliceDirection,
    pub files: Vec<FileEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<SliceGraph>,
    pub projections: SliceProjections,
    pub truncation: Vec<TruncationReason>,
    pub stats: SliceStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceDeltaParams {
    pub query_id: u64,
    pub trace_id: String,
    pub envelope: SliceEnvelope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LspResolveKind {
    Definition,
    TypeDefinition,
    Implementation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspResolveItem {
    pub key: String,
    pub uri: String,
    pub position: LspPosition,
    pub offset_encoding: OffsetEncoding,
    pub kind: LspResolveKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspResolveBatchParams {
    pub query_id: u64,
    pub trace_id: String,
    pub depth: u32,
    pub timeout_ms: u64,
    pub items: Vec<LspResolveItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspLocationDto {
    pub uri: String,
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspResolveItemResult {
    pub key: String,
    #[serde(default)]
    pub locations: Vec<LspLocationDto>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub unattached: bool,
    #[serde(default)]
    pub circuit_open: bool,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub offset_encoding: Option<OffsetEncoding>,
    #[serde(default)]
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspResolveBatchResult {
    pub results: Vec<LspResolveItemResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientHealthSnapshot {
    pub client_name: String,
    pub workspace_root: String,
    pub ewma_ms: f64,
    pub circuit_state: String,
    pub consecutive_timeouts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResult {
    pub protocol_version: u32,
    pub schema_version: u32,
    pub engine_version: String,
    pub cache_bytes: u64,
    pub cache_cap_bytes: u64,
    pub dirty_buffers: usize,
    pub dirty_bytes: u64,
    pub loaded_grammars: Vec<String>,
    pub ewma_clients: Vec<ClientHealthSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
}
