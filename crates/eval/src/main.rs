//! Engram's evaluation runner. Public benchmarks run against the real ingest and search code,
//! in-process, with a fresh in-memory database per question. Rules and suites: docs/EVALUATION.md.

mod longmemeval;
mod report;
mod split;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use engram_embed::{Embedder, HashEmbedder};

#[derive(Parser)]
#[command(name = "engram-eval", about = "Run Engram's evaluation suites")]
struct Cli {
    #[command(subcommand)]
    suite: Suite,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Split {
    /// 20% of questions, chosen by a hash of the id: tune on this.
    Dev,
    /// The other 80%: report on this.
    Test,
    All,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum EmbedModel {
    /// Built-in feature hashing (lexical, no download).
    Hash,
}

impl EmbedModel {
    fn load(self) -> Arc<dyn Embedder> {
        match self {
            EmbedModel::Hash => Arc::new(HashEmbedder::default()),
        }
    }
}

#[derive(Subcommand)]
enum Suite {
    /// Suite A: LongMemEval-S retrieval (recall_any@K over evidence sessions).
    Longmemeval {
        /// Path to longmemeval_s_cleaned.json.
        #[arg(long)]
        data: PathBuf,
        #[arg(long, value_enum, default_value = "dev")]
        split: Split,
        #[arg(long, value_enum, default_value = "hash")]
        embed: EmbedModel,
        /// Only the first N questions of the split (for quick checks).
        #[arg(long)]
        limit: Option<usize>,
        /// Per-question results as JSON.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.suite {
        Suite::Longmemeval { data, split, embed, limit, out } => longmemeval::run(&data, split, embed.load(), limit, out.as_deref()),
    };
    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
