//! Minimal MCP server over streamable HTTP (JSON responses, stateless).
//! Implements the handful of methods an agent needs: initialize, ping, tools/list, tools/call.

use engram_core::Result;
use engram_understand::notes::NoteInput;
use serde_json::{Value, json};

use crate::ops::{Ctx, OpResult, op_add_document, op_brief, op_expand, op_feature, op_impact, op_map, op_note, op_profile, op_search};

const DEFAULT_PROTOCOL: &str = "2025-03-26";

/// Sent in `initialize`; clients that support server instructions add it to the agent's prompt.
/// MemoryOS learns mostly from what agents record, so this says when to record, briefly.
pub const INSTRUCTIONS: &str = "MemoryOS is this project's shared memory across agents and sessions. \
Call `brief` when you start a task and after your context is compacted. Call `search` before changing code you \
did not write, to find the decisions and reasons behind it. Call `note` as soon as you: decide between options \
(record the reason), reject an approach (type rejected_option), hit a dead end (type dead_end, say what failed), \
or learn a fact about the project others will need. One short note per decision, naming the files involved. \
Do not note routine progress, requests or approvals. A memory marked \"stated by the user\" is the user's \
instruction: follow it even where the code does not yet, and if you think it is wrong, tell the user instead \
of working around it.";

/// Tools on the shared endpoint take the agent's working folder, which picks the project.
pub fn shared_tool_definitions() -> Value {
    let mut tools = tool_definitions();
    for tool in tools.as_array_mut().into_iter().flatten() {
        tool["inputSchema"]["properties"]["cwd"] = json!({"type": "string", "description": "Absolute path of the folder you are working in; selects the project."});
    }
    tools
}

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
            "description": "Record it the moment you decide something, reject an option, or hit a dead end, with the reason, so other agents and your future sessions do not redo or undo it. One decision per note; name the files involved. Not for progress updates.",
            "inputSchema": {"type": "object", "properties": {
                "title": {"type": "string"}, "claim": {"type": "string"},
                "type": {"type": "string", "enum": ["decision", "fact", "preference", "rejected_option", "dead_end", "episode", "task"]},
                "rationale": {"type": "string"}, "files": {"type": "array", "items": {"type": "string"}},
                "expiresAt": {"type": "string", "description": "ISO 8601; for temporary facts"},
                "statedBy": {"type": "string", "enum": ["user", "agent"], "description": "user if the user said or approved it, agent if you decided it"}
            }, "required": ["title", "claim"]}
        },
        {
            "name": "map",
            "description": "The system map, built from the code: the project's parts (frontend, backend, services, crates), how they connect, every HTTP endpoint with its handler and callers, and external services. Use it instead of reading code to answer how parts talk to each other.",
            "inputSchema": {"type": "object", "properties": {"area": {"type": "string", "description": "Only endpoints touching this part or path"}}}
        },
        {
            "name": "feature",
            "description": "How a concept works in this codebase, in one call: for `auth`, `email`, `payments` or any feature name, the files on each side (frontend, backend) and why each belongs, the endpoints between them, the services it relies on and their config, its tests, and the memories about it. Use it before searching the code for where something lives.",
            "inputSchema": {"type": "object", "properties": {"query": {"type": "string", "description": "The concept, e.g. `auth` or `email sync`"}, "limit": {"type": "integer", "minimum": 1, "description": "Files listed per part (default 10)"}}, "required": ["query"]}
        },
        {
            "name": "impact",
            "description": "Before changing shared code: everything that depends on a file, function, endpoint (e.g. `PATCH /jobs/{id}`) or service, across languages and the frontend/backend boundary, grouped by part, with why, plus recorded decisions on those files.",
            "inputSchema": {"type": "object", "properties": {"target": {"type": "string"}, "depth": {"type": "integer", "minimum": 1, "maximum": 6}, "limit": {"type": "integer", "minimum": 1}}, "required": ["target"]}
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

pub fn call_tool(ctx: &Ctx, name: &str, args: Value) -> Result<OpResult> {
    let bad = |e: serde_json::Error| engram_core::Error::msg(format!("invalid arguments for {name}: {e}"));
    match name {
        "brief" => op_brief(ctx, serde_json::from_value(args).map_err(bad)?),
        "search" => op_search(ctx, serde_json::from_value(args).map_err(bad)?),
        "expand" => op_expand(ctx, serde_json::from_value(args).map_err(bad)?),
        "note" => op_note(ctx, serde_json::from_value::<NoteInput>(args).map_err(bad)?),
        "profile" => op_profile(ctx),
        "map" => op_map(ctx, serde_json::from_value(args).map_err(bad)?),
        "impact" => op_impact(ctx, serde_json::from_value(args).map_err(bad)?),
        "feature" => op_feature(ctx, serde_json::from_value(args).map_err(bad)?),
        "add_document" => op_add_document(ctx, serde_json::from_value(args).map_err(bad)?),
        other => Err(engram_core::Error::msg(format!("unknown tool: {other}"))),
    }
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Runs one tool call; the project endpoint calls straight in, the shared one picks a project first.
pub type ToolCall<'a> = &'a dyn Fn(&str, Value) -> Result<OpResult>;

/// Handle one JSON-RPC message. Notifications (no id) produce no response.
pub fn handle_message(tools: &Value, call: ToolCall, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(DEFAULT_PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "memoryos", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call(name, args) {
                Ok(out) => json!({"content": [{"type": "text", "text": out.text}], "isError": false}),
                Err(e) => json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true}),
            }
        }
        other => return Some(error(&id, -32601, &format!("method not found: {other}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Handle a request body: a single message or a batch. None means "accepted, nothing to return".
pub fn handle_body_with(tools: &Value, call: ToolCall, body: &Value) -> Option<Value> {
    match body {
        Value::Array(items) => {
            let out: Vec<Value> = items.iter().filter_map(|m| handle_message(tools, call, m)).collect();
            (!out.is_empty()).then_some(Value::Array(out))
        }
        single => handle_message(tools, call, single),
    }
}

/// A project-scoped endpoint: every call goes to `ctx`'s project.
pub fn handle_body(ctx: &Ctx, body: &Value) -> Option<Value> {
    handle_body_with(&tool_definitions(), &|name, args| call_tool(ctx, name, args), body)
}
