# Architecture

The workspace keeps OS integration, semantics, orchestration, and transport separate.

```text
UIA / Win32 provider
        |
    RawSnapshot
        |
normalize -> fingerprint -> stable ref resolver -> prune -> rank
        |
 SemanticSnapshot ----> full renderer
        |
      diff -----------> delta renderer / watch
        |
Runtime resolver -> executor fallback chain -> async verifier
        |                                      |
        +------ high-level local workflows ----+
        |
 CLI / JSONL / MCP Stdio
```

`war-protocol` owns stable public types. `war-core` owns provider/resolver/executor/verifier contracts without importing Windows APIs. `war-semantic` compiles raw provider trees and retains ref state. `war-uia` owns every UIA COM object on its MTA worker. `war-win32` owns focus and `SendInput`. `war-runtime` coordinates sessions. `war-mcp` translates MCP lifecycle and tool calls into the runtime's local request surface. `war-cli` is only an adapter.

## Transport boundaries

The native JSONL service is WAR's internal transport contract. MCP is an adapter over the same runtime handler, not a second automation implementation. This keeps action semantics, verification status, limits, and errors identical across CLI, JSONL, and MCP.

The MCP Stdio adapter is dual-era. Modern `2026-07-28` requests are self-describing through per-request metadata and receive `resultType`, cache hints, and structured tool content. Legacy clients negotiate up to `2025-11-25` through `initialize`. WAR's `session_id` and `epoch` remain explicit tool data rather than hidden transport state, so stable refs stay safe under either MCP era.

## UIA threading and caching

The provider facade sends commands through a channel. The worker initializes COM as MTA, creates `IUIAutomation`, builds one subtree cache per visible top-level window, and keeps element objects private. Callers never receive COM values. Property and structure handlers are installed only after a safe scoped snapshot exists; no global desktop focus hook is registered. Hooks are suspended before every refresh so a provider cannot destroy an event root while `BuildUpdatedCache` is traversing it. Chrome and Edge deliberately use bounded 500 ms polling because their dynamic accessibility trees can corrupt `UIAutomationCore` when native subtree hooks overlap navigation. Known transient cache failures receive a short bounded retry. Callbacks only enqueue a dirty signal, and a 10-second health refresh covers otherwise quiet providers.

`Window(hwnd)` is an exact privacy boundary. `Process(pid)` creates a synthetic semantic root and combines every accessible visible top-level window for that process. `FocusedWindow` resolves from the native foreground HWND rather than walking UIA parents, preventing accidental ascent to Desktop Root.

Every UIA request, including worker initialization, has a deadline. A timed-out or disconnected worker is replaced and active subscriptions are attached to the replacement. Because Windows cannot safely kill an arbitrary stuck COM thread, a provider permits at most three timeout replacements and then opens a circuit; callers must recreate the provider/service process. This bounds in-process leakage under a persistently hostile accessibility provider. The public error retains the operation and timeout duration.

## Identity and safety

Provider node IDs are ephemeral. `SemanticCompiler` scores a new node against unmatched nodes from the previous snapshot. Automation ID, provider, role, name, ancestors, sibling hint, and weak spatial proximity contribute to the score. A score below `0.75` creates a new ref. Semantic queries below `0.75`, or ambiguous top candidates, are rejected rather than used for destructive actions. Stable-ref history is cleared when the window/process identity changes without recycling old public IDs. Every runtime also creates an opaque session nonce; ref actions must match both that nonce and the current epoch. Node scopes are translated from stable IDs to provider IDs at the runtime boundary.

The flattened snapshot remains token-efficient, but the resolver can reconstruct a root-to-node ancestor chain from node depth. `find` exposes only that compact lineage, and capability filters or nearest-capable-ancestor resolution let workflows turn leaf text into an actionable row without another observation round trip.

Agent-facing output has a 256-node budget. Ranking selects useful nodes while retaining their full ancestor chains; a truncated snapshot says so explicitly. UIA strings are bounded and password values are removed before semantic compilation.

The runtime retains bounds beside the semantic snapshot as private session geometry. Full snapshots do not pay the Token cost for every rectangle. `inspect` projects bounds only when requested, combines observe and resolve in one round trip, and defaults to a compact node result. JSONL formatting is also exclusive by default: structured or text, never both unless explicitly requested.

`query` applies bounded role, text, value, capability, and state filters after semantic compilation and returns only selected projections. `wait` reuses the same filter locally with event-assisted bounded polling, so intermediate accessibility trees never cross JSONL or MCP. Both preserve the final snapshot's session and epoch, allowing a returned ref to feed directly into a guarded action.

## Action execution

The agent selects an intent. The provider chooses mechanics. For example `Invoke` attempts `InvokePattern`, then focus plus Space, then a center-point `SendInput` click. `SetValue` attempts `ValuePattern`, then focus, Ctrl+A, and Unicode `SendInput`. A focusable, enabled UIA node with `SetValue` receives `TypeText` even when Electron exposes it as `Group` rather than a native input role. Runtime capability checks happen before execution. Any global input verifies the target process is still foreground. Element clicks also compare the target's root HWND with the window actually under the center point, refusing an obscured target even when both windows belong to the same process.

Provider acceptance is called `dispatched`; it is never treated as proof of a UI effect. With a postcondition, the runtime subscribes before dispatch and combines dirty events with bounded 50 ms polling until the condition is observed or the deadline expires. Results distinguish `verified`, `dispatched_unverified`, and `failed`, and separately report whether the semantic tree changed.

`PointerGesture` is element-relative rather than screen-relative. The semantic capability marks visible bounded canvas-like controls; the UIA adapter converts normalized points using the element's current rectangle and checks the foreground/root window before injection. The Win32 implementation uses absolute virtual-desktop `SendInput` movement and an internal release guard, so an error cannot leave a mouse button held down.

High-level workflows deliberately re-observe and re-resolve between state-changing steps because controls can appear or become enabled asynchronously. The conversation workflow keeps those observations inside the runtime, retries a semantically ineffective Invoke with a physical Click, and returns only a small verified outcome. Its local fixture is a deterministic production regression test and performs no external messaging.
