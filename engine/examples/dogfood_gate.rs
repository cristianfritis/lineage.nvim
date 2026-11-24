use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use lineage_engine::adapters::python::PythonAdapter;
use lineage_engine::adapters::rust::RustAdapter;
use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::adapters::LanguageAdapter;
use lineage_engine::cache::path_to_uri;
use lineage_engine::protocol::schema::{
    LspPosition, OffsetEncoding, RpcId, RpcRequest, RpcResponse, SliceDirection, SliceEnvelope,
    SliceParams, UnknownReason, PROTOCOL_VERSION,
};

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    repos: Vec<RepoSpec>,
}

#[derive(Debug, Deserialize)]
struct RepoSpec {
    name: String,
    language: String,
    url: Option<String>,
    commit: Option<String>,
    local_path: Option<String>,
    #[serde(default)]
    subdirs: Vec<String>,
}

#[derive(Debug, Clone)]
struct Probe {
    language: String,
    workspace_root: PathBuf,
    uri: String,
    line: u32,
    character: u32,
    seed_hash: u64,
}

#[derive(Debug, Serialize)]
struct GateReport {
    gates: GatePassStatus,
    stability: StabilityMetrics,
    latency_ms: LatencyMetrics,
    memory_mb: MemoryMetrics,
    coverage_by_language: BTreeMap<String, LanguageCoverage>,
    unknown_reason_histogram: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
struct GatePassStatus {
    g1_stability: bool,
    g2_phase1_p95_lt_50ms: bool,
    g3_tree_from_p95_lt_350ms: bool,
    g4_rss_le_300mb: bool,
    g5b_workspace_stdlib_unknown_le_25pct: bool,
    all_passed: bool,
}

#[derive(Debug, Serialize)]
struct StabilityMetrics {
    total_requests: usize,
    errors_or_malformed: usize,
    unexpected_exit: bool,
}

#[derive(Debug, Serialize)]
struct LatencyMetrics {
    cold_phase1_rtt_p95: u64,
    warm_phase1_rtt_p95: u64,
    warm_phase1_engine_p95: u64,
    warm_tree_from_rtt_p95: u64,
}

#[derive(Debug, Serialize)]
struct MemoryMetrics {
    peak_vmhwm_mb: f64,
    sustained_vmrss_mb: f64,
}

#[derive(Debug, Default, Serialize)]
struct LanguageCoverage {
    probes: usize,
    workspace_stdlib_calls: usize,
    workspace_stdlib_unknown: usize,
    workspace_stdlib_unknown_pct: f64,
    uninstalled_external_calls: usize,
    raw_total_unknown_boundaries: usize,
}

fn percentile(sorted: &[u64], pct: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * pct).ceil() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn sample_process_memory_bytes(pid: u32) -> (u64, u64) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) {
            let mut hwm_kb = 0u64;
            let mut rss_kb = 0u64;
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmHWM:") {
                    hwm_kb = rest
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);
                } else if let Some(rest) = line.strip_prefix("VmRSS:") {
                    rss_kb = rest
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);
                }
            }
            return (hwm_kb * 1024, rss_kb * 1024);
        }
        (0, 0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
        let spid = Pid::from_u32(pid);
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[spid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        let rss = sys.process(spid).map(|p| p.memory()).unwrap_or(0);
        (rss, rss)
    }
}

fn collect_source_files(dir: &Path, adapter: &Arc<dyn LanguageAdapter>, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.')
            || matches!(
                name_str.as_ref(),
                "node_modules" | "target" | "dist" | "build" | "__pycache__" | ".venv"
            )
        {
            continue;
        }
        if path.is_dir() {
            collect_source_files(&path, adapter, out);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if adapter.supports_extension(ext) && !adapter.classify_test_file(&path_to_uri(&path)) {
                out.push(path);
            }
        }
    }
}

fn ensure_repo_checked_out(spec: &RepoSpec, cache_dir: &Path) -> Result<PathBuf, String> {
    if let Some(ref local) = spec.local_path {
        return Ok(PathBuf::from(local));
    }
    let url = spec.url.as_deref().ok_or("missing repo url")?;
    let commit = spec.commit.as_deref().ok_or("missing commit sha")?;
    let repo_dir = cache_dir.join(&spec.name);
    if !repo_dir.join(".git").exists() {
        fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
        let status = Command::new("git")
            .args([
                "clone",
                "--no-checkout",
                "--filter=blob:none",
                url,
                &repo_dir.to_string_lossy(),
            ])
            .status()
            .map_err(|e| format!("git clone failed: {e}"))?;
        if !status.success() {
            return Err(format!("git clone exited with {status}"));
        }
    }
    let head_ok = Command::new("git")
        .args(["-C", &repo_dir.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == commit)
        .unwrap_or(false);
    let has_worktree = fs::read_dir(&repo_dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy() != ".git")
        })
        .unwrap_or(false);
    if !head_ok || !has_worktree {
        if !head_ok {
            let _ = Command::new("git")
                .args([
                    "-C",
                    &repo_dir.to_string_lossy(),
                    "fetch",
                    "--depth",
                    "1",
                    "origin",
                    commit,
                ])
                .status();
        }
        let st = Command::new("git")
            .args(["-C", &repo_dir.to_string_lossy(), "checkout", "-f", commit])
            .status()
            .map_err(|e| format!("git checkout failed: {e}"))?;
        if !st.success() {
            return Err(format!("git checkout {commit} failed"));
        }
    }
    Ok(repo_dir)
}

struct PersistentSidecar {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl PersistentSidecar {
    fn spawn(bin: &Path) -> Result<Self, String> {
        let mut child = Command::new(bin)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn {}: {e}", bin.display()))?;
        let stdin = child.stdin.take().ok_or("missing stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("missing stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        })
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(RpcResponse, u64), String> {
        let id = RpcId::Str(format!("c-{}", self.next_id));
        self.next_id += 1;
        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(id.clone()),
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        writeln!(self.stdin, "{line}").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;

        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.stdout.read_line(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("sidecar closed stdout unexpectedly".to_string());
            }
            let resp: RpcResponse =
                serde_json::from_str(buf.trim()).map_err(|e| format!("malformed frame: {e}"))?;
            if resp.id == id {
                let rtt = t0.elapsed().as_millis() as u64;
                return Ok((resp, rtt));
            }
        }
    }

    fn shutdown(mut self) -> bool {
        let _ = self.call("shutdown", serde_json::json!({}));
        self.child.wait().map(|s| s.success()).unwrap_or(false)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut manifest_path = PathBuf::from("benchmarks/corpus_manifest.json");
    let mut engine_bin = PathBuf::from("engine/target/release/lineage-engine");
    let mut cache_dir = PathBuf::from(".dogfood-cache");
    let mut report_path = PathBuf::from("benchmarks/reports/stage_a_report.json");
    let mut max_probes_per_repo = 40usize;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" if i + 1 < args.len() => {
                manifest_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--engine-bin" if i + 1 < args.len() => {
                engine_bin = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--cache-dir" if i + 1 < args.len() => {
                cache_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--report" if i + 1 < args.len() => {
                report_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--max-probes-per-repo" if i + 1 < args.len() => {
                max_probes_per_repo = args[i + 1].parse().unwrap_or(40);
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let manifest_raw = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("Failed to read manifest {}: {e}", manifest_path.display()));
    let manifest: CorpusManifest =
        serde_json::from_str(&manifest_raw).expect("Invalid corpus_manifest.json");

    let adapters: HashMap<&'static str, Arc<dyn LanguageAdapter>> = HashMap::from([
        (
            "typescript",
            Arc::new(TypeScriptAdapter::new()) as Arc<dyn LanguageAdapter>,
        ),
        (
            "python",
            Arc::new(PythonAdapter::new()) as Arc<dyn LanguageAdapter>,
        ),
        (
            "rust",
            Arc::new(RustAdapter::new()) as Arc<dyn LanguageAdapter>,
        ),
    ]);

    let mut all_probes = Vec::new();
    for spec in &manifest.repos {
        let repo_root = ensure_repo_checked_out(spec, &cache_dir)
            .unwrap_or_else(|e| panic!("Failed to prepare repo {}: {e}", spec.name));
        let abs_root = fs::canonicalize(&repo_root).unwrap_or(repo_root);
        let adapter = adapters
            .get(spec.language.as_str())
            .expect("Unsupported language in manifest");

        let mut files = Vec::new();
        if spec.subdirs.is_empty() {
            collect_source_files(&abs_root, adapter, &mut files);
        } else {
            for sub in &spec.subdirs {
                let sub_path = abs_root.join(sub);
                if sub_path.is_dir() {
                    collect_source_files(&sub_path, adapter, &mut files);
                }
            }
        }
        files.sort();

        let mut repo_probes = Vec::new();
        for fpath in files {
            let Ok(text) = fs::read_to_string(&fpath) else {
                continue;
            };
            if text.len() > 512 * 1024 {
                continue;
            }
            let uri = path_to_uri(&fpath);
            let skel = adapter.parse_skeleton(&uri, &text);
            let rel = fpath
                .strip_prefix(&abs_root)
                .unwrap_or(&fpath)
                .to_string_lossy();
            for func in &skel.functions {
                if func.name == "<module>" {
                    continue;
                }
                let target_line = if func.range.end.line > func.range.start.line {
                    func.range.start.line + 1
                } else {
                    func.range.start.line
                };
                let seed_input = format!("{}:{}:{}", spec.name, rel, func.symbol_id);
                let seed_hash = xxh3_64(seed_input.as_bytes());
                repo_probes.push(Probe {
                    language: spec.language.clone(),
                    workspace_root: abs_root.clone(),
                    uri: uri.clone(),
                    line: target_line,
                    character: func.range.start.col,
                    seed_hash,
                });
            }
        }
        repo_probes.sort_by_key(|p| p.seed_hash);
        repo_probes.truncate(max_probes_per_repo);
        assert!(
            !repo_probes.is_empty(),
            "No probes extracted for repo {} ({})",
            spec.name,
            spec.language
        );
        println!(
            "[dogfood_gate] {} ({}): selected {} probes",
            spec.name,
            spec.language,
            repo_probes.len()
        );
        all_probes.extend(repo_probes);
    }

    let mut sidecar =
        PersistentSidecar::spawn(&engine_bin).expect("Failed to spawn lineage-engine serve");
    let (init_resp, _) = sidecar
        .call(
            "initialize",
            serde_json::json!({ "client_name": "dogfood_gate", "protocol_version": PROTOCOL_VERSION }),
        )
        .expect("initialize failed");
    assert!(init_resp.error.is_none(), "initialize returned error");

    let mut errors_or_malformed = 0usize;
    let mut total_requests = 0usize;
    let mut cold_rtts = Vec::new();
    let mut warm_rtts = Vec::new();
    let mut warm_engine_ms = Vec::new();
    let mut warm_tree_rtts = Vec::new();
    let mut peak_hwm_bytes = 0u64;
    let mut coverage_by_lang: BTreeMap<String, LanguageCoverage> = BTreeMap::new();
    let mut reason_hist: BTreeMap<String, usize> = BTreeMap::new();

    for pass in 0..2 {
        let is_warm = pass == 1;
        for (idx, probe) in all_probes.iter().enumerate() {
            let params = SliceParams {
                uri: probe.uri.clone(),
                position: LspPosition {
                    line: probe.line,
                    character: probe.character,
                },
                offset_encoding: OffsetEncoding::Utf8,
                direction: SliceDirection::To,
                include_graph: false,
                include_focus: false,
                include_tree: false,
                include_snippets: false,
                exclude_tests: true,
                query_id: Some((idx + 1) as u64),
                lsp_attached: false,
                lsp_client_name: None,
                workspace_root: Some(probe.workspace_root.to_string_lossy().to_string()),
                lsp_timeout_ms: None,
            };
            total_requests += 1;
            match sidecar.call("lineage/slice", serde_json::to_value(&params).unwrap()) {
                Ok((resp, rtt)) => {
                    if resp.error.is_some() {
                        errors_or_malformed += 1;
                        continue;
                    }
                    if let Some(res) = resp.result {
                        match serde_json::from_value::<SliceEnvelope>(res) {
                            Ok(env) => {
                                if is_warm {
                                    warm_rtts.push(rtt);
                                    warm_engine_ms.push(env.stats.phase1_ms);
                                    let entry =
                                        coverage_by_lang.entry(probe.language.clone()).or_default();
                                    entry.probes += 1;
                                    entry.workspace_stdlib_calls +=
                                        env.stats.total_external_generic_calls;
                                    entry.workspace_stdlib_unknown +=
                                        env.stats.unknown_external_generic_calls;
                                    entry.uninstalled_external_calls +=
                                        env.stats.uninstalled_external_calls;
                                    if let Some(card) = &env.projections.hover_card {
                                        entry.raw_total_unknown_boundaries +=
                                            card.unknown_boundaries.len();
                                        for b in &card.unknown_boundaries {
                                            let key = format!("{:?}", b.reason);
                                            *reason_hist.entry(key).or_insert(0) += 1;
                                            let _ = UnknownReason::ExternalUninstalled;
                                        }
                                    }
                                } else {
                                    cold_rtts.push(rtt);
                                }
                            }
                            Err(_) => errors_or_malformed += 1,
                        }
                    } else {
                        errors_or_malformed += 1;
                    }
                }
                Err(_) => errors_or_malformed += 1,
            }

            if is_warm && idx % 5 == 0 {
                let tree_params = SliceParams {
                    uri: probe.uri.clone(),
                    position: LspPosition {
                        line: probe.line,
                        character: probe.character,
                    },
                    offset_encoding: OffsetEncoding::Utf8,
                    direction: SliceDirection::From,
                    include_graph: false,
                    include_focus: false,
                    include_tree: true,
                    include_snippets: false,
                    exclude_tests: true,
                    query_id: Some(10_000 + idx as u64),
                    lsp_attached: false,
                    lsp_client_name: None,
                    workspace_root: Some(probe.workspace_root.to_string_lossy().to_string()),
                    lsp_timeout_ms: None,
                };
                total_requests += 1;
                match sidecar.call("lineage/slice", serde_json::to_value(&tree_params).unwrap()) {
                    Ok((resp, rtt)) => {
                        if resp.error.is_some() {
                            errors_or_malformed += 1;
                        } else {
                            warm_tree_rtts.push(rtt);
                        }
                    }
                    Err(_) => errors_or_malformed += 1,
                }
            }

            if idx % 20 == 0 {
                let (hwm, _) = sample_process_memory_bytes(sidecar.pid());
                peak_hwm_bytes = peak_hwm_bytes.max(hwm);
            }
        }
    }

    let (hwm_final, sustained_rss_bytes) = sample_process_memory_bytes(sidecar.pid());
    peak_hwm_bytes = peak_hwm_bytes.max(hwm_final);
    let clean_exit = sidecar.shutdown();

    cold_rtts.sort_unstable();
    warm_rtts.sort_unstable();
    warm_engine_ms.sort_unstable();
    warm_tree_rtts.sort_unstable();

    let cold_p95 = percentile(&cold_rtts, 0.95);
    let warm_rtt_p95 = percentile(&warm_rtts, 0.95);
    let warm_eng_p95 = percentile(&warm_engine_ms, 0.95);
    let warm_tree_p95 = percentile(&warm_tree_rtts, 0.95);
    let peak_mb = peak_hwm_bytes as f64 / (1024.0 * 1024.0);
    let sustained_mb = sustained_rss_bytes as f64 / (1024.0 * 1024.0);

    let mut g5b_ok = true;
    for cov in coverage_by_lang.values_mut() {
        if cov.workspace_stdlib_calls > 0 {
            cov.workspace_stdlib_unknown_pct =
                (cov.workspace_stdlib_unknown as f64 / cov.workspace_stdlib_calls as f64) * 100.0;
        }
        if cov.workspace_stdlib_calls >= 100 && cov.workspace_stdlib_unknown_pct > 25.0 {
            g5b_ok = false;
        }
    }

    let g1 = errors_or_malformed == 0 && clean_exit;
    let g2 = warm_rtt_p95 < 50 && warm_eng_p95 < 50;
    let g3 = warm_tree_p95 < 350;
    let g4 = peak_mb <= 300.0 && sustained_mb <= 300.0;
    let all_passed = g1 && g2 && g3 && g4 && g5b_ok;

    let report = GateReport {
        gates: GatePassStatus {
            g1_stability: g1,
            g2_phase1_p95_lt_50ms: g2,
            g3_tree_from_p95_lt_350ms: g3,
            g4_rss_le_300mb: g4,
            g5b_workspace_stdlib_unknown_le_25pct: g5b_ok,
            all_passed,
        },
        stability: StabilityMetrics {
            total_requests,
            errors_or_malformed,
            unexpected_exit: !clean_exit,
        },
        latency_ms: LatencyMetrics {
            cold_phase1_rtt_p95: cold_p95,
            warm_phase1_rtt_p95: warm_rtt_p95,
            warm_phase1_engine_p95: warm_eng_p95,
            warm_tree_from_rtt_p95: warm_tree_p95,
        },
        memory_mb: MemoryMetrics {
            peak_vmhwm_mb: peak_mb,
            sustained_vmrss_mb: sustained_mb,
        },
        coverage_by_language: coverage_by_lang,
        unknown_reason_histogram: reason_hist,
    };

    if let Some(parent) = report_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap())
        .expect("Failed to write Stage A report");

    println!("\n=== Stage A Layer 1 Gate Summary ===");
    println!(
        "G1 Stability:   {} (requests={}, errors={}, clean_exit={})",
        g1, total_requests, errors_or_malformed, clean_exit
    );
    println!(
        "G2 Phase 1 p95: {} (warm_rtt={}ms, warm_engine={}ms, cold_rtt={}ms; limit <50ms)",
        g2, warm_rtt_p95, warm_eng_p95, cold_p95
    );
    println!(
        "G3 Tree/From:   {} (warm_rtt_p95={}ms; limit <350ms)",
        g3, warm_tree_p95
    );
    println!(
        "G4 OS Memory:   {} (peak_VmHWM={:.1}MB, sustained_VmRSS={:.1}MB; limit <=300MB)",
        g4, peak_mb, sustained_mb
    );
    println!("G5b Coverage:   {}", g5b_ok);
    for (lang, cov) in &report.coverage_by_language {
        println!(
            "  - {}: ws+stdlib_unknown={}/{} ({:.1}%), uninstalled_ext={}, raw_unknown={}",
            lang,
            cov.workspace_stdlib_unknown,
            cov.workspace_stdlib_calls,
            cov.workspace_stdlib_unknown_pct,
            cov.uninstalled_external_calls,
            cov.raw_total_unknown_boundaries
        );
    }
    println!(
        "UnknownReason histogram: {:?}",
        report.unknown_reason_histogram
    );
    println!("Report written to {}", report_path.display());

    if !all_passed {
        std::process::exit(1);
    }
}
