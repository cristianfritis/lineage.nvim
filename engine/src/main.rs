use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use clap::Parser;
use tracing_appender::rolling;
use tracing_subscriber::fmt;
use tracing_subscriber::EnvFilter;

use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::adapters::{LanguageAdapter, StubAdapter};
use lineage_engine::cache::{
    compute_cache_cap_bytes, path_to_uri, DirtyBufferOverlay, FileIrCache,
};
use lineage_engine::cli::{Cli, CliDirection, CliFormat, Commands};
use lineage_engine::protocol::schema::{LspPosition, OffsetEncoding, SliceDirection, SliceParams};
use lineage_engine::protocol::transport::serve_stdio;
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
    let _ = fmt()
        .with_writer(non_blocking)
        .with_env_filter(EnvFilter::from_default_env())
        .with_ansi(false)
        .try_init();
    Some(dir.join("lineage-engine.log").to_string_lossy().to_string())
}

fn default_adapters() -> Vec<Arc<dyn LanguageAdapter>> {
    vec![Arc::new(TypeScriptAdapter::new()), Arc::new(StubAdapter)]
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
            let resolver: Arc<dyn SymbolResolver> = if let Some(fixture_path) = lsp_fixture {
                match FixtureLspProvider::from_json_file(&fixture_path) {
                    Ok(p) => Arc::new(p),
                    Err(e) => {
                        eprintln!("Fixture error: {e}");
                        std::process::exit(1);
                    }
                }
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
                direction: match direction {
                    CliDirection::To => SliceDirection::To,
                    CliDirection::From => SliceDirection::From,
                },
                include_graph: include_graph || matches!(format, CliFormat::Dot),
                include_snippets: true,
                exclude_tests: false,
                query_id: Some(1),
                lsp_attached: false,
                lsp_client_name: None,
                workspace_root: file.parent().map(|p| p.to_string_lossy().to_string()),
            };
            let cancel = AtomicBool::new(false);
            match engine.slice_query(&params, "cli-1", &cancel, None::<fn(_)>) {
                Ok(env) => {
                    match format {
                        CliFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&env).unwrap());
                        }
                        CliFormat::Text => {
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
                        CliFormat::Dot => {
                            println!("digraph slice {{");
                            if let Some(g) = &env.graph {
                                for n in &g.nodes {
                                    println!(
                                        "  \"{}\" [label=\"{}\"];",
                                        n.id,
                                        n.label.replace('"', "\\\"")
                                    );
                                }
                                for e in &g.edges {
                                    println!("  \"{}\" -> \"{}\";", e.from, e.to);
                                }
                            }
                            println!("}}");
                        }
                    }
                    if trace {
                        eprintln!(
                            "[trace] phase1_ms={} nodes_visited={} contracted={}",
                            env.stats.phase1_ms,
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
    }
}
