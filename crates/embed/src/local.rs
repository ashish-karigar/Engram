//! Embedding models run in-process with ONNX Runtime (via fastembed): no server, no GPU, no Ollama.
//! Weights are downloaded once from Hugging Face into the given cache folder.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use engram_core::{Error, Result};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::Embedder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalModel {
    /// BAAI/bge-small-en-v1.5: 384 dims, ~130 MB, fast.
    BgeSmall,
    /// nomic-embed-text-v1.5: 768 dims, long inputs, trained with code; ~550 MB.
    NomicV15,
    /// Int8-quantized versions: a fraction of the size, faster on CPU, slightly less precise.
    BgeSmallQ,
    NomicV15Q,
}

impl LocalModel {
    pub fn parse(name: &str) -> Option<LocalModel> {
        match name {
            "bge-small" | "bge-small-en-v1.5" => Some(LocalModel::BgeSmall),
            "nomic" | "nomic-embed-text-v1.5" => Some(LocalModel::NomicV15),
            "bge-small-q" => Some(LocalModel::BgeSmallQ),
            "nomic-q" => Some(LocalModel::NomicV15Q),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LocalModel::BgeSmall => "bge-small-en-v1.5",
            LocalModel::NomicV15 => "nomic-embed-text-v1.5",
            LocalModel::BgeSmallQ => "bge-small-en-v1.5-q",
            LocalModel::NomicV15Q => "nomic-embed-text-v1.5-q",
        }
    }

    fn fastembed(self) -> EmbeddingModel {
        match self {
            LocalModel::BgeSmall => EmbeddingModel::BGESmallENV15,
            LocalModel::NomicV15 => EmbeddingModel::NomicEmbedTextV15,
            LocalModel::BgeSmallQ => EmbeddingModel::BGESmallENV15Q,
            LocalModel::NomicV15Q => EmbeddingModel::NomicEmbedTextV15Q,
        }
    }

    /// Instruction prefixes each model was trained with: (query, document).
    fn prefixes(self) -> (&'static str, &'static str) {
        match self {
            LocalModel::BgeSmall | LocalModel::BgeSmallQ => ("Represent this sentence for searching relevant passages: ", ""),
            LocalModel::NomicV15 | LocalModel::NomicV15Q => ("search_query: ", "search_document: "),
        }
    }
}

pub struct LocalEmbedder {
    model: LocalModel,
    /// fastembed needs `&mut` to run a batch, so each instance serves one batch at a time; a small
    /// pool lets several threads embed at once (bulk indexing, evaluation).
    pool: Vec<Mutex<TextEmbedding>>,
    next: AtomicUsize,
}

/// Inputs longer than this are cut before embedding; chunks are well below it.
const MAX_CHARS: usize = 8000;
/// Small batches keep ONNX Runtime's working memory low; larger ones were not faster on CPU.
const BATCH: usize = 2;

impl LocalEmbedder {
    /// `instances` copies of the model (each holds its weights in memory; 1–2 for the app).
    pub fn load(model: LocalModel, cache_dir: PathBuf, instances: usize) -> Result<LocalEmbedder> {
        let pool = (0..instances.max(1))
            .map(|_| {
                let options = TextInitOptions::new(model.fastembed())
                    .with_cache_dir(cache_dir.clone())
                    .with_show_download_progress(false);
                TextEmbedding::try_new(options).map(Mutex::new).map_err(|e| Error::msg(format!("loading {}: {e}", model.name())))
            })
            .collect::<Result<_>>()?;
        Ok(LocalEmbedder { model, pool, next: AtomicUsize::new(0) })
    }

    fn run(&self, prefix: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let inputs: Vec<String> = texts.iter().map(|t| format!("{prefix}{}", t.chars().take(MAX_CHARS).collect::<String>())).collect();
        // Take a free instance if there is one, else wait on the next in turn.
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        let n = self.pool.len();
        let mut model = (0..n)
            .find_map(|i| self.pool[(start + i) % n].try_lock().ok())
            .unwrap_or_else(|| self.pool[start % n].lock().unwrap_or_else(|e| e.into_inner()));
        model.embed(&inputs, Some(BATCH)).map_err(|e| Error::msg(format!("{}: {e}", self.model.name())))
    }
}

impl Embedder for LocalEmbedder {
    fn model_id(&self) -> String {
        format!("local:{}", self.model.name())
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.run(self.model.prefixes().1, texts)
    }

    fn embed_query(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.run(self.model.prefixes().0, texts)
    }
}
