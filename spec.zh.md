# Spec v1 — 技术栈

> 阶段：spec1（技术选型已定，分工初定）。概念定义见 [concepts.md](concepts.zh.md)，本文件只定技术分工，不重复概念。

## 技术选型

| 层 | 技术 | 覆盖概念 | 职责 |
|---|---|---|---|
| UI | TypeScript（无框架，vanilla）+ Tauri 2 frontend | View、pet（表达）、Component、Chat Panel、Autonomy（表现） | 浮动椭圆窗口、颜文字渲染、卡片弹出与方位选择、聊天面板、右键吸附 |
| 系统 | Rust（Tauri backend，单进程） | Ambery、Timer、Harness（Queue / Context / Event Buffer / Compression）、Filter、Tool Set 执行 | 实例生命周期、LLM 调用循环、消息顺序处理、HTTP 端口监听、调度 UIA sidecar |
| UIA 读取 | C#（.NET，独立 sidecar 进程） | Terminal Window / Tab / Content、Status 判定 | 枚举窗口、切 Tab、读 TermControl 全文、状态机判定——直接复用 exp01 已验证代码 |
| Hook | PowerShell 脚本 | Hook | Claude Code `"type": "command"` hook → 读 stdin JSON → POST 到 AmberyBackend 本地端口（与 ~/.claude/hooks/ 现有生态一致，零解释器依赖） |
| 持久化 | JSONL 文件 + 单文件 Config | Storage、Config | append-only：`queue.jsonl` / `context.jsonl` / `work-agents.jsonl`；`AGENTS.md` pet 身份提示词；`config.json` 单独，启动加载 |

## 架构决定

1. **单进程 + 单一协议**：Tauri 应用即 Ambery（内嵌 ambery-core 绑 127.0.0.1）。前端始终走 **HTTP + WebSocket loopback** 与 core 通信——浏览器调试模式连独立运行的 core debug binary，Tauri 模式连内嵌 server，前端代码不变。Tauri commands/events 不采用（理由见 docs/harness.md 末节）。
2. **UIA 读取**：保留 C#（exp01 已验证，不重写）。编译为独立 console exe，作为 Tauri sidecar 随包分发；Rust 通过 stdio（JSON Lines 请求/响应）调用，如 `read_tab`、`list_windows`、`switch_tab`。**打包定案**：self-contained win-x64（非单文件），用户零 .NET runtime 依赖；发布命令 `dotnet publish -c Release`（RID/self-contained 已固化在 sidecar.csproj），Tauri `externalBin` 引用 publish 布局（docs/sidecar.md §打包）。
3. **Hook 链路**：用 `"type": "command"` + **PowerShell 脚本**转发（与 ~/.claude/hooks/ 现有生态一致，零解释器依赖），不用 `"type": "http"`。AmberyBackend 内嵌 HTTP listener 收 POST。
4. **LLM 调用**：Rust 侧，OpenAI 兼容 Chat Completions endpoint，base_url / key 从 Config 读。
5. **Storage**：一律 append-only JSONL，重启 replay 恢复 Queue / Context / 实例清单。后续有查询需求再换 SQLite。
6. **Config**：单文件 JSON，运行时加载；`edit_config` tool 写回。Config 与 Storage 分离（concepts §12/§13）：`%USERPROFILE%\.config\ambery\config.json` + 同根 `storage/`，路径由 core/paths.rs 解析（`AMBERY_CONFIG_DIR` / `AMBERY_STORAGE_DIR` 可覆盖）。

## 已定约束

- HTTP listener 仅绑定 127.0.0.1
- UIA sidecar 通信协议：stdio JSON Lines（docs/sidecar.md）
- 无 Python 解释器依赖（用 PowerShell）
- 前端框架：vanilla TS（显示逻辑简单，不引框架；浏览器模式可直接跑 vite dev 用 Chrome DevTools 测试）
- **hook payload 契约（docs/hook.md）**：command 脚本转发、session_id 身份、marker 定位
