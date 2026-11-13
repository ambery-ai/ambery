# Spec

[English](spec.md) | 中文

> 文档职责/导航：见 [docs-spec.zh.md](docs-spec.zh.md)。概念定义见 [concepts.zh.md](concepts.zh.md)；本文件记录仓库级结构（目录布局）与技术选型。运行机制住在 `docs/`；终端/agent 包在 `packages/` 下各自的目录里带 spec。

## 目录布局

```
ambery/
├── core/                          本体 crate（ambery-core + bins）
├── observe-derive/                Observe 宏 proc-macro crate（ambery-observe-derive）
├── packages/                      包拆分
│   ├── case-runner/               测试回放引擎（ambery-case）
│   ├── terminal-lib/              契约 crate
│   ├── terminals/                 每终端一包
│   │   ├── wt/                    C# UIA sidecar
│   │   ├── zellij/                进程内 CLI adapter
│   │   └── ghostty/…              未来终端包
│   ├── agents/                    每 agent CLI 一包
│   │   ├── claude/                hook 脚本 + filter + marker
│   │   └── opencode/
│   └── apps/                      前端包
│       ├── src/                   共享前端
│       ├── tauri/                 Tauri 壳 + 宿主层
│       └── webui/                 纯 web 形态——同一前端代码的第二宿主
├── docs/                          契约文档（docs-spec.zh.md）
├── experiments/                   一次性原型
├── reports/                       调研结论
├── scripts/                       开发脚本
├── tools/                         诊断工具
└── Cargo.toml                     workspace 清单
```

`packages/` 下每个有 crate 的包在自己的目录里带 spec（`packages/case-runner/spec.md`、`packages/terminal-lib/spec.md`、`packages/apps/spec.md`、`packages/terminals/wt|zellij/spec.md`、`packages/agents/claude|opencode/spec.md`）；本文件管没有下层 spec 认领的部分：仓库结构与技术选型。

## 技术选型

| 选型 | 理由 |
|---|---|
| Rust，单进程（`ambery-core`） | 单一地址空间让 LLM 调用、工具执行与持久化共用一份崩溃可恢复的日志 |
| append-only JSONL 文件 + 单文件 JSON 配置 | 崩溃安全的运行记录；完整 OpenAI 上下文可从日志复原 |

## 技术决策

- **JSONL 而非 SQLite**——日志神圣、视图易逝：append-only 文件让每次运行可复原、可 grep；SQLite 等查询需求真实出现再引入。
- **不引入本地分词器**——token 预算跟随 API 的 usage 真值，与 Claude Code / opencode 的做法一致；本地 BPE 会与 provider 计数漂移。
- **LLM 调用留在 Rust**——OpenAI 兼容 Chat Completions 端点；base_url / key 来自 Config；reasoning_content 按 provider 契约持久化（docs/agent-loop.md）。
- **observe-derive 是构建附属**——`Observe` 宏独立成 crate 仅因 proc-macro 必须如此；它与 core 同版本同发布。

## 架构决定

- **一个消费者，一个协议**：宿主进程是唯一消费者；外部软件只经 Ambery Protocol 的契约面接触。无旁路通道。
- **终端接入只经契约消费**：core 依赖 `ambery-terminal-lib`，永不依赖终端包；激活哪些终端包的组装发生在二进制/配置层。
- **依赖单向**：`core` → `terminal-lib` 仅此；`terminals/*` 与 `agents/*` → `terminal-lib` 仅此，彼此之间与对 core 都不依赖；`apps/*` → `core`；`case` → 全部（只读服务）。二进制可以为接线额外依赖终端/agent crate。协议（concepts §Ambery Protocol）是跨包共享的契约。
- **外部进程只观测、不臆断**：无证据不做生命周期推断。
- **AmberyBackend 不内嵌终端知识**：core 代码路径中不出现 wt/zellij/claude 名字；终端包经契约到达。
- **存储永远 append-only**：重启靠 replay 恢复状态；压缩是标记不是删除；`context.jsonl.bad` 隔离解析失败行而非丢弃。
- **Config 与 Storage 是两个独立域**：单文件 `config.json` + 身份提示词 `AGENTS.md` 在 config 根；运行数据在 `storage/`；路径由 `core/paths.rs` 解析。
