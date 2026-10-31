//! Minimal MCP server over streamable HTTP (JSON responses, stateless).
//! Implements the handful of methods an agent needs: initialize, ping, tools/list, tools/call.

use engram_core::Result;
use engram_understand::notes::NoteInput;
use serde_json::{Value, json};

use crate::ops::{Ctx, OpResult, op_add_document, op_brief, op_expand, op_note, op_profile, op_search};

const DEFAULT_PROTOCOL: &str = "2025-03-26";

/// A deliberately small tool surface: every tool definition costs the agent context tokens.
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "brief",
            "description": "Call at the start of a task and after context compaction. Returns the project map, key decisions and reasons, and recent agent activity. Pass `task` to focus it.",
            "inputSchema": {"type": "object", "properties": {"task": {"type": "string"}, "budget": {"type": "integer", "minimum": 1}}}
        },
        {
            "name": "search",
            "description": "Ask project memory a question (e.g. 'why is auth session based', 'what depends on the lidar filter'). Searches decisions and reasons, and the project's documents and code. Returns ranked results with ids you can pass to `expand`.",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string"},
                "mode": {"type": "string", "enum": ["hybrid", "memories", "documents"], "description": "hybrid (default) searches both"},
                "budget": {"type": "integer", "minimum": 1}, "asOf": {"type": "string"}
            }, "required": ["query"]}
        },
        {
            "name": "expand",
            "description": "Open a memory, document, chunk or code node by id: reasons, version history, evidence, full text, and what it connects to.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "depth": {"type": "integer", "minimum": 1, "maximum": 3}}, "required": ["id"]}
        },
        {
            "name": "note",
            "description": "Record a decision, its reason, a rejected option, or a dead end as you work, so other agents and future sessions know it. Name the files involved.",
            "inputSchema": {"type": "object", "properties": {
                "title": {"type": "string"}, "claim": {"type": "string"},
                "type": {"type": "string", "enum": ["decision", "fact", "preference", "rejected_option", "dead_end", "episode", "task"]},
                "rationale": {"type": "string"}, "files": {"type": "array", "items": {"type": "string"}},
                "expiresAt": {"type": "string", "description": "ISO 8601; for temporary facts"}
            }, "required": ["title", "claim"]}
        },
        {
            "name": "profile",
            "description": "The project's standing facts (conventions and decisions that hold until replaced) and current focus. Cheap; no query needed.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "add_document",
            "description": "Save a document (spec, design note, research, meeting notes) to project memory. It is chunked and searchable at once. Reuse `id` to update it.",
            "inputSchema": {"type": "object", "properties": {"title": {"type": "string"}, "content": {"type": "string"}, "id": {"type": "string"}}, "required": ["title", "content"]}
        }
    ])
}

fn call_tool(ctx: &Ctx, name: &str, args: Value) -> Result<OpResult> {
    let bad = |e: serde_json::Error| engram_core::Error::msg(format!("invalid arguments for {name}: {e}"));
    match name {
        "brief" => op_brief(ctx, serde_json::from_value(args).map_err(bad)?),
        "search" => op_search(ctx, serde_json::from_value(args).map_err(bad)?),
        "expand" => op_expand(ctx, serde_json::from_value(args).map_err(bad)?),
        "note" => op_note(ctx, serde_json::from_value::<NoteInput>(args).map_err(bad)?),
        "profile" => op_profile(ctx),
        "add_document" => op_add_document(ctx, serde_json::from_value(args).map_err(bad)?),
        other => Err(engram_core::Error::msg(format!("unknown tool: {other}"))),
    }
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Handle one JSON-RPC message. Notifications (no id) produce no response.
pub fn handle_message(ctx: &Ctx, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(DEFAULT_PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "engram", "version": env!("CARGO_PKG_VERSION")}
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tool_definitions()}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call_tool(ctx, name, args) {
                Ok(out) => json!({"content": [{"type": "text", "text": out.text}], "isError": false}),
                Err(e) => json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true}),
            }
        }
        other => return Some(error(&id, -32601, &format!("method not found: {other}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Handle a request body: a single message or a batch. None means "accepted, nothing to return".
pub fn handle_body(ctx: &Ctx, body: &Value) -> Option<Value> {
    match body {
        Value::Array(items) => {
            let out: Vec<Value> = items.iter().filter_map(|m| handle_message(ctx, m)).collect();
            (!out.is_empty()).then_some(Value::Array(out))
        }
        single => handle_message(ctx, single),
    }
}
