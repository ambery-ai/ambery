# Tools 设计

[English](tools.md) | 中文

开发工具统一定义：`tools/` 目录下的脚本工具 + core 的独立 bin 工具。

## 工具清单

| 工具 | 说明 | 调用 |
|---|---|---|
| `locate.ps1` | 枚举进程下所有窗口（定位/验证窗口相关改动） | 直接运行 |
| `locate.swift` | `locate.ps1` 的 macOS 对应版——枚举进程下所有窗口 | 直接运行（`swift tools/locate.swift`） |
| `run-vite-dev.ps1` | vite dev 常驻 runner（崩溃自动重启） | 后台常驻 |
| `ambery-activity` | 读取 storage JSONL 的活动查看器（core 独立 bin） | 见下 |

## ambery-activity — storage 活动查看器

读取 Storage 目录下 JSONL 文件（docs/storage.md），TUI 交互查看内部消息流。用于开发/调试时观察系统实际写下的内容。

### 数据源

按 storage 布局（docs/storage.md）读取：

- `queue.jsonl` — Queue 输入排队记录（每行一个 turn 边界）
- `context.jsonl` — Context 消息（message / autonomy / head / session / 压缩边界）
- `effect.jsonl` — 动作流（render / close / window 生命周期 / event_emit）
- `terminal-content.jsonl` — Terminal Content 原文
- `work-agents.jsonl` — Code CLI 实例生命周期记录
- `cron.jsonl` — 定时任务

### 形态

TUI 交互界面（`ratatui`）。核心交互：

- **文件切换**：在不同 JSONL 文件间切换
- **滚动**：上下翻看历史记录
- **筛选**：按 kind / role / 来源过滤
- **跟随**（`--follow`）：tail 新写入的记录

### 键位

两种形态共用同一套键位，左右与上下对称：

| 键 | 动作 |
|---|---|
| `↑` / `k`、`↓` / `j` | 光标上 / 下移动（详情栏聚焦时滚动内容） |
| `←` / `h` | 折叠光标所在的可折叠对象（trajectory 形态）；从详情栏返回列表 |
| `→` / `l` | 展开折叠的容器（trajectory 形态）；其余任意行打开详情栏 |
| `gg` | 跳到顶部 |
| `G` | 跳到底部 |
| `Tab` / `Shift+Tab` | 切换文件源 前进 / 后退 |
| `/` | 开始筛选（kind / summary 子串） |
| `f` | 切换跟随（tail 新写入记录） |
| `q` / `Esc` | 退出 |

### 符号前缀

每一行带一个符号前缀（单符号 + 一个空格），来源文件一眼可辨：

| 符号 | 来源 |
|---|---|
| `▸` | turn 边界 / `[pre turn]` 区域（可折叠容器） |
| `·` | `context.jsonl`（message / autonomy / head / usage / session / compact_boundary） |
| `▪` | `effect.jsonl` |
| `–` | `terminal-content.jsonl` |
| `◇` | `work-agents.jsonl`（Code CLI 实例——被监管的外部 agent，不是本系统 LLM） |
| `◷` | `cron.jsonl` |

选中行用 `❯` 标记（不用 `▶`——会与 `▸` 容器前缀撞形）。来源只由符号表达——行内不再带冗余的 `[file]` 标签；详情栏仍显示完整元信息。

### 详情栏

行可打开与关闭详情栏。`→` / `l` 在任意行（除无内容的 `[pre turn]` 标签行；折叠的容器先展开）打开右侧栏（40% 宽），显示该行的未截断内容：该行的列表文案作为头部，随后是全文。栏打开期间 `↑` / `k`、`↓` / `j` 滚动内容；`i` 切换全屏（全文占满整个区域，`i` 或 `Esc` 退出回分栏）；`←` / `h`（或 `Esc`）关闭栏，列表恢复全宽。行模型保留未截断的全文（`detail`），列表始终渲染截断摘要。

### Trajectory 形态（`--trajectory`）

平铺 JSONL 投影为 **turn-centric 轨迹账本**：顶层单位是本系统 LLM 的一次完整处理回合——**turn** = Queue 放行一轮（concepts §4c-1；`queue.jsonl` 每行一个 turn）。该轮产生的全部内容——context 写入、effect、终端读取、agent 记录、cron 动作——按 ts 归属到最近的 turn，缩进一级渲染其下。

- `queue.jsonl` 每行 = 一个 turn 边界。无 queue 数据时（case 快照常见），`context.jsonl` 的 user message 退化为 turn 边界。
- **被监管的 agent 实例不是本系统 LLM**：被监管的外部会话只以普通 `◇` 行出现，从不构成层级。
- `context.jsonl` 的 `session` 行是普通 `·` 行（context 存储的启动分界，每次后端启动一条）——归属到所在 turn，不是容器。
- 首个 turn 之前的行渲染在 `[pre turn]` 区域下——与 turn 同 glyph、同折叠语义。
- **折叠**按容器，仅两层（turn / `[pre turn]` > 其行）：`←` / `h` 折叠光标所在容器——容器行本身，或其下任意行（向上折叠到所属容器）；`→` / `l` 展开折叠的容器；已展开的容器行或叶子行则打开详情栏。折叠的容器保留边界行并带 `[+n]` 标记（隐藏数量）。`/` 筛选、Tab / Shift+Tab 切文件、`f` 跟随与普通形态一致。

### 实现

Rust 独立 bin，复用 core 的 JSONL 记录类型（`ContextMessage` / `Effect` 等）。目录参数默认取 `storage_dir`（`AMBERY_STORAGE_DIR` 可覆盖），也支持显式传目录。
