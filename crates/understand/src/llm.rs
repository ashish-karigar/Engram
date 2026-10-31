use std::sync::Mutex;
use std::time::Duration;

use engram_core::{Error, Result};
use serde_json::{Value, json};

pub struct LlmRequest {
    pub system: String,
    pub prompt: String,
    pub max_tokens: u32,
}

/// The only thing the pipeline knows about models. Providers are configuration.
pub trait LlmClient: Send + Sync {
    fn name(&self) -> String;
    fn complete(&self, req: &LlmRequest) -> Result<String>;
    /// True when the model runs on this machine and competes with the user for memory.
    fn is_local(&self) -> bool {
        false
    }
}

fn post_json(url: &str, body: Value, bearer: Option<&str>) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| Error::msg(e.to_string()))?;
    let mut request = client.post(url).json(&body);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = request.send().map_err(|e| Error::msg(format!("{url}: {e}")))?;
    let status = response.status();
    let text = response.text().map_err(|e| Error::msg(e.to_string()))?;
    if !status.is_success() {
        return Err(Error::msg(format!("{url} -> {status} {}", text.chars().take(300).collect::<String>())));
    }
    Ok(serde_json::from_str(&text)?)
}

fn messages(req: &LlmRequest) -> Value {
    json!([{"role": "system", "content": req.system}, {"role": "user", "content": req.prompt}])
}

/// Meta's Model API (Muse Spark) through its OpenAI-compatible chat endpoint.
pub struct MetaClient {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl LlmClient for MetaClient {
    fn name(&self) -> String {
        format!("meta:{}", self.model)
    }

    fn complete(&self, req: &LlmRequest) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = json!({"model": self.model, "max_tokens": req.max_tokens, "messages": messages(req)});
        let data = post_json(&url, body, Some(&self.api_key))?;
        Ok(data["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
    }
}

pub struct OllamaClient {
    pub host: String,
    pub model: String,
}

impl LlmClient for OllamaClient {
    fn name(&self) -> String {
        format!("ollama:{}", self.model)
    }

    fn is_local(&self) -> bool {
        true
    }

    fn complete(&self, req: &LlmRequest) -> Result<String> {
        let url = format!("{}/api/chat", self.host.trim_end_matches('/'));
        // Ollama's default context window is ~4k tokens and silently truncates longer prompts.
        // A short keep_alive frees the model's memory soon after a batch instead of after 5 minutes.
        let body = json!({
            "model": self.model, "stream": false, "messages": messages(req), "format": "json", "keep_alive": "20s",
            "options": {"num_ctx": 16384, "temperature": 0.1}
        });
        let data = post_json(&url, body, None)?;
        Ok(data["message"]["content"].as_str().unwrap_or("").to_string())
    }
}

/// Deterministic client for tests and offline demos: replays responses in order,
/// then repeats the last one.
pub struct ScriptedClient {
    responses: Vec<String>,
    next: Mutex<usize>,
}

impl ScriptedClient {
    pub fn new(responses: Vec<String>) -> Self {
        ScriptedClient { responses, next: Mutex::new(0) }
    }
}

impl LlmClient for ScriptedClient {
    fn name(&self) -> String {
        "scripted".into()
    }

    fn complete(&self, _req: &LlmRequest) -> Result<String> {
        let mut next = self.next.lock().unwrap();
        let i = (*next).min(self.responses.len().saturating_sub(1));
        *next += 1;
        Ok(self.responses.get(i).cloned().unwrap_or_else(|| "{\"memories\":[]}".into()))
    }
}

/// Pick a provider from environment variables.
/// `ENGRAM_LLM=meta|ollama|none` (default: meta when `META_API_KEY` is set), plus
/// `META_API_KEY`, `META_BASE_URL`, `META_MODEL`, `OLLAMA_HOST`, `OLLAMA_MODEL`.
pub fn client_from_env() -> Option<Box<dyn LlmClient>> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let choice = var("ENGRAM_LLM").unwrap_or_else(|| if var("META_API_KEY").is_some() { "meta".into() } else { "none".into() });
    match (choice.as_str(), var("META_API_KEY")) {
        ("meta", Some(api_key)) => Some(Box::new(MetaClient {
            api_key,
            base_url: var("META_BASE_URL").unwrap_or_else(|| "https://api.meta.ai/v1".into()),
            model: var("META_MODEL").unwrap_or_else(|| "muse-spark-1.3".into()),
        })),
        ("ollama", _) => Some(Box::new(OllamaClient {
            host: var("OLLAMA_HOST").unwrap_or_else(|| "http://localhost:11434".into()),
            model: var("OLLAMA_MODEL").unwrap_or_else(|| "llama3.1".into()),
        })),
        _ => None,
    }
}
