# Provider contract

A provider implements `war_core::DesktopProvider`:

```rust,ignore
trait DesktopProvider: Send + Sync {
    fn kind(&self) -> NodeSource;
    fn snapshot(&self, scope: SnapshotScope) -> WarResult<RawSnapshot>;
    fn execute(&self, node: Option<ProviderNodeRef>, action: &Action)
        -> WarResult<ProviderActionResult>;
    fn subscribe(&self) -> WarResult<Subscription>;
}
```

Providers must translate native controls into protocol `Role` and `Capabilities`; native UIA/Java/Qt types must not leak upward. A raw snapshot must form a rooted tree with valid parent/child IDs. Provider IDs need only remain valid until its next snapshot because the semantic layer owns stable refs.

Provider event callbacks should do minimal work and signal an affected scope. Refresh, normalization, fingerprinting, diffing, and rendering belong outside the callback. Provider actions should attempt native semantic patterns before keyboard or coordinate fallback and report the method used.

Providers must enforce bounded collection before returning data. The UIA implementation caps raw depth/nodes and individual strings, removes password values, uses exact HWND roots, and aggregates process scope from that process's visible top-level HWNDs rather than traversing Desktop Root. A provider must never use a global-input fallback after the target loses foreground ownership.

Future Java, vision, Office, or application-specific providers can be added without changing the protocol or semantic compiler.
