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
        #[arg(long, value_enum, default_value_t = SliceCliFormat::Json)]
        format: SliceCliFormat,
        #[arg(long)]
        trace: bool,
        #[arg(long)]
        lsp_fixture: Option<PathBuf>,
        #[arg(long)]
        include_graph: bool,
    },
    /// Export a slice graph to Mermaid, Graphviz DOT, or offline HTML+SVG (stdout or --out, default --no-snippets, never auto-opens browser)
    Export {
        /// Target in format <file>:<line>:<col> (required unless --from-envelope is set)
        #[arg(required_unless_present = "from_envelope")]
        target: Option<String>,
        /// Render directly from a SliceEnvelope JSON file (or "-" for stdin)
        #[arg(long, value_name = "PATH", conflicts_with = "target")]
        from_envelope: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = ExportCliFormat::Mermaid)]
        format: ExportCliFormat,
        #[arg(long, value_enum, default_value_t = CliDirection::To)]
        direction: CliDirection,
        #[arg(long, value_name = "DIR")]
        workspace_root: Option<PathBuf>,
        #[arg(long)]
        exclude_tests: bool,
        #[arg(long)]
        null_only: bool,
        #[arg(long)]
        no_lsp: bool,
        #[arg(long, value_name = "PATH")]
        lsp_fixture: Option<PathBuf>,
        #[arg(long, conflicts_with = "no_snippets")]
        snippets: bool,
        #[arg(long, conflicts_with = "snippets")]
        no_snippets: bool,
        #[arg(long)]
        absolute_paths: bool,
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum CliDirection {
    To,
    From,
    Both,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum SliceCliFormat {
    Json,
    Text,
    Dot,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum ExportCliFormat {
    Mermaid,
    Dot,
    Html,
}
