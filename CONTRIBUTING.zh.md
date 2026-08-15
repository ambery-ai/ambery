# Contributing to Ambery

Ambery 是 Agent 桌宠：Hook 驱动读取 Claude Code 会话，pet 观察、报告并管理卡片组件。
本文件给出贡献入口；设计与概念术语见 `concepts.md`，架构决定见 `spec.md`，文档规范见 `docs-spec.md`。

## 先读这些

- `concepts.md`：概念模型与术语表——提交信息、注释和测试名一律用它。
- `spec.md`：技术选型与架构决定。
- `docs/`：按域拆分的详细设计；改动行为前先找对应域文档。
- `dev/`：开发过程记录（未决问题、回归记录）。

## 仓库布局

```
core/             Rust 核心库（Harness / AmberyBackend / server / storage）
ambery-case/      case-runner：storage 快照回放、概念观测、前端 headless case 宿主
app/              前端 vanilla TS（pet / chat / components / positioning）
app/src-tauri/    Tauri 壳（静态窗口 + 多窗口卡片 + /hook 薄 server）
sidecar/          Windows UIA sidecar（C#，仅 Windows 构建/打包）
scripts/          开发脚本（hook 安装、debug brain）
tools/            窗口定位等诊断工具
```

## 环境

- Rust stable（workspace 不含 Tauri 壳，`cargo` 在仓库根运行）
- Node 24 + npm（前端；`app/package-lock.json` 是唯一锁文件）
- Tauri 壳单独构建：`cd app/src-tauri && cargo check`（mac 已启用 `macOSPrivateApi` 支撑透明窗口）
- Windows UIA sidecar：.NET 9 SDK；发布形态 self-contained win-x64，见 `docs/sidecar.md §打包`

## 测试命令

```bash
# Rust workspace（默认 feature = release 形态）
cargo test --workspace

# case-runner feature（观测/回放/前端 headless 注入面）
cargo test -p ambery-core --features case-runner

# 前端 headless case（内嵌 core + vitest；全链路 mock/keyless）
cargo run -p ambery-case -- frontend --silent

# 类型与设计 token 守卫
cd app
npm ci
npx tsc --noEmit
node scripts/lint-tokens.mjs
```

CI 定义见 `.github/workflows/ci.yml`：ubuntu + macos 双平台跑上述矩阵，无 secret、无真实 LLM 调用。

## 提交规范

- **一事一提交**：行为修复、测试适配、文档更新分开提交；不要混装。
- 提交信息用中文（仓库现行约定），首行概括行为，正文写原因与影响面。
- 提交信息与注释不出现任何内部项目/示例名；代码和文档都要能直接公开。
- 每个行为改动带测试：core 单测放在对应模块，前端行为进 `app/test/*.test.ts`。
- 文档与行为一起改：行为变了，对应 `docs/` 设计文档同步更新，否则文档会落空。

## 代码规则

- 前端读取走 store（`app/src/store.ts`），写入走动作层（`app/src/tauri_runtime_actions.ts`）；主逻辑不散调 `invoke`。
- 非只读 Tauri 运行时动作必须进 effect 流（`docs/effect-reporting.md`）。
- 新增 `Effect` 变体必须同步 `effect_kind_payload` 与前端 bridge 分发（穷尽 match 会编译期提醒）。
- storage 是 append-only JSONL：日志神圣、视图易失；不得原地改写历史行。
- 路径解析只经 `core/src/paths.rs`；平台差异用 `cfg(windows)` 门控，非 Windows 不得依赖 UIA。

## Windows 专属边界

- UIA sidecar 只编译、只打包在 Windows；mac/Linux 是 Hook 驱动的核心体验，不是降级版。
- 涉及 sidecar、Tauri 壳窗口行为、install-hooks 的改动在 mac/CI 上只能覆盖编译与协议层；
  真机验证项标注在 `dev/issues.md`，不要声称未验证的 Windows 行为已通过。
