# Windows Agent Runtime

`windows-agent-runtime`（简称 **WAR**）是一个纯 Rust 的 Windows 桌面自动化运行时。它将 Windows UI Automation 转换成紧凑、稳定、可验证的语义接口，供本地 Agent 通过 CLI、JSONL 或 MCP Stdio 操作现有桌面软件。

WAR 的目标不是模拟“盲点鼠标”，而是让 Agent 以较低 Token 成本定位控件、执行操作，并验证操作结果。

> Provider 负责 Windows 上发生了什么以及怎样操作；Semantic 层负责 Agent 应该看到什么；Agent 负责决定做什么。

## 主要能力

| 能力 | 说明 |
| --- | --- |
| 语义观察 | UIA 子树归一化、稳定 `@ref`、角色、状态、能力和差异输出 |
| 精确查询 | `inspect` 定位单个目标，`query` 在服务端筛选少量候选 |
| 本地等待 | `wait` 在 WAR 内部轮询，只向 Agent 返回最终证据 |
| 可靠操作 | Invoke、SetValue、Toggle、Select、Focus、Scroll、Click、键盘、文本和轨迹手势 |
| 结果验证 | 区分 `verified`、`dispatched_unverified` 和 `failed`，不把“已派发”冒充成功 |
| 安全边界 | 前台进程检查、遮挡命中检查、密码值脱敏、请求/树/超时硬上限 |
| Agent 接入 | 长驻 JSONL 服务和 MCP Stdio；支持现代与旧版 MCP 客户端 |
| 应用工作流 | 一次调用完成会话选择、编辑、发送和结果验证 |

当前版本不包含 OCR、视觉识别、Java Access Bridge、Office COM、浏览器 DOM/CDP、云服务或 LLM SDK。WAR 专注于本地 Windows 语义自动化。

## 快速开始

要求：Windows 10/11、Rust 1.80 或更高版本。

```powershell
cargo build --release
```

生成文件：

```text
target/release/war.exe
```

观察当前前台窗口：

```powershell
.\target\release\war.exe snapshot
```

常用 CLI：

```powershell
war snapshot --json
war snapshot --window 123456
war snapshot --process 4242
war find "Save"
war act invoke '@12'
war act set-value '@18' "hello"
war act toggle '@17' true
war watch
war exec actions.json
war serve
war mcp
```

`query` 和 `wait` 面向长驻 JSONL/MCP 调用，不是单独的 CLI 子命令。

## MCP Stdio

推荐让 Agent 客户端把 WAR 作为本地 sidecar 启动：

```json
{
  "mcpServers": {
    "war": {
      "command": "C:\\absolute\\path\\to\\war.exe",
      "args": ["mcp"]
    }
  }
}
```

WAR 通过 stdin/stdout 传输逐行 UTF-8 JSON-RPC，不监听网络端口。stdout 只包含协议消息，日志和错误使用 stderr。

| MCP 工具 | 用途 |
| --- | --- |
| `war.inspect` | 已知名称、角色或 Automation ID 时定位一个控件 |
| `war.query` | 服务端筛选并返回最多 50 个候选控件 |
| `war.wait` | 在本地等待语义条件满足，避免 Agent 反复轮询 |
| `war.snapshot` | 获取有界的结构化或紧凑文本语义树 |
| `war.act` | 执行带会话保护、前置条件和后置验证的动作批次 |
| `war.send_message` | 执行本地验证的聊天发送工作流 |

完整配置、协议兼容性和工具参数见 [MCP 文档](docs/MCP.md)。

## 低 Token 工作流

发现视频链接时，不需要返回整个浏览器页面：

```json
{
  "id": 1,
  "method": "wait",
  "params": {
    "role": "link",
    "value_contains": "bilibili.com/video/",
    "required_capabilities": "INVOKE",
    "limit": 5,
    "fields": ["value"],
    "timeout_ms": 30000
  }
}
```

响应中的 `@ref` 仅属于当前 WAR 会话。使用 ref 执行动作时，必须携带同一次观察返回的 `session_id` 和 `epoch`：

```json
{
  "id": 2,
  "method": "act",
  "params": {
    "expected_session_id": "7e28-18f...-1",
    "expected_epoch": 42,
    "actions": [{"invoke": "@12"}],
    "format": "summary",
    "stop_on_error": true
  }
}
```

`format: "summary"` 保留派发、变化和验证状态，但不返回完整 delta。

## 嵌入现有软件

最稳妥的方式是将 `war.exe` 随软件发布，并由主程序隐藏启动：

```text
MyApp/
├─ MyApp.exe
└─ runtime/
   └─ war.exe
```

主程序使用系统进程 API 启动 `war.exe mcp`，重定向三个标准流，保持一个长期进程，并在退出时关闭 stdin。这样既像内嵌功能，又保留 UIA/COM 故障隔离。

Dart/Flutter、Electron、C# 和 Rust 的生命周期与打包示例见 [软件集成指南](docs/INTEGRATION.md)。

## 性能基准

真实前台 Chrome 的 B 站导航、视频选择和双重验证全部由 WAR 完成，没有使用内置浏览器或 CDP。

| 指标 | 原始实现 | `query` / `wait` 优化后 |
| --- | ---: | ---: |
| WAR 往返 | 12 | 7 |
| 总线缆数据 | 24,917 B | 4,968–5,016 B |
| Token 估算 | 6,230 | 1,242–1,254 |
| 实测时间 | 1,978 ms | 2,155–2,419 ms |

线缆数据和 Token 代理下降约 80%。Token 数为 `ceil(JSONL UTF-8 总字节数 / 4)` 的明确估算，不是模型供应商返回的精确计数。完整过程和冷页面异常值见 [Bilibili 基准报告](docs/benchmarks/bilibili-war-baseline.md)。

复现：

```powershell
cargo build --release
.\scripts\benchmark-bilibili.ps1
```

脚本会启动并最终关闭自己的隔离 Chrome 实例。

## 测试

运行默认测试和静态检查：

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

交互式 Windows 生产门禁会启动自身的 Win32 fixture 和空白画图实例，必须单线程显式运行：

```powershell
cargo test -p war-fixture-win32 --test production_loop -- --ignored --test-threads=1
```

它覆盖窗口隐私、遮挡拒绝、事件延迟、稳定 ref、UIA/输入回退、真实画图轨迹和 warm snapshot p95。

## 安全模型

- 默认只观察当前前台窗口，也可显式限定 HWND、进程或稳定节点。
- `SendInput` 前再次验证目标进程仍处于前台。
- 点击前检查目标中心点没有被其他顶层窗口遮挡。
- 密码控件的值在 Provider 边界被移除。
- Agent 快照最多返回 256 个语义节点，并保留祖先链。
- JSONL/MCP 单条消息最多 1 MiB。
- UIA 调用具有 3 秒默认超时和有界 worker 恢复策略。
- Chrome/Edge 使用安全轮询，避免导航时 UIA 事件根生命周期冲突。

## 仓库结构

| 路径 | 职责 |
| --- | --- |
| `crates/war-protocol` | 公共动作、快照、scope、条件和错误类型 |
| `crates/war-core` | Provider 接口、目标解析、执行与验证 |
| `crates/war-semantic` | 归一化、稳定身份、裁剪、排序、差异和渲染 |
| `crates/war-win32` | 前台/窗口检查和受保护的 `SendInput` |
| `crates/war-uia` | 有界 UI Automation Provider 与 COM worker |
| `crates/war-runtime` | 有状态观察、查询、等待、动作和 JSONL 入口 |
| `crates/war-mcp` | MCP 生命周期、Stdio framing、schema 和运行时适配 |
| `crates/war-cli` | `war.exe` 命令行入口 |
| `tests/fixture-win32` | 原生 Win32 fixture 和生产循环回归测试 |
| `scripts` | 可复现的真实桌面基准脚本 |
| `docs` | 架构、协议、集成、Provider 合约和基准 |

Cargo workspace 仅包含根 `Cargo.toml` 中列出的成员。`target/`、IDE 配置和本地应用状态不会进入版本控制。

## 深入阅读

- [文档索引](docs/README.md)
- [系统架构](docs/ARCHITECTURE.md)
- [JSONL 协议](docs/PROTOCOL.md)
- [MCP Stdio](docs/MCP.md)
- [软件集成指南](docs/INTEGRATION.md)
- [Provider 合约](docs/PROVIDER.md)
- [性能基准](docs/benchmarks/README.md)

## License

源码按 `MIT OR Apache-2.0` 双许可证声明发布。
