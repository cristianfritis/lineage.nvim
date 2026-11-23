use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "lineage-engine",
    version,
    about = "Demand-driven interprocedural slicing sidecar"
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        help = "Directory for source-free rotating log files"
    )]
    pub log_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run JSON-RPC 2.0 NDJSON server over stdio
    Serve,
    /// Dump lowered Pruned SSI IR for a source file
    DumpIr { file: PathBuf },
    /// Dump CFG in Graphviz DOT format
    DumpCfg {
        #[arg(long)]
        dot: bool,
        file: PathBuf,
    },
    /// Run a standalone slice query at <file>:<line>:<col> (1-based line, 0-based or 1-based col)
    Slice {
        /// Target in format <file>:<line>:<col>
        target: String,
        #[arg(long, value_enum, default_value_t = CliDirection::To)]
        direction: CliDirection,
        #[arg(long, value_enum, default_value_t = CliFormat::Json)]
        format: CliFormat,
        #[arg(long)]
        trace: bool,
        #[arg(long)]
        lsp_fixture: Option<PathBuf>,
        #[arg(long)]
        include_graph: bool,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum CliDirection {
    To,
    From,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum CliFormat {
    Json,
    Text,
    Dot,
}
