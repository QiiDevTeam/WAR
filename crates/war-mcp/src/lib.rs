use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use war_runtime::{JsonlRequest, WarRuntime};

pub const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
pub const LATEST_LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";
const LEGACY_PROTOCOL_VERSIONS: [&str; 4] = [
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    LATEST_LEGACY_PROTOCOL_VERSION,
];
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const SERVER_NAME: &str = "windows-agent-runtime";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub trait WarBackend {
    fn call(&self, method: &str, params: Value) -> Result<Value, Value>;
}

impl WarBackend for WarRuntime {
    fn call(&self, method: &str, params: Value) -> Result<Value, Value> {
        let response = self.handle_jsonl(JsonlRequest {
            id: Value::Null,
            method: method.to_owned(),
            params,
            extra: serde_json::Map::new(),
        });
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (_, Some(error)) => Err(error),
            _ => Err(json!({"kind":"internal","message":"WAR returned an empty response"})),
        }
    }
}

pub fn serve_stdio<R: BufRead, W: Write, B: WarBackend>(
    backend: &B,
    mut input: R,
    mut output: W,
) -> std::io::Result<()> {
    let mut state = ConnectionState::default();
    loop {
        let line = match read_bounded_line(&mut input)? {
            BoundedLine::Eof => break,
            BoundedLine::TooLong => {
                write_message(
                    &mut output,
                    &rpc_error(
                        Value::Null,
                        -32600,
                        "MCP message exceeds the 1 MiB limit",
                        None,
                    ),
                )?;
                continue;
            }
            BoundedLine::Line(line) => line,
        };
        let incoming = match serde_json::from_slice::<Incoming>(&line) {
            Ok(message) if message.jsonrpc == "2.0" => message,
            Ok(_) => {
                write_message(
                    &mut output,
                    &rpc_error(Value::Null, -32600, "jsonrpc must be 2.0", None),
                )?;
                continue;
            }
            Err(error) => {
                write_message(
                    &mut output,
                    &rpc_error(
                        Value::Null,
                        -32700,
                        "Parse error",
                        Some(json!(error.to_string())),
                    ),
                )?;
                continue;
            }
        };

        let Some(id) = incoming.id else {
            state.handle_notification(&incoming.method);
            continue;
        };
        let response = state.handle_request(backend, id, &incoming.method, incoming.params);
        write_message(&mut output, &response)?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct Incoming {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Default)]
struct ConnectionState {
    legacy_version: Option<String>,
}

#[derive(Clone, Copy)]
enum Era<'a> {
    Modern,
    Legacy(&'a str),
}

impl ConnectionState {
    fn handle_notification(&mut self, method: &str) {
        match method {
            "notifications/initialized" | "notifications/cancelled" => {}
            _ => {}
        }
    }

    fn handle_request<B: WarBackend>(
        &mut self,
        backend: &B,
        id: Value,
        method: &str,
        params: Value,
    ) -> Value {
        if method == "initialize" {
            return self.initialize(id, &params);
        }
        if method == "server/discover" {
            return match modern_version(&params) {
                Ok(true) => rpc_result(id, modern_discovery()),
                Ok(false) => rpc_error(
                    id,
                    -32600,
                    "Missing io.modelcontextprotocol/protocolVersion metadata",
                    None,
                ),
                Err(error) => with_id(error, id),
            };
        }

        let era = match modern_version(&params) {
            Ok(true) => Era::Modern,
            Ok(false) => match self.legacy_version.as_deref() {
                Some(version) => Era::Legacy(version),
                None => {
                    return rpc_error(
                        id,
                        -32600,
                        "Request must carry modern MCP metadata or follow initialize",
                        None,
                    )
                }
            },
            Err(error) => return with_id(error, id),
        };

        match method {
            "ping" => rpc_result(id, complete(json!({}), era)),
            "tools/list" => rpc_result(id, tools_list(era)),
            "tools/call" => self.call_tool(backend, id, &params, era),
            _ => rpc_error(
                id,
                -32601,
                "Method not found",
                Some(json!({"method":method})),
            ),
        }
    }

    fn initialize(&mut self, id: Value, params: &Value) -> Value {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let selected = if LEGACY_PROTOCOL_VERSIONS.contains(&requested) {
            requested
        } else {
            LATEST_LEGACY_PROTOCOL_VERSION
        };
        self.legacy_version = Some(selected.to_owned());
        rpc_result(
            id,
            json!({
                "protocolVersion": selected,
                "capabilities": {"tools":{"listChanged":false}},
                "serverInfo": server_info(),
                "instructions": server_instructions()
            }),
        )
    }

    fn call_tool<B: WarBackend>(
        &self,
        backend: &B,
        id: Value,
        params: &Value,
        era: Era<'_>,
    ) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return rpc_error(id, -32602, "tools/call requires params.name", None);
        };
        let method = match name {
            "war.snapshot" => "snapshot",
            "war.inspect" => "inspect",
            "war.query" => "query",
            "war.wait" => "wait",
            "war.act" => "act",
            "war.send_message" => "send_message",
            _ => return rpc_error(id, -32602, "Unknown tool", Some(json!({"name":name}))),
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !arguments.is_object() {
            return rpc_error(id, -32602, "tool arguments must be an object", None);
        }
        let result = match backend.call(method, arguments) {
            Ok(value) => tool_result(value, false, era),
            Err(error) => tool_result(json!({"error":error}), true, era),
        };
        rpc_result(id, result)
    }
}

fn modern_version(params: &Value) -> Result<bool, Value> {
    let requested = params
        .get("_meta")
        .and_then(|meta| meta.get("io.modelcontextprotocol/protocolVersion"))
        .and_then(Value::as_str);
    match requested {
        Some(MODERN_PROTOCOL_VERSION) => Ok(true),
        Some(requested) => Err(rpc_error(
            Value::Null,
            -32022,
            "Unsupported protocol version",
            Some(json!({"supported":[MODERN_PROTOCOL_VERSION],"requested":requested})),
        )),
        None => Ok(false),
    }
}

fn modern_discovery() -> Value {
    complete(
        json!({
            "supportedVersions":[MODERN_PROTOCOL_VERSION],
            "capabilities":{"tools":{"listChanged":false}},
            "instructions":server_instructions(),
            "ttlMs":300_000,
            "cacheScope":"public"
        }),
        Era::Modern,
    )
}

fn tools_list(era: Era<'_>) -> Value {
    let mut value = json!({"tools":tool_definitions()});
    if matches!(era, Era::Modern) {
        value["ttlMs"] = json!(300_000);
        value["cacheScope"] = json!("public");
    }
    complete(value, era)
}

fn tool_result(value: Value, is_error: bool, era: Era<'_>) -> Value {
    let text = match era {
        Era::Modern | Era::Legacy("2025-06-18" | "2025-11-25") => {
            if is_error {
                serde_json::to_string(&value).unwrap_or_else(|_| "WAR tool failed".into())
            } else {
                "WAR result is available in structuredContent.".into()
            }
        }
        Era::Legacy(_) => serde_json::to_string(&value)
            .unwrap_or_else(|_| "WAR result serialization failed".into()),
    };
    let mut result = json!({
        "content":[{"type":"text","text":text}],
        "structuredContent":value,
        "isError":is_error
    });
    if matches!(era, Era::Legacy("2024-11-05" | "2025-03-26")) {
        result
            .as_object_mut()
            .expect("object")
            .remove("structuredContent");
    }
    complete(result, era)
}

fn complete(mut value: Value, era: Era<'_>) -> Value {
    if matches!(era, Era::Modern) {
        value["resultType"] = json!("complete");
        value["_meta"] = json!({"io.modelcontextprotocol/serverInfo":server_info()});
    }
    value
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name":"war.inspect",
            "title":"Inspect Windows UI",
            "description":"Observe and resolve one semantic Windows UI element in a single compact call. Prefer this over a full snapshot for targeting.",
            "inputSchema":{
                "type":"object",
                "properties":{
                    "scope":{"description":"focused_window (default), focused_subtree, desktop, or a tagged process/window/node scope"},
                    "name":{"type":"string"},
                    "automation_id":{"type":"string"},
                    "role":{"type":"string"},
                    "required_capabilities":{"type":"string"},
                    "fields":{"type":"array","items":{"enum":["automation_id","value","states","capabilities","bounds","lineage"]}}
                },
                "anyOf":[{"required":["name"]},{"required":["automation_id"]},{"required":["role"]}],
                "additionalProperties":false
            },
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
        }),
        json!({
            "name":"war.snapshot",
            "title":"Snapshot Windows UI",
            "description":"Return a bounded semantic UI snapshot. Use format=text for compact Agent output or structured for programmatic access.",
            "inputSchema":{
                "type":"object",
                "properties":{
                    "scope":{"description":"focused_window (default), focused_subtree, desktop, or a tagged process/window/node scope"},
                    "format":{"enum":["structured","text","both"]}
                },
                "additionalProperties":false
            },
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
        }),
        json!({
            "name":"war.query",
            "title":"Query Windows UI",
            "description":"Observe once and return only semantic nodes matching bounded server-side filters. Prefer this over snapshot when discovering a small set of candidates.",
            "inputSchema":{
                "type":"object",
                "properties":{
                    "scope":{"description":"focused_window (default), focused_subtree, desktop, or a tagged process/window/node scope"},
                    "role":{"type":"string"},
                    "name":{"type":"string"},
                    "name_contains":{"type":"string"},
                    "value":{"type":"string"},
                    "value_contains":{"type":"string"},
                    "automation_id":{"type":"string"},
                    "required_capabilities":{"type":"string"},
                    "enabled":{"type":"boolean"},
                    "limit":{"type":"integer","minimum":1,"maximum":50,"default":10},
                    "fields":{"type":"array","items":{"enum":["automation_id","value","states","capabilities","bounds","lineage"]}}
                },
                "anyOf":[
                    {"required":["role"]},{"required":["name"]},{"required":["name_contains"]},
                    {"required":["value"]},{"required":["value_contains"]},{"required":["automation_id"]},
                    {"required":["required_capabilities"]},{"required":["enabled"]}
                ],
                "additionalProperties":false
            },
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
        }),
        json!({
            "name":"war.wait",
            "title":"Wait for Windows UI",
            "description":"Poll inside WAR until bounded semantic filters match, returning only final evidence instead of every intermediate observation.",
            "inputSchema":{
                "type":"object",
                "properties":{
                    "scope":{"description":"focused_window (default), focused_subtree, desktop, or a tagged process/window/node scope"},
                    "role":{"type":"string"},
                    "name":{"type":"string"},
                    "name_contains":{"type":"string"},
                    "value":{"type":"string"},
                    "value_contains":{"type":"string"},
                    "automation_id":{"type":"string"},
                    "required_capabilities":{"type":"string"},
                    "enabled":{"type":"boolean"},
                    "limit":{"type":"integer","minimum":1,"maximum":50,"default":10},
                    "min_results":{"type":"integer","minimum":1,"maximum":50,"default":1},
                    "timeout_ms":{"type":"integer","minimum":1,"maximum":60000,"default":10000},
                    "poll_interval_ms":{"type":"integer","minimum":50,"maximum":1000,"default":250},
                    "fields":{"type":"array","items":{"enum":["automation_id","value","states","capabilities","bounds","lineage"]}}
                },
                "anyOf":[
                    {"required":["role"]},{"required":["name"]},{"required":["name_contains"]},
                    {"required":["value"]},{"required":["value_contains"]},{"required":["automation_id"]},
                    {"required":["required_capabilities"]},{"required":["enabled"]}
                ],
                "additionalProperties":false
            },
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
        }),
        json!({
            "name":"war.act",
            "title":"Act on Windows UI",
            "description":"Execute a guarded batch of semantic Windows UI actions. @refs require expected_session_id and expected_epoch from inspect/snapshot.",
            "inputSchema":{
                "type":"object",
                "properties":{
                    "expected_session_id":{"type":"string"},
                    "expected_epoch":{"type":"integer","minimum":0},
                    "timeout_ms":{"type":"integer","minimum":1,"maximum":60000},
                    "actions":{"type":"array","minItems":1,"items":{"type":"object"}},
                    "precondition":{"type":["object","null"]},
                    "postcondition":{"type":["object","null"]},
                    "stop_on_error":{"type":"boolean"},
                    "format":{"enum":["structured","text","both","summary"]}
                },
                "required":["actions"],
                "additionalProperties":false
            },
            "annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false}
        }),
        json!({
            "name":"war.send_message",
            "title":"Send Desktop Message",
            "description":"Run WAR's locally verified chat workflow: activate recipient, enter text, send, and verify the outgoing message.",
            "inputSchema":{
                "type":"object",
                "properties":{
                    "scope":{"description":"focused_window (default) or another WAR scope"},
                    "recipient":{"type":"string","minLength":1},
                    "text":{"type":"string"},
                    "list_name":{"type":"string"},
                    "send_label":{"type":"string"},
                    "timeout_ms":{"type":"integer","minimum":1,"maximum":60000}
                },
                "required":["recipient","text"],
                "additionalProperties":false
            },
            "annotations":{"readOnlyHint":false,"destructiveHint":false,"idempotentHint":false,"openWorldHint":true}
        }),
    ]
}

fn server_info() -> Value {
    json!({"name":SERVER_NAME,"version":SERVER_VERSION})
}

fn server_instructions() -> &'static str {
    "Prefer war.inspect for one known target, war.query for bounded discovery, and war.wait for readiness without wire polling. Carry session_id and epoch into war.act when using @refs. Treat dispatched_unverified as unverified until a postcondition succeeds."
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code":code,"message":message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc":"2.0","id":id,"error":error})
}

fn with_id(mut response: Value, id: Value) -> Value {
    response["id"] = id;
    response
}

fn write_message<W: Write>(output: &mut W, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)?;
    output.flush()
}

enum BoundedLine {
    Eof,
    Line(Vec<u8>),
    TooLong,
}

fn read_bounded_line<R: BufRead>(input: &mut R) -> std::io::Result<BoundedLine> {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok(if too_long {
                BoundedLine::TooLong
            } else if line.is_empty() {
                BoundedLine::Eof
            } else {
                BoundedLine::Line(line)
            });
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |index| index + 1);
        if !too_long {
            if line.len() + take <= MAX_MESSAGE_BYTES {
                line.extend_from_slice(&available[..take]);
            } else {
                too_long = true;
                line.clear();
            }
        }
        input.consume(take);
        if newline.is_some() {
            return Ok(if too_long {
                BoundedLine::TooLong
            } else {
                BoundedLine::Line(line)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct StubBackend;

    impl WarBackend for StubBackend {
        fn call(&self, method: &str, params: Value) -> Result<Value, Value> {
            Ok(json!({"method":method,"params":params}))
        }
    }

    fn serve(input: &str) -> Vec<Value> {
        let mut output = Vec::new();
        serve_stdio(&StubBackend, Cursor::new(input.as_bytes()), &mut output).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn modern_meta() -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion":MODERN_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientInfo":{"name":"test","version":"1"},
            "io.modelcontextprotocol/clientCapabilities":{}
        })
    }

    #[test]
    fn serves_modern_discovery_list_and_structured_tool_result() {
        let meta = modern_meta();
        let input = format!(
            "{}\n{}\n{}\n{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":meta}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":modern_meta()}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"war.snapshot","arguments":{"format":"text"},"_meta":modern_meta()}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"war.wait","arguments":{"role":"window"},"_meta":modern_meta()}})
        );
        let responses = serve(&input);
        assert_eq!(responses.len(), 4);
        assert_eq!(
            responses[0]["result"]["supportedVersions"][0],
            MODERN_PROTOCOL_VERSION
        );
        assert_eq!(responses[1]["result"]["tools"].as_array().unwrap().len(), 6);
        assert_eq!(responses[2]["result"]["resultType"], "complete");
        assert_eq!(
            responses[2]["result"]["structuredContent"]["method"],
            "snapshot"
        );
        assert_eq!(
            responses[3]["result"]["structuredContent"]["method"],
            "wait"
        );
    }

    #[test]
    fn serves_legacy_initialize_and_ignores_initialized_notification() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"1\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"war.inspect\",\"arguments\":{\"name\":\"Save\"}}}\n"
        );
        let responses = serve(input);
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0]["result"]["protocolVersion"], "2025-11-25");
        assert!(responses[1]["result"].get("resultType").is_none());
        assert_eq!(
            responses[2]["result"]["structuredContent"]["method"],
            "inspect"
        );
    }

    #[test]
    fn rejects_modern_request_without_metadata_before_initialize() {
        let responses =
            serve("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n");
        assert_eq!(responses[0]["error"]["code"], -32600);
    }

    #[test]
    fn rejects_unknown_modern_version_with_supported_list() {
        let responses = serve(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{\"_meta\":{\"io.modelcontextprotocol/protocolVersion\":\"1900-01-01\"}}}\n",
        );
        assert_eq!(responses[0]["error"]["code"], -32022);
        assert_eq!(
            responses[0]["error"]["data"]["supported"][0],
            MODERN_PROTOCOL_VERSION
        );
    }

    #[test]
    fn does_not_treat_unsupported_modern_request_as_legacy_after_initialize() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{\"_meta\":{\"io.modelcontextprotocol/protocolVersion\":\"1900-01-01\"}}}\n"
        );
        let responses = serve(input);
        assert_eq!(responses[1]["error"]["code"], -32022);
    }
}
