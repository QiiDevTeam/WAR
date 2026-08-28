# Embedding WAR in an application

WAR is best embedded as a private sidecar process. Ship `war.exe` with the application, start `war.exe mcp` without a shell, redirect stdin/stdout/stderr, and keep the process alive for the Agent session.

This arrangement keeps transport private, avoids a listening port, and isolates the host application from UIA/COM provider failures.

## Lifecycle

1. Resolve an absolute, trusted `war.exe` path inside the installation directory.
2. Start `war.exe mcp` with shell execution disabled and all three standard streams redirected.
3. Continuously drain stdout as UTF-8 JSONL and stderr as diagnostics.
4. Send `server/discover`, followed by `tools/list` and `tools/call` requests.
5. Correlate responses by JSON-RPC `id`; allow concurrent requests only after correlation is implemented.
6. Keep one WAR process for one Agent session. If it restarts, discard every old ref, `session_id`, and `epoch`.
7. On shutdown, close stdin, wait briefly for a graceful exit, then kill only that child if necessary.

Do not launch through `cmd.exe`, PowerShell, `shell: true`, or an untrusted path. Sign both the host application and the shipped WAR binary for production distribution.

## Dart and Flutter

Dart's `Process.start` can host the sidecar directly:

```dart
import 'dart:async';
import 'dart:convert';
import 'dart:io';

class WarMcpClient {
  Process? _process;
  StreamSubscription<String>? _stdout;
  StreamSubscription<String>? _stderr;
  var _nextId = 0;
  final _pending = <int, Completer<Map<String, dynamic>>>{};

  static const protocolVersion = '2026-07-28';

  Future<void> start(String absoluteWarPath) async {
    _process = await Process.start(
      absoluteWarPath,
      const ['mcp'],
      runInShell: false,
      mode: ProcessStartMode.normal,
    );

    _stdout = _process!.stdout
        .transform(utf8.decoder)
        .transform(const LineSplitter())
        .listen(_onLine);
    _stderr = _process!.stderr
        .transform(utf8.decoder)
        .transform(const LineSplitter())
        .listen((line) => stderr.writeln('[WAR] $line'));

    _process!.exitCode.then((code) {
      final error = StateError('WAR exited with code $code');
      for (final request in _pending.values) {
        if (!request.isCompleted) request.completeError(error);
      }
      _pending.clear();
    });

    await request('server/discover', {
      '_meta': {
        'io.modelcontextprotocol/protocolVersion': protocolVersion,
        'io.modelcontextprotocol/clientInfo': {
          'name': 'my-flutter-app',
          'version': '1.0.0',
        },
        'io.modelcontextprotocol/clientCapabilities': {},
      },
    }, addMetadata: false);
  }

  Future<Map<String, dynamic>> request(
    String method,
    Map<String, dynamic> params, {
    bool addMetadata = true,
  }) async {
    final process = _process;
    if (process == null) throw StateError('WAR is not running');

    final id = ++_nextId;
    final completer = Completer<Map<String, dynamic>>();
    _pending[id] = completer;
    final actual = Map<String, dynamic>.from(params);
    if (addMetadata) {
      actual['_meta'] = {
        'io.modelcontextprotocol/protocolVersion': protocolVersion,
      };
    }

    process.stdin.writeln(jsonEncode({
      'jsonrpc': '2.0',
      'id': id,
      'method': method,
      'params': actual,
    }));
    await process.stdin.flush();

    return completer.future.timeout(
      const Duration(seconds: 65),
      onTimeout: () {
        _pending.remove(id);
        throw TimeoutException('MCP request timed out: $method');
      },
    );
  }

  Future<Map<String, dynamic>> callTool(
    String name,
    Map<String, dynamic> arguments,
  ) => request('tools/call', {'name': name, 'arguments': arguments});

  void _onLine(String line) {
    if (line.trim().isEmpty) return;
    final message = jsonDecode(line) as Map<String, dynamic>;
    final id = message['id'];
    if (id is! int) return;
    final completer = _pending.remove(id);
    if (completer == null) return;
    final error = message['error'];
    if (error != null) {
      completer.completeError(StateError(jsonEncode(error)));
    } else {
      completer.complete(message['result'] as Map<String, dynamic>);
    }
  }

  Future<void> close() async {
    final process = _process;
    _process = null;
    if (process == null) return;
    await process.stdin.close();
    try {
      await process.exitCode.timeout(const Duration(seconds: 2));
    } on TimeoutException {
      process.kill();
    }
    await _stdout?.cancel();
    await _stderr?.cancel();
    _pending.clear();
  }
}
```

Tool call:

```dart
final war = WarMcpClient();
await war.start(absoluteWarPath);

final result = await war.callTool('war.query', {
  'role': 'button',
  'name_contains': '发送',
  'enabled': true,
  'limit': 5,
  'fields': ['capabilities'],
});

await war.close();
```

### Flutter Windows packaging

Keep the executable as a normal installed file rather than a Flutter asset. For a project that stores it at `runtime/war.exe`, add this to `windows/CMakeLists.txt`:

```cmake
install(
  FILES "${CMAKE_CURRENT_SOURCE_DIR}/../runtime/war.exe"
  DESTINATION "${CMAKE_INSTALL_PREFIX}/runtime"
  COMPONENT Runtime
)
```

Resolve it beside the installed Flutter executable:

```dart
import 'dart:io';
import 'package:path/path.dart' as p;

final appDir = File(Platform.resolvedExecutable).parent.path;
final warPath = p.join(appDir, 'runtime', 'war.exe');
```

## Rust in-process integration

A Rust host can construct the runtime directly:

```rust
use std::sync::Arc;
use war_runtime::WarRuntime;
use war_uia::UiaProvider;

let runtime = WarRuntime::new(Arc::new(UiaProvider::new()?));
war_mcp::serve_stdio(
    &runtime,
    std::io::stdin().lock(),
    std::io::stdout().lock(),
)?;
```

For a GUI program whose standard streams are already in use, call `WarRuntime` directly or connect `serve_stdio` to private pipes. A sidecar remains the simpler production boundary for non-Rust hosts.

## Operational checklist

- Use one long-lived process instead of spawning one process per tool call.
- Drain stderr independently; never parse it as MCP.
- Enforce request timeouts in the host and correlate every response ID.
- Treat process exit as session invalidation.
- Copy `session_id` and `epoch` into every ref-based `war.act` request.
- Close stdin for graceful shutdown; kill only the exact recorded child as fallback.
- Do not expose WAR over an unauthenticated TCP listener.
