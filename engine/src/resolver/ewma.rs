use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::protocol::schema::ClientHealthSnapshot;

pub const EWMA_ALPHA: f64 = 0.25;
pub const COLD_START_MS: f64 = 300.0;
pub const CONSECUTIVE_TIMEOUT_THRESHOLD: u32 = 3;
pub const CIRCUIT_OPEN_DURATION: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientHealthKey {
    pub client_name: String,
    pub workspace_root: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
struct ClientHealthEntry {
    ewma_ms: f64,
    consecutive_timeouts: u32,
    state: CircuitState,
    opened_at: Option<Instant>,
}

impl Default for ClientHealthEntry {
    fn default() -> Self {
        Self {
            ewma_ms: COLD_START_MS,
            consecutive_timeouts: 0,
            state: CircuitState::Closed,
            opened_at: None,
        }
    }
}

#[derive(Default)]
pub struct EwmaTracker {
    entries: Mutex<HashMap<ClientHealthKey, ClientHealthEntry>>,
}

impl EwmaTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check_circuit(&self, key: &ClientHealthKey, now: Instant) -> CircuitState {
        let mut guard = self.entries.lock().expect("ewma lock poisoned");
        let entry = guard.entry(key.clone()).or_default();
        if entry.state == CircuitState::Open {
            if let Some(opened_at) = entry.opened_at {
                if now.duration_since(opened_at) >= CIRCUIT_OPEN_DURATION {
                    entry.state = CircuitState::HalfOpen;
                }
            }
        }
        entry.state
    }

    pub fn record_success(&self, key: &ClientHealthKey, latency_ms: f64) {
        let mut guard = self.entries.lock().expect("ewma lock poisoned");
        let entry = guard.entry(key.clone()).or_default();
        entry.ewma_ms = EWMA_ALPHA * latency_ms + (1.0 - EWMA_ALPHA) * entry.ewma_ms;
        entry.consecutive_timeouts = 0;
        entry.state = CircuitState::Closed;
        entry.opened_at = None;
    }

    pub fn record_timeout(&self, key: &ClientHealthKey, now: Instant) {
        let mut guard = self.entries.lock().expect("ewma lock poisoned");
        let entry = guard.entry(key.clone()).or_default();
        entry.consecutive_timeouts += 1;
        if entry.consecutive_timeouts >= CONSECUTIVE_TIMEOUT_THRESHOLD
            || entry.state == CircuitState::HalfOpen
        {
            entry.state = CircuitState::Open;
            entry.opened_at = Some(now);
        }
    }

    pub fn suggested_timeout_ms(&self, key: &ClientHealthKey, default_ms: u64) -> u64 {
        let guard = self.entries.lock().expect("ewma lock poisoned");
        if let Some(entry) = guard.get(key) {
            let scaled = (entry.ewma_ms * 2.0).round() as u64;
            scaled.clamp(50, default_ms)
        } else {
            default_ms
        }
    }

    pub fn open_clients_for_workspace(&self, workspace_root: &str, now: Instant) -> Vec<String> {
        let mut guard = self.entries.lock().expect("ewma lock poisoned");
        let mut open = Vec::new();
        for (k, entry) in guard.iter_mut() {
            if k.workspace_root == workspace_root {
                if entry.state == CircuitState::Open {
                    if let Some(opened_at) = entry.opened_at {
                        if now.duration_since(opened_at) >= CIRCUIT_OPEN_DURATION {
                            entry.state = CircuitState::HalfOpen;
                        }
                    }
                }
                if entry.state == CircuitState::Open {
                    open.push(k.client_name.clone());
                }
            }
        }
        open
    }

    pub fn snapshots(&self) -> Vec<ClientHealthSnapshot> {
        let guard = self.entries.lock().expect("ewma lock poisoned");
        guard
            .iter()
            .map(|(k, v)| ClientHealthSnapshot {
                client_name: k.client_name.clone(),
                workspace_root: k.workspace_root.clone(),
                ewma_ms: v.ewma_ms,
                circuit_state: match v.state {
                    CircuitState::Closed => "closed".to_string(),
                    CircuitState::Open => "open".to_string(),
                    CircuitState::HalfOpen => "half_open".to_string(),
                },
                consecutive_timeouts: v.consecutive_timeouts,
            })
            .collect()
    }
}
