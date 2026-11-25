use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

#[cfg(all(target_env = "musl", feature = "mimalloc"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use clap::Parser;
use tracing_appender::rolling;
use tracing_subscriber::fmt;
use tracing_subscriber::EnvFilter;

use lineage_engine::adapters::go::GoAdapter;
use lineage_engine::adapters::python::PythonAdapter;
use lineage_engine::adapters::rust::RustAdapter;
use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::adapters::{LanguageAdapter, StubAdapter};
use lineage_engine::cache::{
    compute_cache_cap_bytes, path_to_uri, DirtyBufferOverlay, FileIrCache,
};
use lineage_engine::cli::{Cli, CliDirection, Commands, ExportCliFormat, SliceCliFormat};
use lineage_engine::export::{render_export, ExportOptions};
use lineage_engine::protocol::schema::{
    ExportFormat, ExportSourceMode, LspPosition, OffsetEncoding, Phase2Status, SliceDirection,
    SliceEnvelope, SliceParams,
};
use lineage_engine::protocol::transport::{serve_stdio, DEFAULT_EXPORT_LSP_TIMEOUT_MS};
use lineage_engine::resolver::{FixtureLspProvider, NullLspProvider, SymbolResolver};
use lineage_engine::slicer::SlicerEngine;

fn init_logging(log_dir: Option<&PathBuf>) -> Option<String> {
    let dir = log_dir
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("lineage-engine-logs"));
    let _ = std::fs::create_dir_all(&dir);
    let file_appender = rolling::daily(&dir, "lineage-engine.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // Leak guard for process lifetime so rotating logs flush on exit.
    Box::leak(Box::new(guard));
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt()
        .with_writer(non_blocking)
        .with_env_filter(env_filter)
        .with_ansi(false)
        .try_init();
    Some(dir.join("lineage-engine.log").to_string_lossy().to_string())
}

fn default_adapters() -> Vec<Arc<dyn LanguageAdapter>> {
    vec![
        Arc::new(TypeScriptAdapter::new()),
        Arc::new(PythonAdapter::new()),
        Arc::new(RustAdapter::new()),
        Arc::new(GoAdapter::new()),
        Arc::new(StubAdapter),
    ]
}

fn build_engine(resolver: Arc<dyn SymbolResolver>) -> Arc<SlicerEngine> {
    let cap = compute_cache_cap_bytes();
    let cache = Arc::new(FileIrCache::new(cap));
    let dirty = Arc::new(DirtyBufferOverlay::new());
    Arc::new(SlicerEngine::new(
        cache,
        dirty,
        resolver,
        default_adapters(),
    ))
}

fn parse_target(target: &str) -> Result<(PathBuf, u32, u32), String> {
    let parts: Vec<&str> = target.rsplitn(3, ':').collect();
    if parts.len() != 3 {
        return Err(format!("Expected <file>:<line>:<col>, got '{target}'"));
    }
    let col: u32 = parts[0].parse().map_err(|_| "Invalid column".to_string())?;
    let line: u32 = parts[1].parse().map_err(|_| "Invalid line".to_string())?;
    let file = PathBuf::from(parts[2]);
    let abs = if file.is_absolute() {
        file
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(file)
    };
    Ok((abs, line.saturating_sub(1), col))
}

fn to_slice_direction(dir: CliDirection) -> SliceDirection {
    match dir {
        CliDirection::To => SliceDirection::To,
        CliDirection::From => SliceDirection::From,
        CliDirection::Both => SliceDirection::Both,
    }
}

fn resolve_workspace_root(explicit: Option<PathBuf>, target_file: Option<&Path>) -> PathBuf {
    if let Some(dir) = explicit {
        return std::fs::canonicalize(&dir).unwrap_or(dir);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let start = target_file.and_then(|f| f.parent()).unwrap_or(&cwd);
    for ancestor in start.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
    }
    cwd
}

fn build_resolver(lsp_fixture: Option<&PathBuf>) -> Arc<dyn SymbolResolver> {
    if let Some(fixture_path) = lsp_fixture {
        match FixtureLspProvider::from_json_file(fixture_path) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                eprintln!("Fixture error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        Arc::new(NullLspProvider)
    }
}

fn main() {
    let cli = Cli::parse();
    let log_path = init_logging(cli.log_dir.as_ref());

    match cli.command {
        Commands::Serve => {
            tracing::info!("starting lineage-engine stdio server");
            let engine = build_engine(Arc::new(NullLspProvider));
            serve_stdio(engine, log_path);
        }
        Commands::DumpIr { file } => {
            let abs = std::fs::canonicalize(&file).unwrap_or(file);
            let uri = path_to_uri(&abs);
            let engine = build_engine(Arc::new(NullLspProvider));
            match engine.dump_ir_text(&uri) {
                Ok(out) => print!("{out}"),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::DumpCfg { dot: _, file } => {
            let abs = std::fs::canonicalize(&file).unwrap_or(file);
            let uri = path_to_uri(&abs);
            let engine = build_engine(Arc::new(NullLspProvider));
            match engine.dump_cfg_dot(&uri) {
                Ok(out) => print!("{out}"),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Slice {
            target,
            direction,
            format,
            trace,
            lsp_fixture,
            include_graph,
        } => {
            let (file, line, col) = match parse_target(&target) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            let has_fixture = lsp_fixture.is_some();
            let resolver = build_resolver(lsp_fixture.as_ref());
            let ws_root = resolve_workspace_root(None, Some(&file));

            let engine = build_engine(resolver);
            let uri = path_to_uri(&file);
            let params = SliceParams {
                uri,
                position: LspPosition {
                    line,
                    character: col,
                },
                offset_encoding: OffsetEncoding::Utf8,
                direction: to_slice_direction(direction),
                include_graph: include_graph || matches!(format, SliceCliFormat::Dot),
                include_focus: false,
                include_tree: false,
                include_snippets: !matches!(format, SliceCliFormat::Dot),
                exclude_tests: false,
                query_id: Some(1),
                lsp_attached: has_fixture,
                lsp_client_name: None,
                workspace_root: Some(ws_root.to_string_lossy().to_string()),
                lsp_timeout_ms: has_fixture.then_some(DEFAULT_EXPORT_LSP_TIMEOUT_MS),
            };
            let cancel = AtomicBool::new(false);
            match engine.slice_phase1_with_snapshot(&params, "cli-1", &cancel) {
                Ok((mut env, snap_graph, phase2_opt)) => {
                    let mut p2_status = Phase2Status::NotNeeded;
                    let mut p2_wait_ms = 0u64;
                    if let Some(mut ctx) = phase2_opt {
                        ctx.phase2_ceiling_ms = DEFAULT_EXPORT_LSP_TIMEOUT_MS;
                        let p2_start = Instant::now();
                        if let Some((mut delta_env, full_graph, status)) =
                            engine.execute_phase2_with_status(ctx, &cancel)
                        {
                            p2_wait_ms = p2_start.elapsed().as_millis() as u64;
                            p2_status = status;
                            if params.include_graph {
                                delta_env.graph = Some(full_graph);
                            }
                            env = delta_env;
                        }
                    } else if env.graph.is_none() && params.include_graph {
                        env.graph = Some(snap_graph);
                    }

                    match format {
                        SliceCliFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&env).unwrap());
                        }
                        SliceCliFormat::Text => {
                            if let Some(card) = &env.projections.hover_card {
                                println!("{}", card.verdict_line);
                                for item in &card.phase1_items {
                                    println!(
                                        "  [{}] {}:{} - {}",
                                        item.index,
                                        item.uri,
                                        item.range.start.line + 1,
                                        item.label
                                    );
                                }
                                for b in &card.unknown_boundaries {
                                    println!("  [unverified] {} ({:?})", b.symbol, b.reason);
                                }
                            }
                        }
                        SliceCliFormat::Dot => {
                            let opts = ExportOptions::for_format(
                                ExportFormat::Dot,
                                Some(ws_root),
                                false,
                                None,
                                false,
                                has_fixture,
                                DEFAULT_EXPORT_LSP_TIMEOUT_MS,
                                p2_status,
                                ExportSourceMode::FreshSlice,
                                p2_wait_ms,
                                None,
                            );
                            let out = render_export(&env, &opts, "cli-1");
                            print!("{}", out.result.content);
                        }
                    }
                    if trace {
                        eprintln!(
                            "[trace] phase1_ms={} phase2_ms={} nodes_visited={} contracted={}",
                            env.stats.phase1_ms,
                            env.stats.phase2_ms,
                            env.stats.nodes_visited,
                            env.stats.contracted_chains
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Slice error: {}", e.message);
                    std::process::exit(1);
                }
            }
        }
        Commands::Export {
            target,
            from_envelope,
            format,
            direction,
            workspace_root,
            exclude_tests,
            null_only,
            no_lsp,
            lsp_fixture,
            snippets,
            no_snippets,
            absolute_paths,
            out,
        } => {
            let export_format = match format {
                ExportCliFormat::Mermaid => ExportFormat::Mermaid,
                ExportCliFormat::Dot => ExportFormat::Dot,
                ExportCliFormat::Html => ExportFormat::Html,
            };
            let include_snippets = if snippets {
                Some(true)
            } else if no_snippets {
                Some(false)
            } else {
                None
            };

            let (env, ws_root, lsp_attached, p2_status, p2_wait_ms) =
                if let Some(env_path) = from_envelope {
                    let raw = if env_path.to_str() == Some("-") {
                        let mut s = String::new();
                        if let Err(e) = std::io::stdin().read_to_string(&mut s) {
                            eprintln!("Stdin error: {e}");
                            std::process::exit(1);
                        }
                        s
                    } else {
                        match std::fs::read_to_string(&env_path) {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!("Failed to read {}: {e}", env_path.display());
                                std::process::exit(1);
                            }
                        }
                    };
                    let env: SliceEnvelope = match serde_json::from_str(&raw) {
                        Ok(e) => e,
                        Err(e) => {
                            eprintln!("Invalid SliceEnvelope JSON: {e}");
                            std::process::exit(1);
                        }
                    };
                    let ws_root = resolve_workspace_root(workspace_root, None);
                    let p2_status = if no_lsp {
                        Phase2Status::Skipped
                    } else if env.stats.phase2_ms > 0 {
                        Phase2Status::Complete
                    } else {
                        Phase2Status::NotNeeded
                    };
                    let p2_wait_ms = env.stats.phase2_ms;
                    (env, ws_root, !no_lsp, p2_status, p2_wait_ms)
                } else {
                    let target_str = target.expect("clap requires target without --from-envelope");
                    let (file, line, col) = match parse_target(&target_str) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    };
                    let ws_root = resolve_workspace_root(workspace_root, Some(&file));
                    let use_lsp = lsp_fixture.is_some() && !no_lsp;
                    let resolver = if use_lsp {
                        build_resolver(lsp_fixture.as_ref())
                    } else {
                        Arc::new(NullLspProvider)
                    };
                    let engine = build_engine(resolver);
                    let uri = path_to_uri(&file);
                    let params = SliceParams {
                        uri,
                        position: LspPosition {
                            line,
                            character: col,
                        },
                        offset_encoding: OffsetEncoding::Utf8,
                        direction: to_slice_direction(direction),
                        include_graph: false,
                        include_focus: false,
                        include_tree: false,
                        include_snippets: include_snippets.unwrap_or(false),
                        exclude_tests,
                        query_id: Some(1),
                        lsp_attached: use_lsp,
                        lsp_client_name: None,
                        workspace_root: Some(ws_root.to_string_lossy().to_string()),
                        lsp_timeout_ms: Some(DEFAULT_EXPORT_LSP_TIMEOUT_MS),
                    };
                    let cancel = AtomicBool::new(false);
                    match engine.slice_phase1_with_snapshot(&params, "cli-export", &cancel) {
                        Ok((mut env, snap_graph, phase2_opt)) => match (no_lsp, phase2_opt) {
                            (true, _) | (false, None) => {
                                env.graph = Some(snap_graph);
                                let status = if no_lsp {
                                    Phase2Status::Skipped
                                } else {
                                    Phase2Status::NotNeeded
                                };
                                (env, ws_root, use_lsp, status, 0)
                            }
                            (false, Some(mut ctx)) => {
                                ctx.phase2_ceiling_ms = DEFAULT_EXPORT_LSP_TIMEOUT_MS;
                                let p2_start = Instant::now();
                                match engine.execute_phase2_with_status(ctx, &cancel) {
                                    Some((mut delta_env, full_graph, status)) => {
                                        let wait_ms = p2_start.elapsed().as_millis() as u64;
                                        delta_env.graph = Some(full_graph);
                                        (delta_env, ws_root, use_lsp, status, wait_ms)
                                    }
                                    None => {
                                        let wait_ms = p2_start.elapsed().as_millis() as u64;
                                        env.graph = Some(snap_graph);
                                        (env, ws_root, use_lsp, Phase2Status::TimedOut, wait_ms)
                                    }
                                }
                            }
                        },
                        Err(e) => {
                            eprintln!("Export slice error: {}", e.message);
                            std::process::exit(1);
                        }
                    }
                };

            let mut opts = ExportOptions::for_format(
                export_format,
                Some(ws_root),
                null_only,
                include_snippets,
                absolute_paths,
                lsp_attached,
                DEFAULT_EXPORT_LSP_TIMEOUT_MS,
                p2_status,
                ExportSourceMode::FreshSlice,
                p2_wait_ms,
                None,
            );
            opts.exclude_tests = exclude_tests;
            let output = render_export(&env, &opts, "cli-export");
            if let Some(out_path) = out {
                if let Err(e) = std::fs::write(&out_path, output.result.content.as_bytes()) {
                    eprintln!("Failed to write {}: {e}", out_path.display());
                    std::process::exit(1);
                }
            } else {
                print!("{}", output.result.content);
            }
        }
    }
}
