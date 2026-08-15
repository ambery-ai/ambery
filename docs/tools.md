# Tools 设计

开发工具统一定义：`tools/` 目录下的脚本工具 + core 的独立 bin 工具。

## 工具清单

| 工具 | 说明 | 调用 |
|---|---|---|
| `locate.ps1` | 枚举进程下所有窗口（定位/验证窗口相关改动） | 直接运行 |
| `run-vite-dev.ps1` | vite dev 常驻 runner（崩溃自动重启） | 后台常驻 |
| `overseer-activity` | 读取 storage JSONL 的活动查看器（core 独立 bin） | 见下 |

## overseer-activity — storage 活动查看器

读取 Storage 目录下 JSONL 文件（docs/storage.md），TUI 交互查看内部消息流。用于开发/调试时观察系统实际写下的内容。

### 数据源

按 storage 布局（docs/storage.md）读取：

- `context.jsonl` — Context 消息（message / autonomy / head / 压缩边界）
- `queue.jsonl` — Queue 输入排队记录
- `effect.jsonl` — 动作流（render / close / window 生命周期 / event_emit）
- `terminal-content.jsonl` — Terminal Content 原文

### 形态

TUI 交互界面（`ratatui`）。核心交互：

- **文件切换**：在不同 JSONL 文件间切换
- **滚动**：上下翻看历史记录
- **筛选**：按 kind / role / 来源过滤
- **跟随**（`--follow`）：tail 新写入的记录

### 实现

Rust 独立 bin，复用 core 的 JSONL 记录类型（`ContextMessage` / `Effect` 等）。目录参数默认取 `storage_dir`（`OVERSEER_STORAGE_DIR` 可覆盖），也支持显式传目录。
