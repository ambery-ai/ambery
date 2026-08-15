# Component 设计

> 概念定义见 concepts.md §5。本文档定调用协议、生命周期事件、方位几何、渲染与交互事件格式。

## 调用协议（call_component）

ペット经 Tool Set 发起，OverseerBackend 转发给 UI 渲染。**`call_component` 管理整条生命周期（创建/更新/关闭）**——同一 id 首次创建、后续原地更新、显式关闭。

Tool schema 以 `anyOf` 声明每种类型的完整字段（各家 LLM provider 均支持嵌套 `anyOf`，无需 `oneOf`/`const`）：

```ts
// 方位名（与 docs/toolset.md 一致；引擎内部按 16 方位环解析，见 positioning/types.ts）
type Direction =
  | 'n' | 'ne' | 'e' | 'se'
  | 's' | 'sw' | 'w' | 'nw'
  | 'auto';

type ComponentSpec =
  | { id: string; type: 'text_card';  direction?: Direction; title: string; text: string }
  | { id: string; type: 'quick_jump'; direction?: Direction; label: string; target: string }
  | { id: string; type: 'git_display'; direction?: Direction; title: string;
      entries: { hash: string; msg: string; time: string }[]; diff?: string }
  | { id: string; type: 'data_chart'; direction?: Direction; title: string;
      chart: { kind: 'line' | 'bar' | 'pie'; labels: string[];
               series: { name: string; data: number[] }[] } }
  | { id: string; type: 'todobox';    direction?: Direction; title: string;
      items: { text: string; done: boolean }[] };
```

- `id` 由ペット生成（如 `cmp-001`），同一 id 重复调用 = **原地更新**（而非 toggle 关闭）。创建和更新共享同一 spec 结构。
- 关闭 card：`spec.action = "close"`（此时只需 `id`，忽略其余字段）。两级兼容：LLM 把 `action` 放在 args 顶层（与 `spec` 并列）时后端同样识别为关闭。
- `direction` 省略或 `auto` 时按「屏幕剩余空间最大的方位」自动选择。
- 文本量级约束（见 §渲染）是 system prompt 对ペット的约束，渲染层不做硬校验。

#### ⟡ 一致性剖析

Component、Card、Surface 与 OS Window 不是同一层概念：Component 是 Agent 可调用的结构化内容；Card 是该内容作为持续工作产物的持久对象；Surface 是用户可见、可隐藏、可恢复的逻辑界面；OS Window 只是 Surface 在当前进程中的物理投影。把四者分开，Card 才能跨重启存在而 window 句柄不必持久化，effect 也只审计动作而不反推当前 Card。除 pet 外的 Chat 与 Card 统一为 Managed Surface（显示 / 隐藏 / 恢复语义统一，进 engine 占区）；pet 自身是锚点与交互入口，不属于 Surface。Cards Shelf 也不是 Surface——它是 pet 锚定的瞬时管理弹出层（同 Menu 一类：失焦即关、不进 engine 占区、无持久状态），其真相在被管理的 `.card.json` 集合。

Card 文件以完整 JSON 持久化：`component` 是 Agent 正常读取与更新的 Component spec；`_meta` 是本地 Surface 管理状态（schema 版本、创建时刻、显示选择与布局），不进入 Agent 投影，也不会被普通同 id 更新覆盖。这样用户隐藏 Card 后，Agent 仍可更新其内容，却不能借一次普通更新偷偷覆盖用户的显示选择。

## Card 文件（.card.json）

一张 Card = `memory/cards/<id>.card.json` 一个完整 JSON 文件——内容（`component`）与 Surface 状态（`_meta`）同位。文件即 Card 的跨重启真相：启动恢复从文件读，不经 effect.jsonl replay（动作审计不反推 Card）。

```json
{
  "component": { "id": "todo-1", "type": "todobox", "title": "清单", "items": [] },
  "_meta": {
    "schema": 1,
    "created": 1785860000000,
    "user_closed": false,
    "layout": { "direction": "sse", "offset": [30, 40], "manual": true }
  }
}
```

- `component`：Agent 的 ComponentSpec 原文；`call_component` 同 id 更新只换 component。
- `_meta`：本地 Surface 管理状态——不进入 Agent 投影（tool result 只回 ok/id），普通同 id 更新不覆盖：
  - `schema`：文件格式版本（当前 1）。
  - `created`：创建时刻（epoch ms）；更新不刷新。
  - `user_closed`：显示选择——用户隐藏 = true（Cards Shelf 显隐切换经 `set_card_user_closed` 写入）；Agent 更新不覆盖。
  - `layout.direction`：spec.direction（`auto`/省略 = null；每次更新跟随 spec）。
  - `layout.offset`：相对 pet 中心的偏移（用户拖拽结束经 `update_card_layout` IPC 回写；自动布局 = null）。
  - `layout.manual`：用户亲手拖过 = true（place 保持偏移不重算）。
- dismiss（agent close / 用户 ×）：结束 Surface——删文件、出注册表、忘记布局。
- id 即文件相对路径：可含 `/`（嵌套子目录），禁空段与 `..` 段（tool 校验拒绝）。
- 恢复：启动时扫描 `memory/cards/` 全部 `.card.json` 重建注册表；坏文件跳过（单文件病灶不带倒整体）。窗口重建由 pet 启动时 pull（readonly `list_cards` IPC，返回 component + `_meta`）：`user_closed=false` 的卡片重建窗口；`layout.manual` 的相对 pet 偏移先 seed 进 engine（`seedManual`），card 的 `requestPlace` 命中 manual 占区即原位恢复、不占自动布局。不用 push-at-startup——effect 广播在 webview 就绪前发出会丢，pull 没有时序漏洞。

## 卡片生命周期事件

每个 card 在其生命周期中产出自然语言事件。无论来源（agent 调用 / 用户操作），事件均进入 Event Buffer，Queue 放行时附带入 Context。格式：

| 事件 | 来源 | 自然语言 |
|---|---|---|
| `created` | agent | `card created: {type}「{title}」({id}) @ {YYMMDD-HH:MM}, → 存活 N` |
| `closed_by_user` | 用户 × | `用户关闭了 {type}「{title}」({id})`<br>`card closed: {type}「{title}」({id}), {start} / {end}, → 存活 N`（两行） |
| `closed_by_agent` | agent | `card closed: {type}「{title}」({id}), {start} / {end}, → 存活 N`（一行） |
| `user_action` | 用户交互 | `用户{勾选了/取消勾选了/新增了} {type} 条目「{text}」`（逐条过程） |

交互事件的前端协议：前端只上报结构化事实（action/cardId/字段），自然语言文本由 core 按 Harness 语言经 lifecycle 单源现写（docs/i18n.md §Harness 内部语言）；本表为 zh 语义样例。

- agent 更新 card 不产事件（tool result 已有完整 spec）。
- `start` / `end` 格式：`YYMMDD-HH:MM`。
- `存活 N` = 事件后剩余的 card 数量。

## 方位几何

- 锚点 = View 中心。卡片按 direction 向量偏移：`锚点 ± (View 半径 + 12px 间距 + 卡片半尺寸)`。
- 斜方位（如 top-right）= 两轴分别偏移。
- 出屏处理（docs/window-follow.md §出屏与重叠）：不压人 > 完全可见；完全失踪（与所在屏零相交）才沿 16 方位环换向重试。
- 已弹出卡片经 engine 以 pet 相对偏移跟随 View 移动（docs/window-follow.md；用户拖拽可改偏移基准）。

## 渲染

- 卡片结构：header（title + 关闭按钮 ×）+ body（按类型渲染）。
- `data_chart` 用内联 SVG（无依赖）：line=polyline，bar=rect，pie=path；hover 详情用 SVG `<title>` 原生 tooltip。
- `git_display` 的 diff 用 `<details>` 折叠。
- 五种类型的文本量级约束（system prompt 对ペット的约束，渲染层不硬校验）：

| 类型 | 描述 | 文本量级 | 交互 |
|---|---|---|---|
| `text_card` | 标题 + 文本，基础信息展示 | 200-500 字 | 复制/关闭 |
| `quick_jump` | 快捷跳转 | 50-100 字 | 点击跳转 |
| `git_display` | git log / diff / status 可视化 | 500-2000 字 | 展开 diff |
| `data_chart` | 折线/柱/饼图 | 标题+图例 50 字 | hover 详情 |
| `todobox` | 待办列表，可勾选 | 30-100 字/条 | 勾选/新增 |

## 交互事件 → Event Buffer

concepts §5/§10e：用户交互**不写 Context 的 user role，也不经 Queue**，写入 Harness 的 Event Buffer。每条记录携带两部分载荷：

- **自然语言**（必填）：操作过程描述
- **结构化状态快照**（可选）：仅 todobox 类交互时附带，同 card 在单次 flush 内去重合并为一份最终状态

前端经 `bridge.pushEvent(desc, state?)` 上报：

| 交互 | 自然语言 | 结构化快照 |
|---|---|---|
| 关闭 | `用户关闭了 {type}「{title}」({id})` + 生命周期 `card closed:` 行 | — |
| 复制（text_card） | `用户复制了 text_card「{title}」的内容` | — |
| 跳转（quick_jump） | `用户点击 quick_jump 跳转到「{target}」` | — |
| 展开 diff（git_display） | `用户展开了 git_display「{title}」的 diff` | — |
| 勾选/取消（todobox） | `用户勾选了 todobox 条目「{text}」` / `用户取消勾选了 todobox 条目「{text}」` | 同 card 去重合并：`{id, type, items: [{text, done}]}` |
| 新增（todobox） | `用户新增了 todobox 条目「{text}」` | 同上 |

**设计决定**：hover 详情（data_chart）不进 Event Buffer——concepts 列举的交互（关闭、跳转、勾选）都是显式动作，hover 过于频繁会淹没 Buffer。

跳转动作（quick_jump → 切 WT 标签页）在 C# sidecar 接入前仅上报事件，不执行真实切换。
