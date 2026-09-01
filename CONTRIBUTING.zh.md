# Contributing

[English](CONTRIBUTING.md) | 中文

Ambery 是 Agent 模型的桌宠：它 hook 进 agent 会话、观察模型的行动，并通过桌面上的卡片窗口自主表达。

有任何问题、想法或 bug——直接开 issue 或 discussion，不用先问。项目很早期，一切贡献都欢迎。

## 跟随文档提交（docs-first）

文档是项目的设计源头：改行为前，先读 `docs/` 里对应的域文档，行为变了文档一起改（文档与代码分开提交）。术语见 `concepts.md`；技术选型与结构决定见 `spec.md`；未决问题与回归记录在 `dev/`。

## 仓库布局

```
core/             Rust 核心库（harness / backend / server / storage）
ambery-case/      case-runner：快照回放、概念观测、前端 headless 宿主
packages/terminal-lib/      终端访问契约 crate（trait / 信封 / composite / 测试桩）
packages/terminals/wt/      Windows Terminal 包：C# UIA sidecar + Rust 客户端（仅 Windows）
packages/terminals/zellij/  zellij 包：进程内 CLI adapter
app/              前端 vanilla TypeScript（pet / chat / 卡片 / positioning）
app/src-tauri/    Tauri 壳（静态窗口 + 卡片窗口 + /hook 薄 server）
scripts/          开发脚本
tools/            诊断工具
```

## 提交规范

- **一事一提交**：行为、测试、文档分开。
- **首行英文 subject**，概括行为变更（开源协作与工具链友好）；正文写原因。
- 行为改动带测试：Rust 单测在模块内，前端测试进 `app/test/*.test.ts`。
- 提交与注释不出现内部项目/示例名——仓库是公开的。

## 代码规则

- 前端读走 store（`app/src/store.ts`），写走 action 层（`app/src/tauri_runtime_actions.ts`）；不散落 `invoke`。
- 非只读 Tauri 动作进 effect 流（`docs/effect-reporting.md`）；新 `Effect` 变体同步 `effect_kind_payload` 与 bridge 分发。
- storage 是 append-only JSONL：日志神圣、视图易逝，不就地改写历史行。
- 路径解析只走 `core/src/paths.rs`；平台差异用 `cfg(windows)` 门控，非 Windows 不依赖 UIA。

## 测试

```bash
cargo test --workspace
cargo test -p ambery-core --features case-runner
cargo run -p ambery-case -- frontend --silent   # 前端 headless case，无 key
cd app && npm ci && npx tsc --noEmit && node scripts/lint-tokens.mjs
```

CI（`.github/workflows/ci.yml`）在 ubuntu + macOS 跑上述命令；无 secrets、无真实 LLM 调用。

## 许可

MIT。贡献即同意按 MIT License 授权。
