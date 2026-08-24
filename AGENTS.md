# AGENTS.md

Ambery 是 Tauri 多窗口桌宠（pet/chat/menu/shelf/card）+ Rust core。改动前先读对应 docs/ 域文档。

## 提交信息

- 精简：subject 一句概括改动即可，不写长 body。
- 提交信息用英文（开源规范：subject 英文，国际协作与工具链友好）。
- 这是一个开源的项目，需要避免在项目和提交信息中加入任何敏感信息。

## 运行与构建

- 壳必须经 tauri CLI：`npx tauri dev`（开发热更，连 vite 5174）或 `npx tauri build`（生产，嵌 dist 出安装包）。
- 裸 `cargo build` 保持 dev 模式但不带 vite，跑起来"127.0.0.1 拒绝连接"，只做编译检查。
- CI（push/PR，三平台门禁）：Rust 测试（默认 + case-runner）、前端 tsc/token 守卫/双语配对、frontend vitest、shell cargo check。本地改完至少跑 `cargo test --workspace`。
- Release：`v*` tag 或 workflow_dispatch 触发 → 三平台 `npx tauri build` 出安装包传 Release。

## Debug

手段按覆盖领域：

- `cargo test --workspace` → core 逻辑（Rust 测试）。
- `cargo run -p ambery-case -- <case.case>` → 行为复现（两段式 .case 回放；`--health` 校验 / `export` 从 storage 造 case）。
- `cargo run -p ambery-case -- frontend --silent` → 前端行为（headless，嵌 core + vitest）。
- `cargo run -p ambery-case -- serve --brain-addr <url>` → 端到端链路（配合 LLM 替身 `python3 scripts/debug_brain.py` 与前端 `npm run dev`，浏览器观察）。
- tauri CLI（`npx tauri dev` / `build`）→ 壳层（窗口、构建形态）。
- `tools/locate.ps1` → 定位 Ambery 所有窗口的位置/尺寸/可见性（表格式输出）。

## 存储 JSONL 输出文件

默认目录 `~/.config/ambery/storage/`（`AMBERY_STORAGE_DIR` 可覆盖），app 运行产物，调试/回放用，每个文件一行：

- `context.jsonl` — Context 统一信封（session / head / message / autonomy 行；head 行复现最近上下文，其余留痕）。
- `queue.jsonl` — 入队输入留痕（role / source；排队轨迹，非对话本体）。
- `effect.jsonl` — 动作流记录（后端副作用 + 前端非只读调用，如 render_component / window_resized / window_moved）。
- `terminal-content.jsonl` — 终端原文存档（Filter 前），`filtered_content` 的现算源。
- `work-agents.jsonl` — 实例生命周期 upsert 日志（register / 状态变更 / closed，每行一条全量快照）。
- `cron.jsonl` — cron 计划条目（append-only，启动 replay 折叠为当前计划集）。
- `context.jsonl.bad` — 解析失败坏行的隔离区（`read_all` 跳过并把原行移入此处）。

## User Goals

- UI 交互禁止浏览器原生弹窗（alert / prompt / confirm）：错误与输入用应用内 UI 元素表达（内联提示 / 内联表单），不用系统对话框。
