# Benchmarks

Benchmark reports live in this directory and must state enough context to reproduce the result.

For Agent-facing desktop runs, record:

- exact WAR binary and build profile;
- starting foreground application and page state;
- success condition observed through WAR rather than inferred from dispatch;
- wall-clock time from the first WAR request through the verified final observation;
- number of WAR round trips;
- UTF-8 request and response bytes on the WAR/MCP wire;
- Token count when the caller exposes it, otherwise a clearly labeled estimate of `ceil(total UTF-8 bytes / 4)`;
- invalid setup attempts separately from the measured run.

Setup actions needed only to create the initial foreground fixture are excluded from the timed interval. Navigation, targeting, clicking, and final verification must use WAR for a WAR benchmark.
