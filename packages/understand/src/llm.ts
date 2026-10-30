export interface LlmRequest {
  system: string;
  prompt: string;
  maxTokens?: number;
}

/** The only thing the pipeline knows about models. Providers are configuration. */
export interface LlmClient {
  readonly name: string;
  complete(req: LlmRequest): Promise<string>;
}

async function postJson(url: string, body: unknown, headers: Record<string, string> = {}): Promise<unknown> {
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${url} -> ${res.status} ${(await res.text()).slice(0, 300)}`);
  return res.json();
}

export interface OpenAiCompatConfig {
  baseUrl: string;
  apiKey: string;
  model: string;
}

/** Meta's Model API (Muse Spark) via its OpenAI-compatible chat endpoint. */
export function metaClient(cfg: OpenAiCompatConfig): LlmClient {
  return {
    name: `meta:${cfg.model}`,
    async complete(req) {
      const data = (await postJson(
        `${cfg.baseUrl.replace(/\/$/, "")}/chat/completions`,
        {
          model: cfg.model,
          max_tokens: req.maxTokens ?? 2000,
          messages: [
            { role: "system", content: req.system },
            { role: "user", content: req.prompt },
          ],
        },
        { authorization: `Bearer ${cfg.apiKey}` },
      )) as { choices?: { message?: { content?: string } }[] };
      return data.choices?.[0]?.message?.content ?? "";
    },
  };
}

export function ollamaClient(cfg: { host: string; model: string }): LlmClient {
  return {
    name: `ollama:${cfg.model}`,
    async complete(req) {
      const data = (await postJson(`${cfg.host.replace(/\/$/, "")}/api/chat`, {
        model: cfg.model,
        stream: false,
        messages: [
          { role: "system", content: req.system },
          { role: "user", content: req.prompt },
        ],
      })) as { message?: { content?: string } };
      return data.message?.content ?? "";
    },
  };
}

/** Deterministic client for tests and offline demos. */
export function scriptedClient(responses: string | string[] | ((req: LlmRequest) => string)): LlmClient {
  let i = 0;
  return {
    name: "scripted",
    async complete(req) {
      if (typeof responses === "function") return responses(req);
      if (typeof responses === "string") return responses;
      return responses[i++] ?? "{\"memories\":[]}";
    },
  };
}

/**
 * Pick a provider from environment variables.
 *   ENGRAM_LLM=meta|ollama|none   (default: meta if META_API_KEY is set, else none)
 *   META_API_KEY, META_BASE_URL, META_MODEL
 *   OLLAMA_HOST, OLLAMA_MODEL
 */
export function clientFromEnv(env: NodeJS.ProcessEnv = process.env): LlmClient | null {
  const choice = env.ENGRAM_LLM ?? (env.META_API_KEY ? "meta" : "none");
  if (choice === "meta" && env.META_API_KEY) {
    return metaClient({
      apiKey: env.META_API_KEY,
      baseUrl: env.META_BASE_URL ?? "https://api.meta.ai/v1",
      model: env.META_MODEL ?? "muse-spark-1.3",
    });
  }
  if (choice === "ollama") {
    return ollamaClient({ host: env.OLLAMA_HOST ?? "http://localhost:11434", model: env.OLLAMA_MODEL ?? "llama3.1" });
  }
  return null;
}
