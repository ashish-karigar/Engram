//! Cross-encoder rerankers: read the query and one candidate together and score how well the
//! candidate answers it. More precise than comparing vectors, and more expensive (one model run per
//! candidate), so search uses it only on its top few results.

use std::path::PathBuf;
use std::sync::Mutex;

use engram_core::{Error, Result};
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

pub trait Reranker: Send + Sync {
    fn model_id(&self) -> String;
    /// Relevance of each document to the query, in 0..=1, in the documents' order.
    fn score(&self, query: &str, documents: &[String]) -> Result<Vec<f32>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankModel {
    /// jinaai/jina-reranker-v1-turbo-en: 38M parameters, fast on CPU.
    JinaTurbo,
    /// BAAI/bge-reranker-base: 278M parameters, slower, usually more accurate.
    BgeBase,
}

impl RerankModel {
    pub fn parse(name: &str) -> Option<RerankModel> {
        match name {
            "jina-turbo" => Some(RerankModel::JinaTurbo),
            "bge-reranker-base" => Some(RerankModel::BgeBase),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RerankModel::JinaTurbo => "jina-reranker-v1-turbo-en",
            RerankModel::BgeBase => "bge-reranker-base",
        }
    }

    fn fastembed(self) -> RerankerModel {
        match self {
            RerankModel::JinaTurbo => RerankerModel::JINARerankerV1TurboEn,
            RerankModel::BgeBase => RerankerModel::BGERerankerBase,
        }
    }
}

pub struct LocalReranker {
    model: RerankModel,
    inner: Mutex<TextRerank>,
}

impl LocalReranker {
    pub fn load(model: RerankModel, cache_dir: PathBuf) -> Result<LocalReranker> {
        let options = RerankInitOptions::new(model.fastembed()).with_cache_dir(cache_dir).with_show_download_progress(false);
        let inner = TextRerank::try_new(options).map_err(|e| Error::msg(format!("loading {}: {e}", model.name())))?;
        Ok(LocalReranker { model, inner: Mutex::new(inner) })
    }
}

/// Batches are kept small for the same memory reason as the embedders.
const BATCH: usize = 4;

impl Reranker for LocalReranker {
    fn model_id(&self) -> String {
        format!("local:{}", self.model.name())
    }

    fn score(&self, query: &str, documents: &[String]) -> Result<Vec<f32>> {
        if documents.is_empty() {
            return Ok(vec![]);
        }
        let docs: Vec<&str> = documents.iter().map(String::as_str).collect();
        let mut model = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let results = model.rerank(query, docs.as_slice(), false, Some(BATCH)).map_err(|e| Error::msg(format!("{}: {e}", self.model.name())))?;
        // Results come back best first, with the input position; the model gives logits.
        let mut scores = vec![0.0; documents.len()];
        for r in results {
            scores[r.index] = 1.0 / (1.0 + (-r.score).exp());
        }
        Ok(scores)
    }
}
