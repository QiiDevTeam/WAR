# MCP Stdio server

Run the server from a release build:

```powershell
cargo build --release
.\target\release\war.exe mcp
```

An MCP client configuration should launch the absolute binary path without extra shell text:

```json
{
  "mcpServers": {
    "war": {
      "command": "C:\\Users\\fish_\\Desktop\\ComputerUse\\target\\release\\war.exe",
      "args": ["mcp"]
    }
  }
}
```

The server uses newline-delimited UTF-8 JSON-RPC on stdin/stdout. It never writes logs or human-readable banners to stdout; a client can close stdin for graceful shutdown. Messages larger than 1 MiB are rejected and drained without killing the process.

## Compatibility

The adapter supports both MCP protocol eras:

- `2026-07-28`: `server/discover`, per-request `io.modelcontextprotocol/protocolVersion`, `resultType`, response cache hints, and structured tool content.
- `2024-11-05` through `2025-11-25`: `initialize` / `notifications/initialized` and the negotiated legacy response shapes.

Modern Stdio messages remain one JSON-RPC object per line. Modern requests must carry their protocol version in `params._meta`; unsupported versions return `-32022` with the supported version list. The tool list is stable and advertised with `listChanged: false`.

## Tools

| Tool | Purpose | Mutation |
| --- | --- | --- |
| `war.inspect` | Observe and resolve one element with selected projected fields | Read-only |
| `war.query` | Observe once and return up to 50 server-filtered candidates | Read-only |
| `war.wait` | Poll locally until server-filtered candidates appear | Read-only |
| `war.snapshot` | Return a bounded semantic tree in structured or compact text form | Read-only |
| `war.act` | Execute guarded action batches and optional postconditions | May mutate desktop state |
| `war.send_message` | Run the locally verified recipient/editor/send workflow | Sends a message |

Prefer `war.inspect` for one known target. Use `war.query` for bounded discovery by role, exact/contained name or value, automation ID, capability, and enabled state. Use `war.wait` for the same filters when UI state is still loading; intermediate observations stay inside WAR, and only final matches, observation count, and elapsed time cross the wire. Both tools cap results at 50 and can project `bounds`, `value`, `states`, `capabilities`, or `lineage` only when needed.

For example, this discovers at most five invokable video links without returning the rest of the browser tree:

```json
{"role":"link","value_contains":"bilibili.com/video/","required_capabilities":"INVOKE","limit":5,"fields":["value"]}
```

An `@ref` is local to one WAR runtime snapshot. Every `war.act` call containing refs must copy both `session_id` and `epoch` from the observation into `expected_session_id` and `expected_epoch`. A modern MCP connection is stateless at the protocol layer, but these explicit values preserve WAR's stale-reference protection.

An action is `verified` only when its requested postcondition was observed. `dispatched_unverified` means Windows accepted the input but WAR did not prove the intended effect. Callers should not report success from dispatch alone.

## Minimal wire smoke test

Modern discovery:

```json
{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"smoke","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}}}
```

Legacy initialization:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}
```

The implementation follows the official MCP [Stdio transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio), [version compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning), [server discovery](https://modelcontextprotocol.io/specification/2026-07-28/server/discover), and [tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools) specifications.
