# Protocol

The first transport is newline-delimited JSON on stdin/stdout. Each request and response occupies one line.

```json
{"id":1,"method":"snapshot","params":{"scope":{"kind":"focused_window"}}}
{"id":1,"result":{"snapshot":{"session_id":"7e28-18f...-1","epoch":42}}}
```

`snapshot` and `act` default to `"format":"structured"`. Use `"format":"text"` for rendered text only or `"format":"both"` for compatibility/debugging. For actions, `"format":"summary"` omits the delta while retaining dispatch, effect, verification, and observation metadata. The default never duplicates the same observation in two representations.

`params.scope` is optional and defaults to `focused_window`. Scope uses Serde's tagged shape: `{"kind":"process","value":1234}`, `{"kind":"window","value":123}`, `{"kind":"node","value":12}`, or `{"kind":"focused_subtree"}`.

For compatibility with the compact examples, request fields may also be flat: `{"id":1,"method":"snapshot","scope":"focused_window"}`. The strings `desktop`, `focused_window`, and `focused_subtree` are accepted directly; process/window/node scopes retain the tagged form because they require an ID. If both forms are present, fields inside `params` take precedence.

Find after a snapshot:

```json
{"id":2,"method":"find","params":{"name":"Save","role":"button","required_capabilities":"INVOKE"}}
```

`find` returns the resolved ID and confidence together with `session_id`, `epoch`, and a compact root-to-node `lineage`. The lineage exposes actionable ancestors without requiring another full snapshot. `required_capabilities` is an optional bitflags string such as `"CLICK"` or `"SET_VALUE | GET_VALUE"`.

For normal Agent targeting, `inspect` combines observation and resolution and returns only requested fields:

```json
{"id":2,"method":"inspect","params":{"scope":"focused_window","automation_id":"image","fields":["bounds","capabilities","lineage"]}}
{"id":2,"result":{"session_id":"7e28-18f...-1","epoch":43,"confidence":1.0,"node":{"id":117,"role":"group","name":"在画布上使用 画笔 工具","bounds":{"left":510.0,"top":462.0,"width":1536.0,"height":907.0},"capabilities":"POINTER_GESTURE"},"lineage":[...]}}
```

Allowed projected fields are `automation_id`, `value`, `states`, `capabilities`, `bounds`, and `lineage`. With no `fields`, the response contains only ref guards, confidence, ID, role, and name.

For discovery, `query` observes once and filters inside WAR rather than returning unrelated nodes:

```json
{"id":3,"method":"query","params":{"scope":"focused_window","role":"link","value_contains":"bilibili.com/video/","required_capabilities":"INVOKE","limit":5,"fields":["value"]}}
{"id":3,"result":{"session_id":"7e28-18f...-1","epoch":44,"matches":[{"id":91,"role":"link","name":"Example","value":"https://www.bilibili.com/video/BV..."}],"returned":1,"total_matches":1,"truncated":false}}
```

Filters are combined with AND. Supported filters are `role`, exact `name` / `value` / `automation_id`, case-insensitive `name_contains` / `value_contains`, `required_capabilities`, and `enabled`. `limit` defaults to 10 and is capped at 50.

`wait` accepts the same filters and fields, but performs readiness polling inside the runtime until `min_results` are present:

```json
{"id":4,"method":"wait","params":{"role":"window","name_contains":"Example","limit":1,"timeout_ms":30000,"poll_interval_ms":250}}
```

`timeout_ms` is limited to 1–60000 ms and `poll_interval_ms` to 50–1000 ms. A successful response additionally reports `observations` and `elapsed_ms`; timeout is a structured `timeout` error. Refs returned by `query` or `wait` use the same `session_id` and `epoch` guards as snapshots and inspection.

Actions use compact refs and can be batched:

```json
{"id":3,"method":"act","params":{"expected_session_id":"7e28-18f...-1","expected_epoch":42,"timeout_ms":15000,"actions":[{"invoke":"@12"},{"set_value":{"target":"@18","value":"hello"}}],"postcondition":{"type":"gone","target":"@17"},"stop_on_error":true}}
```

Targets may be a ref string, `{"semantic":{"role":"button","name":"Save","automation_id":null,"required_capabilities":"INVOKE","ancestor":null}}`, or `{"coordinates":{"x":10,"y":20}}`. Coordinate targeting is an explicit fallback, never the stable identity.

Continuous element-relative input uses normalized coordinates. Every point must be inside `0.0..=1.0`; the runtime resolves the target's latest bounds, validates the foreground/hit window, and guarantees button release on failure:

```json
{"pointer_gesture":{"target":"@117","button":"left","points":[{"x":0.40,"y":0.45},{"x":0.45,"y":0.50},{"x":0.50,"y":0.45}],"duration_ms":120}}
```

Any JSONL action or condition containing an `@ref` must include both `expected_session_id` and `expected_epoch`. Mismatches return `stale_session` or `stale_snapshot` before execution. Semantic and coordinate targets do not require these guards. Requests are capped at 1 MiB per line; an oversized line is drained and returns `request_too_large` without terminating the service.

An action result uses `status: "verified"` only when its postcondition was observed. Without a postcondition, successful provider dispatch is reported as `status: "dispatched_unverified"`; `effect` independently says `changed` or `no_change`. The per-action field is named `dispatched`, not `success`. Postconditions are settled by event-assisted polling until satisfied or the batch deadline, rather than by a single immediate snapshot.

## High-level message workflow

```json
{"id":4,"method":"send_message","params":{"scope":"focused_window","recipient":"Amiracle","text":"test","list_name":"会话列表","send_label":"发送","timeout_ms":15000}}
```

`scope`, `list_name`, `send_label`, and `timeout_ms` are optional. The runtime resolves the recipient text under the named conversation-list ancestor, climbs to the nearest invokable/clickable ancestor, detects a no-op Invoke and retries with Click, waits for a writable editor named for the recipient, verifies the draft and enabled Send button, invokes Send, and verifies both an empty composer and an increased visible outgoing-message count. Success returns only a compact report; intermediate snapshots and deltas remain local.

Errors are structured and correlated with the request ID. Provider calls return within the configured timeout (3 seconds by default). Action batches default to a 15-second overall deadline; `timeout_ms` may set 1–60000 ms. `watch` is exposed as the streaming CLI command in v0.1; long-lived adapters should call the runtime subscription API directly.

Snapshots include an opaque `session_id`, `total_nodes`, and `truncated`. The default semantic budget is 256 nodes and always preserves the ancestor chain of each retained node. Provider strings are bounded before entering the semantic layer, and UIA password values are redacted.
