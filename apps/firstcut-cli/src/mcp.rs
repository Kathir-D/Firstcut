//! `firstcut-cli mcp` — the registry as MCP tools over stdio (docs/v2-plan.md §4.3). One Model,
//! one dispatch, `tools/list` generated from [`firstcut_engine::registry::REGISTRY`] and
//! `tools/call` delegated to it — the same verbs the UI, the CLI and the control channel speak,
//! never a second hand-written list.
//!
//! Minimal JSON-RPC 2.0 on newline-delimited stdin/stdout (stdout is protocol; logs go to
//! stderr). Methods: `initialize`, `tools/list`, `tools/call`, `ping`; notifications get no
//! reply. `--connect host:port` (bridging to the control channel) is a later leg.
//!
//! Registry ids contain dots and MCP tool names may not, so a tool named
//! `firstcut_<id with dots as underscores>` carries the id in its description; a call names
//! either form. Headless here means the Model is in this process: `file.open` is just the first
//! tool the client calls.

use std::io::{BufRead, Write};

use serde_json::{Map, Value, json};

use firstcut_engine::model::Model;
use firstcut_engine::registry::{REGISTRY, dispatch};

pub fn cmd_mcp(_args: &[String]) -> Result<(), String> {
    serve()
}

/// Reads requests until stdin closes. Never writes a log line to stdout: that is the wire.
fn serve() -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut model = Model::new();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            // A peer that talks JSON-RPC must not send junk; rid it politely without a reply id.
            write_reply(&mut stdout, json!({ "jsonrpc": "2.0", "id": Value::Null, "error": { "code": -32700, "message": "parse error" } }))?;
            continue;
        };
        let Some(reply) = handle(&request, &mut model) else { continue };
        write_reply(&mut stdout, reply)?;
    }
    Ok(())
}

fn write_reply(stdout: &mut impl Write, reply: Value) -> Result<(), String> {
    let mut line = serde_json::to_string(&reply).map_err(|e| format!("mcp: encoding a reply: {e}"))?;
    line.push('\n');
    stdout.write_all(line.as_bytes()).map_err(|e| format!("mcp: writing a reply: {e}"))?;
    stdout.flush().map_err(|e| format!("mcp: flushing: {e}"))
}

/// One request in, one reply out; `None` for notifications (no `id`), which are not answered.
fn handle(request: &Value, model: &mut Model) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        "initialize" => {
            // Echo the client's protocol version when it sends one, so older peers keep theirs.
            let version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18").to_string();
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": version,
                    "capabilities": { "tools": {"listChanged": false} },
                    "serverInfo": { "name": "firstcut", "version": env!("CARGO_PKG_VERSION") }
                }
            }))
        }
        "ping" => Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
        "tools/list" => {
            let tools: Vec<Value> = REGISTRY.iter().map(tool_of).collect();
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } }))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let arguments = params.get("arguments").and_then(Value::as_object).cloned().unwrap_or_default();
            let id_of_tool = tool_id(name);
            let result = dispatch(model, &id_of_tool, &Value::Object(arguments));
            let (text, is_error) = match result {
                Ok(value) => (serde_json::to_string_pretty(&value).unwrap_or_default(), false),
                Err(error) => (error, true),
            };
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "content": [{ "type": "text", "text": text }], "isError": is_error }
            }))
        }
        _ if id.is_none() => None, // a notification we do not know: ignored, as the spec asks
        other => Some(json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": format!("no such method: {other}") }
        })),
    }
}

/// One registry definition as an MCP tool: dotted id exposed as `firstcut_file_open`, with the
/// input schema read off the declared parameters (`uint …` → integer, everything else → string).
fn tool_of(def: &firstcut_engine::registry::CommandDef) -> Value {
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();
    for (name, kind) in def.params {
        let schema = if kind.contains("uint") { json!({ "type": "integer", "minimum": 0 }) } else { json!({ "type": "string" }) };
        properties.insert((*name).to_string(), schema);
        required.push(json!(name));
    }
    let name = format!("firstcut_{}", def.id.replace('.', "_"));
    json!({
        "name": name,
        "description": format!("{} (registry id: {})", def.description, def.id),
        "inputSchema": { "type": "object", "properties": properties, "required": required },
    })
}

/// The registry id a tool name names: both `firstcut_file_open` and the raw `file.open` work;
/// ids have exactly one dot and no underscores, so the round trip is lossless.
fn tool_id(name: &str) -> String {
    let stripped = name.strip_prefix("firstcut_").unwrap_or(name);
    match stripped.split_once('_') {
        Some((kind, tail)) if !stripped.contains('.') => format!("{kind}.{tail}"),
        _ => stripped.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, id: u64, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn initialize_replies_with_capabilities_and_the_clients_version() {
        let mut model = Model::new();
        let reply = handle(&request("initialize", 1, json!({"protocolVersion": "2025-03-26", "caps": {}})), &mut model).unwrap();
        assert_eq!(reply.pointer("/result/protocolVersion").and_then(Value::as_str), Some("2025-03-26"), "the client's version is echoed");
        assert_eq!(reply.pointer("/result/capabilities/tools/listChanged").and_then(Value::as_bool), Some(false));
        assert_eq!(reply.pointer("/result/serverInfo/name").and_then(Value::as_str), Some("firstcut"));
    }

    #[test]
    fn tools_list_is_generated_from_the_registry() {
        let mut model = Model::new();
        let reply = handle(&request("tools/list", 2, json!({})), &mut model).unwrap();
        let tools = reply.pointer("/result/tools").and_then(Value::as_array).unwrap();
        assert_eq!(tools.len(), REGISTRY.len(), "one tool per registry entry, generated");
        let first = tools.iter().find(|tool| tool.get("name").and_then(Value::as_str) == Some("firstcut_rate_stars")).unwrap();
        assert_eq!(first.pointer("/inputSchema/properties/stars/type").and_then(Value::as_str), Some("integer"));
        assert_eq!(first.pointer("/inputSchema/required/0").and_then(Value::as_str), Some("stars"));
    }

    #[test]
    fn a_call_dispatches_and_reports_the_result() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let bytes = firstcut_meta::meta::cr3::SyntheticCr3::r8().at("2026:08:27 19:54:00").build();
        std::fs::write(folder.path().join("IMG_0001.CR3"), bytes).unwrap();

        let mut model = Model::new();
        let open = handle(
            &request(
                "tools/call",
                3,
                json!({
                    "name": "firstcut_file_open",
                    "arguments": { "path": folder.path(), "sessionDir": sessions.path() }
                }),
            ),
            &mut model,
        )
        .unwrap();
        assert_eq!(open.pointer("/result/isError").and_then(Value::as_bool), Some(false));
        assert_eq!(
            open.pointer("/result/content/0/text").and_then(Value::as_str).map(|text| text.contains("\"photoCount\": 1")),
            Some(true),
            "{open}"
        );

        let rate = handle(&request("tools/call", 4, json!({ "name": "rate.stars", "arguments": { "stars": 4u8 } })), &mut model).unwrap();
        assert_eq!(rate.pointer("/result/isError").and_then(Value::as_bool), Some(false), "{rate}");
        assert!(model.current_photo().unwrap().rating.stars == 4);
    }

    #[test]
    fn a_bad_call_is_an_error_result_not_a_crash() {
        let mut model = Model::new();
        let reply = handle(&request("tools/call", 5, json!({ "name": "rate.stars", "arguments": { "stars": 9 } })), &mut model).unwrap();
        assert_eq!(reply.pointer("/result/isError").and_then(Value::as_bool), Some(true));
        assert_eq!(reply.pointer("/result/content/0/text").and_then(Value::as_str), Some("rate.stars: stars is 0..=5"));
    }

    #[test]
    fn notifications_are_not_answered() {
        let mut model = Model::new();
        assert_eq!(handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), &mut model), None);
    }

    #[test]
    fn tool_names_round_trip_to_registry_ids() {
        assert_eq!(tool_id("firstcut_file_open"), "file.open");
        assert_eq!(tool_id("firstcut_settings.autoAdvance"), "settings.autoAdvance");
        assert_eq!(tool_id("rate.stars"), "rate.stars", "the raw dotted form also works");
    }
}
