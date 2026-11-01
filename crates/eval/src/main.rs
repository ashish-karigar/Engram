//! Engram's evaluation runner. Public benchmarks run against the real ingest and search code,
//! in-process, with a fresh in-memory database per question. Rules and suites: docs/EVALUATION.md.

mod cache;
mod history;
mod links;
mod longmemeval;
mod map;
mod qa;
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
#[command(name = "engram-eval", about = "Run MemoryOS's evaluation suites")]
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
        /// Only one question type, e.g. temporal-reasoning.
        #[arg(long)]
        kind: Option<String>,
        /// Per-question results as JSON.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Suite A answer accuracy: the configured model answers from MemoryOS's retrieval and judges
    /// with LongMemEval's own prompts. Costs model calls; results are saved and resumable.
    LongmemevalQa {
        /// Path to longmemeval_s_cleaned.json.
        #[arg(long)]
        data: PathBuf,
        #[arg(long, value_enum, default_value = "dev")]
        split: Split,
        #[arg(long, value_enum, default_value = "bge-small-q")]
        embed: EmbedModel,
        /// Only one question type, e.g. temporal-reasoning.
        #[arg(long)]
        kind: Option<String>,
        /// Only the first N questions of the split.
        #[arg(long)]
        limit: Option<usize>,
        /// Chunks of retrieved history the model reads per question.
        #[arg(long, default_value_t = 20)]
        context: usize,
        /// Questions in flight at once.
        #[arg(long, default_value_t = 6)]
        parallel: usize,
        /// JSONL of answers and verdicts; existing lines are kept and skipped.
        #[arg(long)]
        out: PathBuf,
    },
    /// Impact against git history: did impact point at the files really changed together?
    History {
        root: PathBuf,
        #[arg(long, default_value_t = 300)]
        commits: usize,
        #[arg(long, default_value_t = 3)]
        depth: usize,
    },
    /// Suite H: how accurately the model links memories, on labelled pairs (uses the configured model).
    Links,
    /// Print a repository's system map (areas, endpoints, callers, services).
    Map {
        /// Repository root.
        root: PathBuf,
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
        Suite::Longmemeval { data, split, embed, rerank, rerank_weight, rerank_top, limit, kind, out } => {
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
            let result = longmemeval::run(&data, split, kind.as_deref(), cached.clone(), rerank, limit, out.as_deref());
            if let Err(e) = cached.save() {
                eprintln!("could not save the embedding cache: {e}");
            }
            result
        }
        Suite::LongmemevalQa { data, split, embed, kind, limit, context, parallel, out } => {
            let inner = embed.load();
            let safe: String = inner.model_id().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
            let cached = Arc::new(cache::CachedEmbedder::open(inner, data.with_file_name(format!("embed-cache-{safe}.bin"))));
            let result = match links::client() {
                // One model answers and judges; LongMemEval's official judge is GPT-4o, reported next to the score.
                Some(model) => qa::run(&data, split, kind.as_deref(), cached.clone(), qa::QaSetup { reader: model.as_ref(), judge: model.as_ref(), context, parallel }, limit, &out),
                None => Err(engram_core::Error::msg("no model configured: set META_API_KEY, or choose a model in the app's Settings")),
            };
            if let Err(e) = cached.save() {
                eprintln!("could not save the embedding cache: {e}");
            }
            result
        }
        Suite::Scale { dir, sizes, queries } => scale::run(&dir, &sizes, queries),
        Suite::Map { root } => map::run(&root),
        Suite::Links => links::run(),
        Suite::History { root, commits, depth } => history::run(&root, commits, depth),
    };
    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
