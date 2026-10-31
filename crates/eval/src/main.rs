//! Engram's evaluation runner. Public benchmarks run against the real ingest and search code,
//! in-process, with a fresh in-memory database per question. Rules and suites: docs/EVALUATION.md.

mod cache;
mod longmemeval;
mod report;
mod scale;
mod split;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use engram_embed::{Embedder, HashEmbedder, LocalEmbedder, LocalModel, LocalReranker, RerankModel};

/// Model weights are shared with the app: `ENGRAM_MODELS`, else the app's data folder.
fn models_dir() -> PathBuf {
    std::env::var_os("ENGRAM_MODELS").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library/Application Support/Engram/models")
    })
}

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
    /// BAAI/bge-small-en-v1.5, in-process.
    BgeSmall,
    /// BAAI/bge-small-en-v1.5, int8-quantized, in-process.
    BgeSmallQ,
    /// nomic-embed-text-v1.5, in-process.
    Nomic,
}

impl EmbedModel {
    fn load(self) -> Arc<dyn Embedder> {
        let local = |m| -> Arc<dyn Embedder> {
            // ONNX Runtime already spreads one batch over several cores; a few instances keep them busy.
            let instances = 3;
            Arc::new(LocalEmbedder::load(m, models_dir(), instances).unwrap_or_else(|e| panic!("{e}")))
        };
        match self {
            EmbedModel::Hash => Arc::new(HashEmbedder::default()),
            EmbedModel::BgeSmall => local(LocalModel::BgeSmall),
            EmbedModel::BgeSmallQ => local(LocalModel::BgeSmallQ),
            EmbedModel::Nomic => local(LocalModel::NomicV15),
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
        /// Rerank the top results with a cross-encoder: jina-turbo or bge-reranker-base.
        #[arg(long)]
        rerank: Option<String>,
        /// Share of the final score that comes from the reranker.
        #[arg(long, default_value_t = 0.5)]
        rerank_weight: f64,
        /// How many top results the reranker re-scores.
        #[arg(long, default_value_t = 20)]
        rerank_top: usize,
        /// Only the first N questions of the split (for quick checks).
        #[arg(long)]
        limit: Option<usize>,
        /// Per-question results as JSON.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Suite D: search speed and nearest-neighbour recall on synthetic projects of growing size.
    Scale {
        /// Where the synthetic databases are kept (reused between runs).
        #[arg(long)]
        dir: PathBuf,
        #[arg(long, value_delimiter = ',', default_value = "10000,100000,1000000")]
        sizes: Vec<usize>,
        #[arg(long, default_value_t = 100)]
        queries: usize,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.suite {
        Suite::Longmemeval { data, split, embed, rerank, rerank_weight, rerank_top, limit, out } => {
            let rerank = rerank.map(|name| {
                let model = RerankModel::parse(&name).unwrap_or_else(|| panic!("unknown reranker: {name}"));
                let loaded = LocalReranker::load(model, models_dir()).unwrap_or_else(|e| panic!("{e}"));
                longmemeval::RerankSetup { model: Arc::new(loaded), weight: rerank_weight, top: rerank_top }
            });
            // Cache vectors next to the data, per model, so tuning runs do not re-embed.
            let inner = embed.load();
            let safe: String = inner.model_id().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
            let file = data.with_file_name(format!("embed-cache-{safe}.bin"));
            let cached = Arc::new(cache::CachedEmbedder::open(inner, file));
            eprintln!("embedding cache: {} vectors", cached.cached());
            let result = longmemeval::run(&data, split, cached.clone(), rerank, limit, out.as_deref());
            if let Err(e) = cached.save() {
                eprintln!("could not save the embedding cache: {e}");
            }
            result
        }
        Suite::Scale { dir, sizes, queries } => scale::run(&dir, &sizes, queries),
    };
    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
