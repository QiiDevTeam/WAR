use crate::WarRuntime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::thread;
use std::time::{Duration, Instant};
use war_core::{ancestor_chain, resolve_target};
use war_protocol::{
    ActionBatch, Capabilities, Role, SemanticNode, SemanticSnapshot, SemanticTarget,
    SendMessageRequest, SnapshotScope, Target, WarError,
};
use war_semantic::{render_delta, render_snapshot};

const MAX_JSONL_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct JsonlRequest {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonlResponse {
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl WarRuntime {
    pub fn serve_jsonl<R: BufRead, W: Write>(
        &self,
        mut input: R,
        mut output: W,
    ) -> std::io::Result<()> {
        loop {
            let response = match read_bounded_line(&mut input)? {
                BoundedLine::Eof => break,
                BoundedLine::TooLong => JsonlResponse {
                    id: Value::Null,
                    result: None,
                    error: Some(json!({
                        "kind":"request_too_large",
                        "message":format!("JSONL request exceeds {MAX_JSONL_REQUEST_BYTES} bytes")
                    })),
                },
                BoundedLine::Line(line) => match serde_json::from_slice::<JsonlRequest>(&line) {
                    Ok(request) => self.handle_jsonl(request),
                    Err(error) => JsonlResponse {
                        id: Value::Null,
                        result: None,
                        error: Some(json!({"kind":"parse_error","message":error.to_string()})),
                    },
                },
            };
            serde_json::to_writer(&mut output, &response)?;
            writeln!(output)?;
            output.flush()?;
        }
        Ok(())
    }

    pub fn handle_jsonl(&self, request: JsonlRequest) -> JsonlResponse {
        let JsonlRequest {
            id,
            method,
            params,
            extra,
        } = request;
        let params = merge_params(params, extra);
        let result: Result<Value, WarError> = (|| match method.as_str() {
            "snapshot" => {
                let scope = params
                    .get("scope")
                    .cloned()
                    .map(parse_scope)
                    .transpose()?
                    .unwrap_or_default();
                let snapshot = self.observe(scope)?;
                match output_format(&params)? {
                    OutputFormat::Structured => Ok(json!({"snapshot":snapshot})),
                    OutputFormat::Text => Ok(
                        json!({"session_id":snapshot.session_id,"epoch":snapshot.epoch,"text":render_snapshot(&snapshot)}),
                    ),
                    OutputFormat::Both => {
                        Ok(json!({"snapshot":snapshot,"text":render_snapshot(&snapshot)}))
                    }
                    OutputFormat::Summary => Err(WarError::InvalidRequest(
                        "snapshot does not support summary format".into(),
                    )),
                }
            }
            "inspect" => {
                let scope = params
                    .get("scope")
                    .cloned()
                    .map(parse_scope)
                    .transpose()?
                    .unwrap_or_default();
                let role = params
                    .get("role")
                    .cloned()
                    .map(serde_json::from_value::<Role>)
                    .transpose()
                    .map_err(|e| WarError::InvalidRequest(e.to_string()))?;
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let automation_id = params
                    .get("automation_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if name.is_none() && automation_id.is_none() && role.is_none() {
                    return Err(WarError::InvalidRequest(
                        "inspect requires name, automation_id, or role".into(),
                    ));
                }
                let required_capabilities = params
                    .get("required_capabilities")
                    .cloned()
                    .map(serde_json::from_value::<Capabilities>)
                    .transpose()
                    .map_err(|e| WarError::InvalidRequest(e.to_string()))?
                    .unwrap_or_default();
                let fields = projection_fields(&params, "inspect")?;
                let snapshot = self.observe(scope)?;
                let target = Target::Semantic(SemanticTarget {
                    role,
                    name,
                    automation_id,
                    required_capabilities,
                    ancestor: None,
                });
                let resolved = resolve_target(&snapshot, &target)?;
                let node = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.id == resolved.id)
                    .ok_or_else(|| WarError::TargetNotFound(format!("@{}", resolved.id)))?;
                let mut projected = serde_json::Map::from_iter([
                    ("id".into(), json!(node.id)),
                    ("role".into(), json!(node.role)),
                    ("name".into(), json!(node.name)),
                ]);
                if fields.contains("automation_id") {
                    projected.insert("automation_id".into(), json!(node.automation_id));
                }
                if fields.contains("value") {
                    projected.insert("value".into(), json!(node.value));
                }
                if fields.contains("states") {
                    projected.insert("states".into(), json!(node.states));
                }
                if fields.contains("capabilities") {
                    projected.insert("capabilities".into(), json!(node.capabilities));
                }
                if fields.contains("bounds") {
                    projected.insert("bounds".into(), json!(self.current_bounds(node.id)?));
                }
                let lineage = fields
                    .contains("lineage")
                    .then(|| {
                        ancestor_chain(&snapshot, node.id).map(|nodes| {
                            nodes
                                .into_iter()
                                .map(|node| json!({"id":node.id,"role":node.role,"name":node.name}))
                                .collect::<Vec<_>>()
                        })
                    })
                    .transpose()?;
                let mut result = serde_json::Map::from_iter([
                    ("session_id".into(), json!(snapshot.session_id)),
                    ("epoch".into(), json!(snapshot.epoch)),
                    ("confidence".into(), json!(resolved.confidence)),
                    ("node".into(), Value::Object(projected)),
                ]);
                if let Some(lineage) = lineage {
                    result.insert("lineage".into(), json!(lineage));
                }
                Ok(Value::Object(result))
            }
            "query" => {
                let scope = params
                    .get("scope")
                    .cloned()
                    .map(parse_scope)
                    .transpose()?
                    .unwrap_or_default();
                let query = parse_query(&params, "query")?;
                let snapshot = self.observe(scope)?;
                Ok(query_snapshot(self, &snapshot, &query)?.value)
            }
            "wait" => {
                let scope = params
                    .get("scope")
                    .cloned()
                    .map(parse_scope)
                    .transpose()?
                    .unwrap_or_default();
                let query = parse_query(&params, "wait")?;
                let timeout_ms = bounded_u64(&params, "timeout_ms", 10_000, 1, 60_000)?;
                let poll_interval_ms = bounded_u64(&params, "poll_interval_ms", 250, 50, 1_000)?;
                let min_results = bounded_usize(&params, "min_results", 1, 1, query.limit)?;
                self.wait_for_query(
                    scope,
                    &query,
                    min_results,
                    Duration::from_millis(timeout_ms),
                    Duration::from_millis(poll_interval_ms),
                )
            }
            "find" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| WarError::InvalidRequest("find requires params.name".into()))?;
                let role = params
                    .get("role")
                    .cloned()
                    .map(serde_json::from_value::<Role>)
                    .transpose()
                    .map_err(|e| WarError::InvalidRequest(e.to_string()))?;
                let required_capabilities = params
                    .get("required_capabilities")
                    .cloned()
                    .map(serde_json::from_value::<Capabilities>)
                    .transpose()
                    .map_err(|e| WarError::InvalidRequest(e.to_string()))?
                    .unwrap_or_default();
                let snapshot = self.current()?.ok_or_else(|| {
                    WarError::InvalidRequest("take a snapshot before find".into())
                })?;
                let target = Target::Semantic(SemanticTarget {
                    role,
                    name: Some(name.into()),
                    automation_id: None,
                    required_capabilities,
                    ancestor: None,
                });
                let resolved = resolve_target(&snapshot, &target)?;
                let lineage = ancestor_chain(&snapshot, resolved.id)?
                    .into_iter()
                    .map(|node| json!({"id":node.id,"role":node.role,"name":node.name,"capabilities":node.capabilities}))
                    .collect::<Vec<_>>();
                Ok(
                    json!({"session_id":snapshot.session_id,"epoch":snapshot.epoch,"id":resolved.id,"confidence":resolved.confidence,"lineage":lineage}),
                )
            }
            "act" => {
                let batch: ActionBatch = serde_json::from_value(params.clone())
                    .map_err(|e| WarError::InvalidRequest(e.to_string()))?;
                if batch.uses_refs()
                    && (batch.expected_session_id.is_none() || batch.expected_epoch.is_none())
                {
                    return Err(WarError::InvalidRequest(
                        "act with session refs requires expected_session_id and expected_epoch from the snapshot".into(),
                    ));
                }
                let report = self.act(&batch)?;
                let status = match report.outcome.status {
                    war_core::ExecutionStatus::Verified => "verified",
                    war_core::ExecutionStatus::DispatchedUnverified => "dispatched_unverified",
                    war_core::ExecutionStatus::Failed => "failed",
                };
                let effect = match report.outcome.effect {
                    war_core::ObservedEffect::Changed => "changed",
                    war_core::ObservedEffect::NoChange => "no_change",
                };
                let mut result = json!({"status":status,"effect":effect,"actions":report.outcome.actions.iter().map(|a| json!({"index":a.index,"dispatched":a.dispatched,"method":a.method,"fallback_used":a.fallback_used,"error":a.error})).collect::<Vec<_>>(),"verified":report.outcome.verified,"observations":report.observations});
                match output_format(&params)? {
                    OutputFormat::Structured => result["delta"] = json!(report.delta),
                    OutputFormat::Text => result["text"] = json!(render_delta(&report.delta)),
                    OutputFormat::Both => {
                        result["delta"] = json!(report.delta);
                        result["text"] = json!(render_delta(&report.delta));
                    }
                    OutputFormat::Summary => {}
                }
                Ok(result)
            }
            "send_message" => {
                let mut workflow_params = params.clone();
                if let Some(scope) = params.get("scope").cloned() {
                    workflow_params["scope"] = serde_json::to_value(parse_scope(scope)?)
                        .map_err(|e| WarError::InvalidRequest(e.to_string()))?;
                }
                let request: SendMessageRequest = serde_json::from_value(workflow_params)
                    .map_err(|e| WarError::InvalidRequest(e.to_string()))?;
                Ok(serde_json::to_value(self.send_message(&request)?)
                    .map_err(|e| WarError::Provider(e.to_string()))?)
            }
            other => Err(WarError::InvalidRequest(format!("unknown method {other}"))),
        })();
        match result {
            Ok(value) => JsonlResponse {
                id,
                result: Some(value),
                error: None,
            },
            Err(error) => {
                JsonlResponse {
                    id,
                    result: None,
                    error: Some(serde_json::to_value(error).unwrap_or_else(
                        |e| json!({"kind":"serialization","message":e.to_string()}),
                    )),
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum OutputFormat {
    Structured,
    Text,
    Both,
    Summary,
}

fn output_format(params: &Value) -> Result<OutputFormat, WarError> {
    match params
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("structured")
    {
        "structured" => Ok(OutputFormat::Structured),
        "text" => Ok(OutputFormat::Text),
        "both" => Ok(OutputFormat::Both),
        "summary" => Ok(OutputFormat::Summary),
        other => Err(WarError::InvalidRequest(format!(
            "unknown output format {other:?}"
        ))),
    }
}

fn projection_fields<'a>(params: &'a Value, operation: &str) -> Result<HashSet<&'a str>, WarError> {
    const ALLOWED: [&str; 6] = [
        "automation_id",
        "value",
        "states",
        "capabilities",
        "bounds",
        "lineage",
    ];
    let Some(fields) = params.get("fields") else {
        return Ok(HashSet::new());
    };
    let values = fields
        .as_array()
        .ok_or_else(|| WarError::InvalidRequest(format!("{operation} fields must be an array")))?;
    let mut selected = HashSet::new();
    for value in values {
        let field = value.as_str().ok_or_else(|| {
            WarError::InvalidRequest(format!("{operation} field must be a string"))
        })?;
        if !ALLOWED.contains(&field) {
            return Err(WarError::InvalidRequest(format!(
                "unknown {operation} field {field:?}"
            )));
        }
        selected.insert(field);
    }
    Ok(selected)
}

struct QuerySpec<'a> {
    role: Option<Role>,
    name: Option<&'a str>,
    name_contains: Option<&'a str>,
    value: Option<&'a str>,
    value_contains: Option<&'a str>,
    automation_id: Option<&'a str>,
    required_capabilities: Capabilities,
    enabled: Option<bool>,
    limit: usize,
    fields: HashSet<&'a str>,
}

struct QueryOutcome {
    value: Value,
    returned: usize,
}

fn parse_query<'a>(params: &'a Value, operation: &str) -> Result<QuerySpec<'a>, WarError> {
    let role = params
        .get("role")
        .cloned()
        .map(serde_json::from_value::<Role>)
        .transpose()
        .map_err(|error| WarError::InvalidRequest(error.to_string()))?;
    let required_capabilities = params
        .get("required_capabilities")
        .cloned()
        .map(serde_json::from_value::<Capabilities>)
        .transpose()
        .map_err(|error| WarError::InvalidRequest(error.to_string()))?
        .unwrap_or_default();
    let enabled = params
        .get("enabled")
        .map(|value| {
            value.as_bool().ok_or_else(|| {
                WarError::InvalidRequest(format!("{operation} enabled must be a boolean"))
            })
        })
        .transpose()?;
    let spec = QuerySpec {
        role,
        name: optional_string(params, "name", operation)?,
        name_contains: optional_string(params, "name_contains", operation)?,
        value: optional_string(params, "value", operation)?,
        value_contains: optional_string(params, "value_contains", operation)?,
        automation_id: optional_string(params, "automation_id", operation)?,
        required_capabilities,
        enabled,
        limit: bounded_usize(params, "limit", 10, 1, 50)?,
        fields: projection_fields(params, operation)?,
    };
    if spec.role.is_none()
        && spec.name.is_none()
        && spec.name_contains.is_none()
        && spec.value.is_none()
        && spec.value_contains.is_none()
        && spec.automation_id.is_none()
        && spec.required_capabilities.is_empty()
        && spec.enabled.is_none()
    {
        return Err(WarError::InvalidRequest(format!(
            "{operation} requires at least one selector"
        )));
    }
    Ok(spec)
}

fn optional_string<'a>(
    params: &'a Value,
    field: &str,
    operation: &str,
) -> Result<Option<&'a str>, WarError> {
    params
        .get(field)
        .map(|value| {
            value.as_str().ok_or_else(|| {
                WarError::InvalidRequest(format!("{operation} {field} must be a string"))
            })
        })
        .transpose()
}

fn bounded_u64(
    params: &Value,
    field: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, WarError> {
    let value = params
        .get(field)
        .map(Value::as_u64)
        .unwrap_or(Some(default));
    match value {
        Some(value) if (minimum..=maximum).contains(&value) => Ok(value),
        _ => Err(WarError::InvalidRequest(format!(
            "{field} must be an integer from {minimum} through {maximum}"
        ))),
    }
}

fn bounded_usize(
    params: &Value,
    field: &str,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, WarError> {
    bounded_u64(
        params,
        field,
        default as u64,
        minimum as u64,
        maximum as u64,
    )
    .map(|value| value as usize)
}

fn query_snapshot(
    runtime: &WarRuntime,
    snapshot: &SemanticSnapshot,
    query: &QuerySpec<'_>,
) -> Result<QueryOutcome, WarError> {
    let matching = snapshot
        .nodes
        .iter()
        .filter(|node| node_matches(node, query))
        .collect::<Vec<_>>();
    let total_matches = matching.len();
    let mut matches = Vec::with_capacity(total_matches.min(query.limit));
    for node in matching.into_iter().take(query.limit) {
        matches.push(project_node(runtime, snapshot, node, &query.fields)?);
    }
    let returned = matches.len();
    Ok(QueryOutcome {
        value: json!({
            "session_id":snapshot.session_id,
            "epoch":snapshot.epoch,
            "matches":matches,
            "returned":returned,
            "total_matches":total_matches,
            "truncated":total_matches > returned || snapshot.truncated
        }),
        returned,
    })
}

fn node_matches(node: &SemanticNode, query: &QuerySpec<'_>) -> bool {
    query.role.map_or(true, |role| node.role == role)
        && query
            .name
            .map_or(true, |expected| text_equals(node.name.as_deref(), expected))
        && query.name_contains.map_or(true, |expected| {
            text_contains(node.name.as_deref(), expected)
        })
        && query.value.map_or(true, |expected| {
            text_equals(node.value.as_deref(), expected)
        })
        && query.value_contains.map_or(true, |expected| {
            text_contains(node.value.as_deref(), expected)
        })
        && query.automation_id.map_or(true, |expected| {
            text_equals(node.automation_id.as_deref(), expected)
        })
        && node.capabilities.contains(query.required_capabilities)
        && query
            .enabled
            .map_or(true, |enabled| node.states.enabled == enabled)
}

fn text_equals(actual: Option<&str>, expected: &str) -> bool {
    actual.is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
}

fn text_contains(actual: Option<&str>, expected: &str) -> bool {
    actual.is_some_and(|actual| actual.to_lowercase().contains(&expected.to_lowercase()))
}

fn project_node(
    runtime: &WarRuntime,
    snapshot: &SemanticSnapshot,
    node: &SemanticNode,
    fields: &HashSet<&str>,
) -> Result<Value, WarError> {
    let mut projected = serde_json::Map::from_iter([
        ("id".into(), json!(node.id)),
        ("role".into(), json!(node.role)),
        ("name".into(), json!(node.name)),
    ]);
    if fields.contains("automation_id") {
        projected.insert("automation_id".into(), json!(node.automation_id));
    }
    if fields.contains("value") {
        projected.insert("value".into(), json!(node.value));
    }
    if fields.contains("states") {
        projected.insert("states".into(), json!(node.states));
    }
    if fields.contains("capabilities") {
        projected.insert("capabilities".into(), json!(node.capabilities));
    }
    if fields.contains("bounds") {
        projected.insert("bounds".into(), json!(runtime.current_bounds(node.id)?));
    }
    if fields.contains("lineage") {
        let lineage = ancestor_chain(snapshot, node.id)?
            .into_iter()
            .map(|ancestor| json!({"id":ancestor.id,"role":ancestor.role,"name":ancestor.name}))
            .collect::<Vec<_>>();
        projected.insert("lineage".into(), json!(lineage));
    }
    Ok(Value::Object(projected))
}

impl WarRuntime {
    fn wait_for_query(
        &self,
        scope: SnapshotScope,
        query: &QuerySpec<'_>,
        min_results: usize,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<Value, WarError> {
        let started = Instant::now();
        let deadline = started + timeout;
        let subscription = self.provider.subscribe().ok();
        let mut observations = 0usize;
        loop {
            observations += 1;
            match self.observe(scope.clone()) {
                Ok(snapshot) => {
                    let mut outcome = query_snapshot(self, &snapshot, query)?;
                    if outcome.returned >= min_results {
                        outcome.value["observations"] = json!(observations);
                        outcome.value["elapsed_ms"] = json!(started.elapsed().as_millis());
                        return Ok(outcome.value);
                    }
                }
                Err(WarError::Provider(_) | WarError::TargetNotFound(_)) => {}
                Err(error) => return Err(error),
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(WarError::Timeout {
                    operation: "wait for semantic query".into(),
                    timeout_ms: timeout.as_millis().min(u64::MAX as u128) as u64,
                });
            }
            let delay = poll_interval.min(deadline.saturating_duration_since(now));
            if let Some(subscription) = &subscription {
                let _ = subscription.receiver().recv_timeout(delay);
            } else {
                thread::sleep(delay);
            }
        }
    }
}

fn merge_params(params: Value, extra: serde_json::Map<String, Value>) -> Value {
    let mut merged = match params {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => return other,
    };
    for (key, value) in extra {
        merged.entry(key).or_insert(value);
    }
    Value::Object(merged)
}

fn parse_scope(value: Value) -> Result<war_protocol::SnapshotScope, WarError> {
    if let Some(scope) = value.as_str() {
        return match scope {
            "desktop" => Ok(war_protocol::SnapshotScope::Desktop),
            "focused_window" => Ok(war_protocol::SnapshotScope::FocusedWindow),
            "focused_subtree" => Ok(war_protocol::SnapshotScope::FocusedSubtree),
            other => Err(WarError::InvalidRequest(format!(
                "unknown string scope {other:?}"
            ))),
        };
    }
    serde_json::from_value(value).map_err(|error| WarError::InvalidRequest(error.to_string()))
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
            if line.len() + take <= MAX_JSONL_REQUEST_BYTES {
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

    #[test]
    fn bounded_reader_drains_oversize_line_and_recovers() {
        let mut bytes = vec![b'x'; MAX_JSONL_REQUEST_BYTES + 1];
        bytes.extend_from_slice(b"\n{}\n");
        let mut input = Cursor::new(bytes);
        assert!(matches!(
            read_bounded_line(&mut input).unwrap(),
            BoundedLine::TooLong
        ));
        assert!(matches!(
            read_bounded_line(&mut input).unwrap(),
            BoundedLine::Line(line) if line == b"{}\n"
        ));
    }
}
