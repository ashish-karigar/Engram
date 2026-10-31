use std::time::Duration;

use engram_core::{Error, Result};
use serde_json::{Value, json};

/// Turns text into vectors. Providers are configuration; the store never mixes models.
pub trait Embedder: Send + Sync {
    /// Stable identifier, stored next to every vector (e.g. `hash-384`, `ollama:nomic-embed-text`).
    fn model_id(&self) -> String;
    /// Vectors for stored text (chunks, memories).
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// Vectors for search queries. Models trained with separate query and document instructions
    /// override this; for the rest it is the same as `embed`.
    fn embed_query(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed(texts)
    }
}

/// Built-in, offline embedder: signed feature hashing of words and character trigrams.
/// No download and fully deterministic. Captures lexical and sub-word overlap, not deep
/// semantics; use Ollama or an API model for that.
pub struct HashEmbedder {
    pub dim: usize,
}

impl Default for HashEmbedder {
    fn default() -> Self {
        HashEmbedder { dim: 384 }
    }
}

/// FNV-1a: a fixed hash, so stored vectors stay valid across compiler versions.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn features(text: &str) -> Vec<(String, f32)> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).collect();
    let mut out: Vec<(String, f32)> = words.iter().map(|w| (format!("w:{w}"), 1.0)).collect();
    // Character 4-grams of the word stem catch "downsample" ~ "downsampling" without
    // over-weighting common endings like "-tion".
    for w in words.iter().filter(|w| w.chars().count() >= 4) {
        let padded: Vec<char> = format!("^{w}").chars().collect();
        out.extend(padded.windows(4).map(|g| (format!("g:{}", g.iter().collect::<String>()), 0.4)));
    }
    out.extend(words.windows(2).map(|p| (format!("b:{} {}", p[0], p[1]), 0.7)));
    out
}

impl Embedder for HashEmbedder {
    fn model_id(&self) -> String {
        format!("hash-v2-{}", self.dim)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut v = vec![0f32; self.dim];
                for (feature, weight) in features(text) {
                    let bits = fnv1a(&feature);
                    let sign = if bits >> 63 == 0 { 1.0 } else { -1.0 };
                    v[(bits % self.dim as u64) as usize] += sign * weight;
                }
                normalize(v)
            })
            .collect())
    }
}

pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

fn post(url: &str, body: Value, bearer: Option<&str>) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| Error::msg(e.to_string()))?;
    let mut req = client.post(url).json(&body);
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }
    let res = req.send().map_err(|e| Error::msg(format!("{url}: {e}")))?;
    let status = res.status();
    let text = res.text().map_err(|e| Error::msg(e.to_string()))?;
    if !status.is_success() {
        return Err(Error::msg(format!("{url} -> {status} {}", text.chars().take(200).collect::<String>())));
    }
    Ok(serde_json::from_str(&text)?)
}

fn vectors(v: &Value) -> Result<Vec<Vec<f32>>> {
    let rows = v.as_array().ok_or_else(|| Error::msg("embedding response has no vectors"))?;
    Ok(rows
        .iter()
        .map(|row| normalize(row.as_array().into_iter().flatten().filter_map(Value::as_f64).map(|x| x as f32).collect()))
        .collect())
}

/// Local semantic embeddings through Ollama (e.g. `nomic-embed-text`).
pub struct OllamaEmbedder {
    pub host: String,
    pub model: String,
}

impl Embedder for OllamaEmbedder {
    fn model_id(&self) -> String {
        format!("ollama:{}", self.model)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", self.host.trim_end_matches('/'));
        vectors(&post(&url, json!({"model": self.model, "input": texts}), None)?["embeddings"])
    }
}

/// Any OpenAI-compatible `/embeddings` endpoint (including Meta's, if offered).
pub struct OpenAiEmbedder {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl Embedder for OpenAiEmbedder {
    fn model_id(&self) -> String {
        format!("openai:{}", self.model)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let data = post(&url, json!({"model": self.model, "input": texts}), Some(&self.api_key))?;
        let rows: Vec<Value> = data["data"].as_array().into_iter().flatten().map(|d| d["embedding"].clone()).collect();
        vectors(&Value::Array(rows))
    }
}

/// `ENGRAM_EMBED=hash|ollama|openai` (default hash). Ollama: `OLLAMA_HOST`, `ENGRAM_EMBED_MODEL`.
/// OpenAI-compatible: `ENGRAM_EMBED_BASE_URL`, `ENGRAM_EMBED_API_KEY`, `ENGRAM_EMBED_MODEL`.
pub fn embedder_from_env() -> Box<dyn Embedder> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    match var("ENGRAM_EMBED").as_deref() {
        Some("ollama") => Box::new(OllamaEmbedder {
            host: var("OLLAMA_HOST").unwrap_or_else(|| "http://localhost:11434".into()),
            model: var("ENGRAM_EMBED_MODEL").unwrap_or_else(|| "nomic-embed-text".into()),
        }),
        Some("openai") => match (var("ENGRAM_EMBED_BASE_URL"), var("ENGRAM_EMBED_API_KEY")) {
            (Some(base_url), Some(api_key)) => Box::new(OpenAiEmbedder {
                base_url,
                api_key,
                model: var("ENGRAM_EMBED_MODEL").unwrap_or_else(|| "text-embedding-3-small".into()),
            }),
            _ => Box::new(HashEmbedder::default()),
        },
        _ => Box::new(HashEmbedder::default()),
    }
}
