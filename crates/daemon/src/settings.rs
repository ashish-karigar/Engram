use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use engram_core::{Error, Result};
use engram_embed::embedder::{Embedder, HashEmbedder, OllamaEmbedder, OpenAiEmbedder};
use engram_understand::llm::{LlmClient, MetaClient, OllamaClient};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::project::RuntimeOptions;
use crate::secrets::write_private;

/// Everything a user can change from the Settings page. Stored in `<app data>/settings.json` (0600).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub llm: LlmSettings,
    pub embeddings: EmbedSettings,
    pub indexing: IndexSettings,
    pub memory: MemorySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LlmSettings {
    /// none | meta | ollama
    pub provider: String,
    pub meta_base_url: String,
    pub meta_model: String,
    pub meta_api_key: Option<String>,
    pub ollama_host: String,
    pub ollama_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct EmbedSettings {
    /// hash | ollama | openai
    pub provider: String,
    pub ollama_host: String,
    pub ollama_model: String,
    pub openai_base_url: String,
    pub openai_model: String,
    pub openai_api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct IndexSettings {
    /// Read Claude Code, Codex and Muse session logs for sessions run in the project.
    pub sessions: bool,
    pub index_docs: bool,
    pub index_code: bool,
    pub max_file_kb: u64,
    /// Extra paths to skip: `data/` (folder), `*.csv` (extension), or any substring.
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemorySettings {
    pub extract_every_secs: u64,
    pub decay_every_hours: u64,
    pub snapshot_every_secs: u64,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

impl Default for LlmSettings {
    fn default() -> Self {
        LlmSettings {
            provider: env("ENGRAM_LLM").unwrap_or_else(|| if env("META_API_KEY").is_some() { "meta".into() } else { "none".into() }),
            meta_base_url: env("META_BASE_URL").unwrap_or_else(|| "https://api.meta.ai/v1".into()),
            meta_model: env("META_MODEL").unwrap_or_else(|| "muse-spark-1.3".into()),
            meta_api_key: env("META_API_KEY"),
            ollama_host: env("OLLAMA_HOST").unwrap_or_else(|| "http://localhost:11434".into()),
            ollama_model: env("OLLAMA_MODEL").unwrap_or_else(|| "llama3.1".into()),
        }
    }
}

impl Default for EmbedSettings {
    fn default() -> Self {
        EmbedSettings {
            provider: env("ENGRAM_EMBED").unwrap_or_else(|| "hash".into()),
            ollama_host: env("OLLAMA_HOST").unwrap_or_else(|| "http://localhost:11434".into()),
            ollama_model: env("ENGRAM_EMBED_MODEL").unwrap_or_else(|| "nomic-embed-text".into()),
            openai_base_url: env("ENGRAM_EMBED_BASE_URL").unwrap_or_else(|| "https://api.openai.com/v1".into()),
            openai_model: env("ENGRAM_EMBED_MODEL").unwrap_or_else(|| "text-embedding-3-small".into()),
            openai_api_key: env("ENGRAM_EMBED_API_KEY"),
        }
    }
}

impl Default for IndexSettings {
    fn default() -> Self {
        IndexSettings { sessions: true, index_docs: true, index_code: true, max_file_kb: 512, ignore: vec![] }
    }
}

impl Default for MemorySettings {
    fn default() -> Self {
        MemorySettings { extract_every_secs: 60, decay_every_hours: 6, snapshot_every_secs: 15 }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings { llm: LlmSettings::default(), embeddings: EmbedSettings::default(), indexing: IndexSettings::default(), memory: MemorySettings::default() }
    }
}

const MASK: &str = "••••";

fn mask(key: &Option<String>) -> Value {
    match key {
        Some(k) if k.len() > 4 => json!(format!("{MASK}{}", &k[k.len() - 4..])),
        Some(_) => json!(MASK),
        None => Value::Null,
    }
}

/// Keep the stored key when the UI sends back the masked value; clear it on "".
fn merge_key(current: &Option<String>, incoming: &Option<String>) -> Option<String> {
    match incoming.as_deref() {
        Some(v) if v.starts_with(MASK) => current.clone(),
        Some("") => None,
        Some(v) => Some(v.to_string()),
        None => current.clone(),
    }
}

impl Settings {
    pub fn load(base: &Path) -> Settings {
        std::fs::read_to_string(base.join("settings.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, base: &Path) -> Result<()> {
        write_private(&base.join("settings.json"), &serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// What the UI may see: API keys are masked, never sent back in full.
    pub fn public(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        v["llm"]["metaApiKey"] = mask(&self.llm.meta_api_key);
        v["embeddings"]["openaiApiKey"] = mask(&self.embeddings.openai_api_key);
        v
    }

    /// Apply an edit from the UI on top of the current settings.
    pub fn merged(&self, incoming: Value) -> Result<Settings> {
        let mut next: Settings = serde_json::from_value(incoming).map_err(|e| Error::msg(format!("invalid settings: {e}")))?;
        next.llm.meta_api_key = merge_key(&self.llm.meta_api_key, &next.llm.meta_api_key);
        next.embeddings.openai_api_key = merge_key(&self.embeddings.openai_api_key, &next.embeddings.openai_api_key);
        next.validate()?;
        Ok(next)
    }

    fn validate(&self) -> Result<()> {
        let one_of = |v: &str, ok: &[&str], what: &str| {
            if ok.contains(&v) { Ok(()) } else { Err(Error::msg(format!("{what} must be one of {}", ok.join(", ")))) }
        };
        one_of(&self.llm.provider, &["none", "meta", "ollama"], "model provider")?;
        one_of(&self.embeddings.provider, &["hash", "ollama", "openai"], "embedding provider")?;
        if self.memory.extract_every_secs < 10 || self.memory.snapshot_every_secs < 5 {
            return Err(Error::msg("intervals are too short"));
        }
        Ok(())
    }

    pub fn llm_client(&self) -> Option<Box<dyn LlmClient>> {
        let l = &self.llm;
        match l.provider.as_str() {
            "meta" => l.meta_api_key.clone().map(|api_key| {
                Box::new(MetaClient { api_key, base_url: l.meta_base_url.clone(), model: l.meta_model.clone() }) as Box<dyn LlmClient>
            }),
            "ollama" => Some(Box::new(OllamaClient { host: l.ollama_host.clone(), model: l.ollama_model.clone() })),
            _ => None,
        }
    }

    pub fn embedder(&self) -> Arc<dyn Embedder> {
        let e = &self.embeddings;
        match (e.provider.as_str(), &e.openai_api_key) {
            ("ollama", _) => Arc::new(OllamaEmbedder { host: e.ollama_host.clone(), model: e.ollama_model.clone() }),
            ("openai", Some(key)) => Arc::new(OpenAiEmbedder { base_url: e.openai_base_url.clone(), api_key: key.clone(), model: e.openai_model.clone() }),
            _ => Arc::new(HashEmbedder::default()),
        }
    }

    pub fn runtime(&self) -> RuntimeOptions {
        let me = self.clone();
        RuntimeOptions {
            llm: Arc::new(move || me.llm_client()),
            embedder: self.embedder(),
            pipeline_every: Duration::from_secs(self.memory.extract_every_secs),
            decay_every: Duration::from_secs(self.memory.decay_every_hours * 3600),
            capture_debounce: Duration::from_millis(500),
            snapshot_interval: Duration::from_secs(self.memory.snapshot_every_secs),
            index: self.indexing.clone(),
            sources: engram_sessions::Sources::default(),
        }
    }
}

/// Matches the user's extra ignore patterns: `dir/`, `*.ext`, or a plain substring.
pub fn ignored(path: &str, patterns: &[String]) -> bool {
    patterns.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).any(|p| {
        if let Some(ext) = p.strip_prefix("*.") {
            path.ends_with(&format!(".{ext}"))
        } else if p.ends_with('/') {
            path.starts_with(p) || path.contains(&format!("/{p}"))
        } else {
            path.contains(p)
        }
    })
}
