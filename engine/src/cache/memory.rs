use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use moka::sync::Cache;
use sysinfo::{MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

use crate::ir::{FileSkeleton, FunctionIr};
use crate::protocol::schema::UnknownReason;

pub const MAX_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_LINE_BYTES: usize = 4096;
pub const MAX_DIRTY_BUFFERS: usize = 32;
pub const MAX_DIRTY_BYTES: usize = 16 * 1024 * 1024;

pub fn compute_cache_cap_bytes() -> u64 {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
    );
    sys.refresh_memory();
    let total_ram = sys.total_memory();
    let three_pct = total_ram.saturating_mul(3) / 100;
    let min_cap = 64 * 1024 * 1024;
    let max_cap = 140 * 1024 * 1024;
    three_pct.clamp(min_cap, max_cap)
}

pub fn current_process_rss_bytes() -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    sys.process(pid).map(|p| p.memory())
}

pub fn parse_worker_cap() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    (cpus / 2).clamp(1, 4)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskStamp {
    pub mtime: Option<SystemTime>,
    pub len: u64,
}

#[derive(Debug, Clone)]
pub enum GuardError {
    NonRegularFile,
    FileTooLarge,
    IoError(String),
}

impl GuardError {
    pub fn to_unknown_reason(&self) -> UnknownReason {
        match self {
            GuardError::NonRegularFile => UnknownReason::NonRegularFile,
            GuardError::FileTooLarge => UnknownReason::FileTooLarge,
            GuardError::IoError(_) => UnknownReason::NonRegularFile,
        }
    }
}

pub fn uri_to_path(uri: &str) -> PathBuf {
    if let Some(stripped) = uri.strip_prefix("file://") {
        PathBuf::from(stripped)
    } else {
        PathBuf::from(uri)
    }
}

pub fn path_to_uri(path: &Path) -> String {
    format!("file://{}", path.to_string_lossy())
}

pub fn check_text_guards(text: &str) -> Result<(), GuardError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(GuardError::FileTooLarge);
    }
    if text.split('\n').any(|line| line.len() > MAX_LINE_BYTES) {
        return Err(GuardError::FileTooLarge);
    }
    Ok(())
}

pub fn read_file_guarded(path: &Path) -> Result<(String, DiskStamp), GuardError> {
    let path_str = path.to_string_lossy();
    if path_str.starts_with("/proc/")
        || path_str == "/proc"
        || path_str.starts_with("/sys/")
        || path_str == "/sys"
    {
        return Err(GuardError::NonRegularFile);
    }

    let meta = fs::metadata(path).map_err(|e| GuardError::IoError(e.to_string()))?;
    if !meta.is_file() {
        return Err(GuardError::NonRegularFile);
    }
    if meta.len() as usize > MAX_FILE_BYTES {
        return Err(GuardError::FileTooLarge);
    }

    let text = fs::read_to_string(path).map_err(|e| GuardError::IoError(e.to_string()))?;
    check_text_guards(&text)?;

    let stamp = DiskStamp {
        mtime: meta.modified().ok(),
        len: meta.len(),
    };
    Ok((text, stamp))
}

#[derive(Debug, Clone)]
pub struct CachedSkeleton {
    pub skeleton: Arc<FileSkeleton>,
    pub stamp: Option<DiskStamp>,
    pub approx_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionIrKey {
    pub uri: String,
    pub symbol_id: String,
    pub content_hash: u64,
}

#[derive(Debug, Clone)]
pub struct CachedFunctionIr {
    pub ir: Arc<FunctionIr>,
    pub approx_bytes: u32,
}

/// Byte-weighted LRU cache for FileSkeletons and lowered FunctionIr.
pub struct FileIrCache {
    pub cap_bytes: u64,
    skeletons: Cache<String, CachedSkeleton>,
    functions: Cache<FunctionIrKey, CachedFunctionIr>,
}

impl FileIrCache {
    pub fn new(cap_bytes: u64) -> Self {
        let skel_cap = cap_bytes / 4;
        let fn_cap = cap_bytes.saturating_sub(skel_cap);

        let skeletons = Cache::builder()
            .max_capacity(skel_cap)
            .weigher(|_k: &String, v: &CachedSkeleton| v.approx_bytes.max(64))
            .build();

        let functions = Cache::builder()
            .max_capacity(fn_cap)
            .weigher(|_k: &FunctionIrKey, v: &CachedFunctionIr| v.approx_bytes.max(128))
            .build();

        Self {
            cap_bytes,
            skeletons,
            functions,
        }
    }

    pub fn get_skeleton(
        &self,
        uri: &str,
        current_stamp: Option<&DiskStamp>,
    ) -> Option<Arc<FileSkeleton>> {
        let entry = self.skeletons.get(uri)?;
        match (entry.stamp.as_ref(), current_stamp) {
            (Some(cached), Some(disk)) if cached == disk => Some(entry.skeleton),
            (None, None) => Some(entry.skeleton),
            _ => {
                self.skeletons.invalidate(uri);
                None
            }
        }
    }

    pub fn insert_skeleton(
        &self,
        uri: String,
        skeleton: Arc<FileSkeleton>,
        stamp: Option<DiskStamp>,
    ) {
        let approx_bytes = (uri.len()
            + skeleton.functions.len() * 128
            + skeleton.imports.len() * 96
            + skeleton.exports.len() * 96
            + 256) as u32;
        self.skeletons.insert(
            uri,
            CachedSkeleton {
                skeleton,
                stamp,
                approx_bytes,
            },
        );
    }

    pub fn get_function(
        &self,
        uri: &str,
        symbol_id: &str,
        content_hash: u64,
    ) -> Option<Arc<FunctionIr>> {
        let key = FunctionIrKey {
            uri: uri.to_string(),
            symbol_id: symbol_id.to_string(),
            content_hash,
        };
        self.functions.get(&key).map(|e| e.ir)
    }

    pub fn insert_function(
        &self,
        uri: String,
        symbol_id: String,
        content_hash: u64,
        ir: Arc<FunctionIr>,
    ) {
        let approx_bytes = (ir.insts.len() * 160 + ir.blocks.len() * 96 + 256) as u32;
        let key = FunctionIrKey {
            uri,
            symbol_id,
            content_hash,
        };
        self.functions
            .insert(key, CachedFunctionIr { ir, approx_bytes });
    }

    pub fn invalidate_uri(&self, uri: &str) {
        self.skeletons.invalidate(uri);
    }

    pub fn approx_weighted_bytes(&self) -> u64 {
        self.skeletons.run_pending_tasks();
        self.functions.run_pending_tasks();
        self.skeletons.weighted_size() + self.functions.weighted_size()
    }
}

#[derive(Debug, Clone)]
pub struct DirtyBufferEntry {
    pub changedtick: i64,
    pub text: Arc<String>,
    pub content_hash: u64,
}

/// Bounded LRU overlay for unsaved Neovim buffers (max 32 buffers / 16 MB).
#[derive(Default)]
pub struct DirtyBufferOverlay {
    inner: Mutex<DirtyOverlayInner>,
}

#[derive(Default)]
struct DirtyOverlayInner {
    entries: HashMap<String, DirtyBufferEntry>,
    lru_order: VecDeque<String>,
    total_bytes: usize,
}

impl DirtyBufferOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, uri: String, changedtick: i64, text: String) -> Result<(), GuardError> {
        check_text_guards(&text)?;
        let content_hash = xxhash_rust::xxh3::xxh3_64(text.as_bytes());
        let new_len = text.len();
        let entry = DirtyBufferEntry {
            changedtick,
            text: Arc::new(text),
            content_hash,
        };

        let mut guard = self.inner.lock().expect("dirty overlay lock poisoned");
        if let Some(old) = guard.entries.remove(&uri) {
            guard.total_bytes = guard.total_bytes.saturating_sub(old.text.len());
            guard.lru_order.retain(|u| u != &uri);
        }

        while (guard.entries.len() >= MAX_DIRTY_BUFFERS
            || guard.total_bytes + new_len > MAX_DIRTY_BYTES)
            && !guard.lru_order.is_empty()
        {
            if let Some(evict_uri) = guard.lru_order.pop_front() {
                if let Some(evicted) = guard.entries.remove(&evict_uri) {
                    guard.total_bytes = guard.total_bytes.saturating_sub(evicted.text.len());
                }
            }
        }

        guard.total_bytes += new_len;
        guard.entries.insert(uri.clone(), entry);
        guard.lru_order.push_back(uri);
        Ok(())
    }

    pub fn remove(&self, uri: &str) {
        let mut guard = self.inner.lock().expect("dirty overlay lock poisoned");
        if let Some(old) = guard.entries.remove(uri) {
            guard.total_bytes = guard.total_bytes.saturating_sub(old.text.len());
            guard.lru_order.retain(|u| u != uri);
        }
    }

    pub fn get(&self, uri: &str) -> Option<DirtyBufferEntry> {
        let mut guard = self.inner.lock().expect("dirty overlay lock poisoned");
        let entry = guard.entries.get(uri)?.clone();
        guard.lru_order.retain(|u| u != uri);
        guard.lru_order.push_back(uri.to_string());
        Some(entry)
    }

    pub fn stats(&self) -> (usize, u64) {
        let guard = self.inner.lock().expect("dirty overlay lock poisoned");
        (guard.entries.len(), guard.total_bytes as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_cap_bytes_within_calibrated_bounds() {
        let cap = compute_cache_cap_bytes();
        assert!(cap >= 64 * 1024 * 1024, "cap too small: {cap}");
        assert!(cap <= 140 * 1024 * 1024, "cap too large: {cap}");
    }
}
