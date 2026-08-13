use std::io::{self, BufRead};

use jarvis_core::{McpIngressRequest, Runtime, SqliteStore};
use serde_json::{json, Value};

fn main() {
    let store = SqliteStore::open("jarvis.db").expect("JARVIS SQLite store açılamadı");
    let mut runtime = Runtime::with_store(store);
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle(&mut runtime, request),
            Err(error) => jsonrpc_error(Value::Null, -32700, &format!("parse error: {error}")),
        };
        println!("{response}");
    }
}

fn handle(runtime: &mut Runtime, request: Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return jsonrpc_error(id, -32600, "jsonrpc must be 2.0");
    }
    match request.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}})
        }
        Some("tools/list") => json!({"jsonrpc":"2.0","id":id,"result":{"tools":tools()}}),
        Some("tools/call") => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return jsonrpc_error(id, -32602, "tools/call requires params.name");
            };
            let argument = params
                .get("arguments")
                .and_then(|arguments| arguments.get("input"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let request_id = params
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or("mcp-stdio")
                .to_owned();
            let (task, tool, verification) = runtime.handle_mcp(McpIngressRequest {
                schema_version: 1,
                request_id,
                tool_id: name.to_owned(),
                argument,
            });
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":tool.output}],"isError":tool.error.is_some(),"structuredContent":{"task_id":task.task_id,"state":format!("{:?}",task.state),"capability":task.capability,"verification":format!("{:?}",verification.status),"error":tool.error}}})
        }
        _ => jsonrpc_error(id, -32601, "method not found"),
    }
}

fn tools() -> Vec<Value> {
    [
        ("jarvis.system.health", "Read local JARVIS core health."),
        ("jarvis.system.time", "Read local epoch time."),
        ("jarvis.file.read_workspace", "Read a contained workspace file; arguments.input is a relative path."),
        ("jarvis.project.info", "Read safe project metadata."),
        ("jarvis.code.project_outline", "Read a safe source-file outline."),
        ("jarvis.docs.workspace_summary", "Read the workspace README as untrusted project content."),
        ("jarvis.note.create", "Create an approval-gated local note; arguments.input is its text."),
    ]
    .into_iter()
    .map(|(name, description)| json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":{"input":{"type":"string"}}}}))
    .collect()
}

fn jsonrpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
