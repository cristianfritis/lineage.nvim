use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::protocol::schema::{
    LspResolveBatchParams, LspResolveBatchResult, LspResolveItemResult, LspResolveKind,
    OffsetEncoding,
};
use crate::resolver::ewma::{CircuitState, ClientHealthKey, EwmaTracker};

pub const DEFAULT_LSP_TIMEOUT_MS: u64 = 150;
pub const PHASE2_CEILING_MS: u64 = 250;
pub const MAX_IMPLEMENTATION_TARGETS: usize = 5;

pub type ReverseRpcSender =
    Arc<dyn Fn(LspResolveBatchParams) -> Result<LspResolveBatchResult, String> + Send + Sync>;

pub trait SymbolResolver: Send + Sync {
    fn resolve_batch(&self, params: LspResolveBatchParams, now: Instant) -> LspResolveBatchResult;

    fn ewma_tracker(&self) -> Option<&EwmaTracker> {
        None
    }
}

pub struct NullLspProvider;

impl SymbolResolver for NullLspProvider {
    fn resolve_batch(&self, params: LspResolveBatchParams, _now: Instant) -> LspResolveBatchResult {
        LspResolveBatchResult {
            results: params
                .items
                .into_iter()
                .map(|item| LspResolveItemResult {
                    key: item.key,
                    locations: Vec::new(),
                    timed_out: false,
                    unattached: true,
                    circuit_open: false,
                    client_name: None,
                    offset_encoding: Some(OffsetEncoding::Utf16),
                    latency_ms: 0,
                })
                .collect(),
        }
    }
}

pub struct StdioLspProvider {
    sender: ReverseRpcSender,
    ewma: Arc<EwmaTracker>,
    workspace_root: String,
    default_client: String,
}

impl StdioLspProvider {
    pub fn new(
        sender: ReverseRpcSender,
        ewma: Arc<EwmaTracker>,
        workspace_root: String,
        default_client: String,
    ) -> Self {
        Self {
            sender,
            ewma,
            workspace_root,
            default_client,
        }
    }
}

impl SymbolResolver for StdioLspProvider {
    fn resolve_batch(
        &self,
        mut params: LspResolveBatchParams,
        now: Instant,
    ) -> LspResolveBatchResult {
        let key = ClientHealthKey {
            client_name: self.default_client.clone(),
            workspace_root: self.workspace_root.clone(),
        };
        if self.ewma.check_circuit(&key, now) == CircuitState::Open {
            return LspResolveBatchResult {
                results: params
                    .items
                    .into_iter()
                    .map(|item| LspResolveItemResult {
                        key: item.key,
                        locations: Vec::new(),
                        timed_out: false,
                        unattached: false,
                        circuit_open: true,
                        client_name: Some(self.default_client.clone()),
                        offset_encoding: None,
                        latency_ms: 0,
                    })
                    .collect(),
            };
        }

        if params.timeout_ms == 0 {
            params.timeout_ms = self.ewma.suggested_timeout_ms(&key, DEFAULT_LSP_TIMEOUT_MS);
        }

        match (self.sender)(params.clone()) {
            Ok(res) => {
                for item in &res.results {
                    let client_key = ClientHealthKey {
                        client_name: item
                            .client_name
                            .clone()
                            .unwrap_or_else(|| self.default_client.clone()),
                        workspace_root: self.workspace_root.clone(),
                    };
                    if item.timed_out {
                        self.ewma.record_timeout(&client_key, now);
                    } else if !item.unattached && !item.circuit_open {
                        self.ewma
                            .record_success(&client_key, item.latency_ms as f64);
                    }
                }
                res
            }
            Err(_) => {
                self.ewma.record_timeout(&key, now);
                LspResolveBatchResult {
                    results: params
                        .items
                        .into_iter()
                        .map(|item| LspResolveItemResult {
                            key: item.key,
                            locations: Vec::new(),
                            timed_out: true,
                            unattached: false,
                            circuit_open: false,
                            client_name: Some(self.default_client.clone()),
                            offset_encoding: None,
                            latency_ms: params.timeout_ms,
                        })
                        .collect(),
                }
            }
        }
    }

    fn ewma_tracker(&self) -> Option<&EwmaTracker> {
        Some(&self.ewma)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureRule {
    pub key: String,
    #[serde(default)]
    pub kind: Option<LspResolveKind>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub offset_encoding: Option<OffsetEncoding>,
    #[serde(default)]
    pub locations: Vec<crate::protocol::schema::LspLocationDto>,
    #[serde(default)]
    pub simulate_timeout: bool,
    #[serde(default)]
    pub simulate_latency_ms: u64,
    #[serde(default)]
    pub sleep_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FixtureFile {
    #[serde(default)]
    pub rules: Vec<FixtureRule>,
}

/// Deterministic standalone LSP resolver for M0/M1/CLI tests.
pub struct FixtureLspProvider {
    rules: HashMap<String, Vec<FixtureRule>>,
    call_counts: std::sync::Mutex<HashMap<String, usize>>,
    pub ewma: EwmaTracker,
}

impl FixtureLspProvider {
    pub fn from_rules(rules: Vec<FixtureRule>) -> Self {
        let mut map: HashMap<String, Vec<FixtureRule>> = HashMap::new();
        for rule in rules {
            map.entry(rule.key.clone()).or_default().push(rule);
        }
        Self {
            rules: map,
            call_counts: std::sync::Mutex::new(HashMap::new()),
            ewma: EwmaTracker::new(),
        }
    }

    pub fn from_json_file(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let parsed: FixtureFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        Ok(Self::from_rules(parsed.rules))
    }
}

impl SymbolResolver for FixtureLspProvider {
    fn resolve_batch(&self, params: LspResolveBatchParams, now: Instant) -> LspResolveBatchResult {
        let mut results = Vec::with_capacity(params.items.len());
        for item in params.items {
            let matching = self.rules.get(&item.key);
            let rule_opt = matching.and_then(|list| {
                let mut counts = self.call_counts.lock().expect("fixture lock");
                let idx = counts.entry(item.key.clone()).or_insert(0);
                let chosen = list.get(*idx).or_else(|| list.last()).cloned();
                *idx += 1;
                chosen
            });

            if let Some(rule) = rule_opt {
                let client_key = ClientHealthKey {
                    client_name: rule
                        .client_name
                        .clone()
                        .unwrap_or_else(|| "fixture_ts".to_string()),
                    workspace_root: rule
                        .workspace_root
                        .clone()
                        .unwrap_or_else(|| "/workspace".to_string()),
                };

                if self.ewma.check_circuit(&client_key, now) == CircuitState::Open {
                    results.push(LspResolveItemResult {
                        key: item.key,
                        locations: Vec::new(),
                        timed_out: false,
                        unattached: false,
                        circuit_open: true,
                        client_name: Some(client_key.client_name),
                        offset_encoding: rule.offset_encoding,
                        latency_ms: 0,
                    });
                    continue;
                }

                if rule.sleep_ms > 0 {
                    std::thread::sleep(Duration::from_millis(rule.sleep_ms));
                }

                if rule.simulate_timeout {
                    self.ewma.record_timeout(&client_key, now);
                    results.push(LspResolveItemResult {
                        key: item.key,
                        locations: Vec::new(),
                        timed_out: true,
                        unattached: false,
                        circuit_open: false,
                        client_name: Some(client_key.client_name),
                        offset_encoding: rule.offset_encoding,
                        latency_ms: rule.simulate_latency_ms.max(params.timeout_ms),
                    });
                } else {
                    self.ewma
                        .record_success(&client_key, rule.simulate_latency_ms as f64);
                    results.push(LspResolveItemResult {
                        key: item.key,
                        locations: rule.locations,
                        timed_out: false,
                        unattached: false,
                        circuit_open: false,
                        client_name: Some(client_key.client_name),
                        offset_encoding: Some(
                            rule.offset_encoding.unwrap_or(OffsetEncoding::Utf16),
                        ),
                        latency_ms: rule.simulate_latency_ms,
                    });
                }
            } else {
                results.push(LspResolveItemResult {
                    key: item.key,
                    locations: Vec::new(),
                    timed_out: false,
                    unattached: false,
                    circuit_open: false,
                    client_name: None,
                    offset_encoding: Some(OffsetEncoding::Utf16),
                    latency_ms: 0,
                });
            }
        }
        LspResolveBatchResult { results }
    }

    fn ewma_tracker(&self) -> Option<&EwmaTracker> {
        Some(&self.ewma)
    }
}
