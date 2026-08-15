# Effect Reporting（Tauri 运行时动作上报）

> 概念：所有**非只读 Tauri 运行时动作**统一进 effect 流（docs/case-runner.md
> §Tauri 运行时动作可观测：`(Tauri runtime actions − readonly) ⊆ effects`）。Tauri 运行时动作同时覆盖 WebView 的 `@tauri-apps/api` 与 Rust 壳的 `tauri` API；动作流格式与记录点见 `docs/storage.md` §effect.jsonl。本文定义运行时动作层、通道、kind/payload、打包规则与埋点清单。

## 原则

> **非只读全进，只读不进**——有副作用的 Tauri 运行时动作统一上报进动作流；纯读取调用不进。

> **一动作一记录**——每个非只读 Tauri 运行时动作各有一条 effect；一次调用发生多个动作时分别记录，不能以笼统的调用名合并。

> **高频打包**——高频同类动作按目标去抖合并为一条记录；具体清单与窗口参数在正文契约。

> **模拟环境不进**——原则只覆盖真实运行时调用；调试环境（浏览器）的 DOM 模拟操作不上报。

> **动作与留痕同出口**——所有非只读 Tauri 运行时动作只能经语义化运行时动作层执行；该层在动作成功后写对应 effect，调用失败不写“已发生”的 effect。

## 运行时动作层

非只读 Tauri runtime action 的唯一出口是同名的两侧动作层：

```text
app/src/tauri_runtime_actions.ts       ← WebView `@tauri-apps/api` 的写动作
app/src-tauri/src/tauri_runtime_actions.rs
                                        ← Rust 壳 `tauri` API 的写动作
```

它们共享**动作词表与 kind / payload 契约**，不共享跨语言代码。业务入口只编排语义化动作，不直接调用 Tauri 写 API，也不自行拼写 effect 的 kind / payload：

```text
业务入口
  → hide_window("pet")              → window_hidden {window:"pet"}
  → show_window("chat")             → window_visible {window:"chat"}
  → close_window("card-x")          → window_closed {window:"card-x"}
  → emit_event("cards:hide")        → event_emit {event:"cards:hide"}
  → create_card_window(...)          → window_opened {window:"card-x"}
```

动作层执行真实 Tauri API 后才记录：

```text
运行时动作
  ├─ 成功 → 恰好一条对应 effect（高频打包例外见下）
  └─ 失败 → 返回/处理原错误；不写“动作已发生”的 effect
```

同一业务调用触发多个运行时动作时，逐个转发、逐个记录。例如隐藏 pet、隐藏 chat、发送 `cards:hide` 与 `pet:hidden` 是四个动作，不能合成一条 `toggle` effect。只读调用不属于动作层：`getByLabel`、`outerPosition`、`currentMonitor`、`listen` 等可在调用处直接使用。

`window-adapter.ts` 的 Tauri 写操作即 WebView 动作层的窗口部分（收编于同名动作层，不在其外叠加第二层）。

#### ⟡ 一致性剖析

WebView 的 `@tauri-apps/api` 与 Rust 壳的 `tauri` API 是同一类 Tauri runtime action 的两个实现面，不应因代码位置不同而分裂 kind、payload 或 `origin`。两侧经同名语义化动作层共享动作词表：成功后记录对应 effect，失败不制造“已发生”的证据；复合入口逐动作转发，不能以 `toggle` 等笼统 effect 代替多个窗口与事件动作。动作层统一的是非只读动作与留痕，纯读取仍可直接调用，避免为无副作用查询制造多余包装。

## 通道

Tauri 运行时动作分两类，通道不同：

| 类 | 例子 | 通道 |
|---|------|------|
| 调用 core 的写动作 | WebView invoke `append_user` / `push_event` / `set_config` / `update_card_layout` / `set_card_user_closed` / `ensure_card_window` / `close_card_window` / `export_theme` / `import_theme` | **不单独上报**——core 接收端（HTTP handler 与 Tauri command 双运输层各自）在接收时记录，origin=frontend |
| 不经 core 的运行时动作 | WebView WebviewWindow 创建/关闭、setSize/setPosition、show/hide、startDragging、emit/emitTo；Rust 壳 WebviewWindow / AppHandle 的等价动作 | 仅运行时动作层可记录：WebView 经 `record_effect` Tauri command（生产）/ `POST /effect`（debug HTTP，亦供测试）；Rust 壳直接写同一记录入口 |

- 两种通道最终共用单点记录：`harness.log_effect(Frontend, kind, payload, now_ms)` 写 effect.jsonl；`origin=frontend` 指 Tauri UI 侧，不区分 WebView 与 Rust 壳的实现位置。
- WebView 上报 fire-and-forget：不 await、错误吞掉（上报失败不破坏窗口逻辑）；Rust 壳记录同样不得阻断其主动作。

## kind / payload

| kind | 含义 | payload | 频率 |
|------|------|---------|------|
| user_message | append_user（端点记录） | {text} | 低 |
| interaction | push_event（端点记录） | {desc, card_id?} | 低 |
| config_update | set_config（端点记录；LLM edit_config 路径仍是 config_changed/backend） | {path} | 低 |
| card_layout | update_card_layout（端点记录；Card 布局回写，docs/components.md §Card 文件） | {id, manual} | 低 |
| card_visibility | set_card_user_closed（端点记录；Cards Shelf 显隐切换写显示选择） | {id, user_closed} | 低 |
| expression_changed | Autonomy 表情/动作实际变化（覆盖/回落/推导语义显式；未变不记，window_resized 不侧击推断） | {face, motion, source: set_autonomy\|revert\|derive} | 低 |
| window_opened | Rust 壳 `ensure_card_window` create 分支（窗口决策上提，docs/case-runner.md） | {window} | 低 |
| window_closed | Rust 壳 `close_card_window`（agent close / shelf dismiss / 用户 × 三路径收口） | {window} | 低 |
| window_resized | WebView / Rust 壳 `setSize` / setOffset | {window, w?, h?, top?, left?, count?} | 打包 |
| window_moved | WebView / Rust 壳 `setPosition` | {window, x, y, count?} | 打包 |
| window_drag | WebView `startDragging` | {window} | 低 |
| window_visible | WebView / Rust 壳 `show` | {window} | 低 |
| window_focused | WebView `setFocus` / Rust 壳聚焦等价动作（showWindow 内的第二动作；menu 前台抢焦） | {window} | 低 |
| window_hidden | WebView / Rust 壳 `hide` | {window} | 低 |
| theme_export | `export_theme`（端点记录；主题导出文件副作用，docs/theme.md） | {name} | 低 |
| event_emit | WebView / Rust 壳 `emit` / `emitTo` | {event, target?, count?} | 打包 |

打包：key = kind + 区分键（window 名 / event 名），250ms 静默期后 flush 一条，payload 保留
最后一次值并附 `count`（合并条数）。

## 运行时动作层覆盖表

同一运行时动作只在其真正执行处记录一次；一次调用触发多个动作则逐个记录。例如 Rust 壳的 toggle 隐藏 pet / chat 并 emit 两个事件，必须写两条 `window_hidden` 与两条 `event_emit`，不能写一条 `toggle`。下表是动作层覆盖现状。

| 动作层 | 语义化动作 | 当前调用点 / 覆盖 |
|------|------------|------------------|
| WebView | resize_window / move_window / show_window / hide_window | `window-adapter.ts` 的 setSize/setOffset、setPosition、show/hide；pet/card/chat 三窗共用，window 名取 getCurrentWindow().label |
| Rust 壳 | ensure_card_window / close_card_window | pet/card/shelf 三路径（agent render/close、shelf 显隐/dismiss、用户 ×）经 WebView 动作层 invoke 触发，Rust 命令端点记录：create→window_opened、reuse→event_emit(card:spec)、close→window_closed |
| WebView | start_dragging | `windows/pet.ts`、`windows/card-window.ts`、`windows/chat-window.ts`；对应 window_drag |
| WebView | emit_event | `windows/pet.ts`、`positioning/tauri-server.ts` 的 emit / emitTo；对应 event_emit |
| WebView | hide_window | `windows/menu.ts` 的 menu hide；对应 window_hidden |
| Rust 壳 | show_window / hide_window / close_window / emit_event | `app/src-tauri` 的 toggle、托盘关闭及其他 WebviewWindow / AppHandle 等价动作；逐个对应 window_visible / window_hidden / window_closed / event_emit |
| 浏览器模拟 | — | browser adapter / drag.ts / component-manager.ts **不进入动作层、不埋点**（DOM 模拟，非 Tauri 运行时动作） |

## 明确不做

- **不为 effect.jsonl 加 replay**：动作流是观测日志，启动不复原（与其他留痕文件一致，
  effect 行不进内存）。
- **不在 case 数据节加 effect 节**：case 从空 effect.jsonl 起跑剧情，运行期记录由 steps 产生
  （后端）——前端上报属真实运行环境，headless 下不可达（case-runner.md §边界隔离）。
