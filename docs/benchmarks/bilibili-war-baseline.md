# WAR Bilibili baseline

- Date: 2026-08-28 (Asia/Shanghai)
- Binary: `target/release/war.exe`
- Browser: a foreground, isolated-profile Google Chrome window launched by `scripts/benchmark-bilibili.ps1`
- Scenario: WAR locates Chrome's address bar, enters `https://www.bilibili.com/`, discovers a visible ordinary video link, invokes it, and verifies both the resulting video URL and matching Chrome window title.
- Status: verified, with an immediate stability repeat and a final rebuilt-release acceptance run.

The timed interval starts before WAR's first address-bar inspection and stops only after final WAR verification. Browser process/profile setup and teardown are excluded. Navigation, targeting, invocation, and verification all use WAR; the in-app browser and Chrome DevTools Protocol are not used.

## Primary result

| Metric | Result |
| --- | --- |
| Verified outcome | `bilibili.com/video/BV1rF8B6QECt/` and matching video window title |
| Selected video | `日本社畜吃点啥？私密马赛老板✋🏻😭🤚🏻瓦塔西只是午休找餐馆花了丁点儿时间` |
| Wall-clock time | 1,978 ms |
| WAR round trips | 12 |
| Request bytes | 1,872 |
| Response bytes | 23,045 |
| Total wire bytes | 24,917 |
| Token estimate | 6,230 (`ceil(total UTF-8 JSONL bytes / 4)`) |

The immediate repeat also verified successfully in 2,344 ms over 14 round trips, 31,032 total bytes, and an estimated 7,758 tokens. After the final release rebuild, an acceptance run selected a different current homepage video and verified in 3,179 ms over 18 round trips, 44,602 bytes, and an estimated 11,151 tokens. The variation comes from current homepage content and additional bounded readiness polls while Chrome loads.

`Token estimate` is a transport-size proxy, not the model's exact tokenizer count; the exact model token usage is not exposed to this script. An earlier exploratory structured-snapshot run used 70,524 bytes (estimated 17,631 tokens) and is excluded from the official baseline because the compact text path superseded it.

## Server-filtered query/wait result

After adding `query`, local `wait`, and action `format: "summary"`, the same script no longer transfers complete page snapshots or intermediate readiness observations.

| Metric | First optimized run | Stability repeat |
| --- | ---: | ---: |
| Verified outcome | Yes | Yes |
| Wall-clock time | 2,419 ms | 2,155 ms |
| WAR wire round trips | 7 | 7 |
| Request bytes | 1,554 | 1,556 |
| Response bytes | 3,414 | 3,460 |
| Total wire bytes | 4,968 | 5,016 |
| Token estimate | 1,242 | 1,254 |
| Homepage wait | 645 ms / 4 observations | 669 ms / 4 observations |
| Video URL wait | 66 ms / 1 observation | 144 ms / 2 observations |
| Window-title wait | 1,404 ms / 7 observations | 1,044 ms / 5 observations |

Against the original 1,978 ms primary result, the first optimized run reduced wire bytes and the Token proxy by 80.1% and wire round trips by 41.7%, while wall time increased by 22.3%. The repeat was only 8.9% slower than that unusually fast original run. Across the three earlier valid runs versus the two optimized runs, the small-sample mean improved from about 2,500 ms to 2,287 ms; changing homepage content and network readiness prevent treating that as a controlled browser-performance claim.

One intermediate cold-page run took 11,883 ms but still used exactly seven wire round trips. This confirms the intended separation: page/network readiness may vary, while intermediate UIA polling no longer increases Agent round trips or returns repeated trees.

## Reproduce

```powershell
cargo build --release
.\scripts\benchmark-bilibili.ps1
```

The script starts its own visible Chrome window and closes only that run's processes afterward. It returns non-zero unless both URL and title evidence are observed through WAR.
