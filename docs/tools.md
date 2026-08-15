# Tools 设计

开发工具统一定义：`tools/` 目录下的脚本工具 + core 的独立 bin 工具。

## 工具清单

| 工具 | 说明 | 调用 |
|---|---|---|
| `locate.ps1` | 枚举进程下所有窗口（定位/验证窗口相关改动） | 直接运行 |
| `run-vite-dev.ps1` | vite dev 常驻 runner（崩溃自动重启） | 后台常驻 |
| `ambery-activity` | 读取 storage JSONL 的活动查看器（core 独立 bin） | 见下 |

## ambery-activity — storage 活动查看器

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

### Trajectory 形态（`--trajectory`）

参考 dsh 的 trajectory 概念：平铺 JSONL 投影为 **turn-aware 紧凑轨迹账本**——保留因果结构而不是只给一行行日志。

- `context.jsonl` 的 `session` 行 = 会话边界（较重规则）；`queue.jsonl` 每行 = 一个 turn 边界（Queue 放行一轮 = 一次触发）；其余行按 ts 归属到最近 turn，缩进为事件行。
- 无 queue 数据时（case 快照常见），`context.jsonl` 的 user message 退化为 turn 边界。
- `x` 折叠/展开全部事件行——只看 session/turn 骨架；`/` 筛选、Tab 切文件、`f` 跟随与普通形态一致。

### 实现

Rust 独立 bin，复用 core 的 JSONL 记录类型（`ContextMessage` / `Effect` 等）。目录参数默认取 `storage_dir`（`AMBERY_STORAGE_DIR` 可覆盖），也支持显式传目录。
