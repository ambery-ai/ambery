# Component 设计

[English](components.md) | 中文

> - **本文档范围**——概念定义见 [concepts.zh.md](../concepts.zh.md) §Component。本文档定调用协议、生命周期事件、方位几何、渲染与交互事件格式。

## 调用协议（call_component）

pet 经 Tool Set 发起，AmberyBackend 转发给 UI 渲染。**`call_component` 管理整条生命周期（创建/更新/关闭）**——同一 id 首次创建、后续原地更新、显式关闭。

Tool schema 以 `anyOf` 声明每种类型的完整字段（各家 LLM provider 均支持嵌套 `anyOf`，无需 `oneOf`/`const`）。spec 顶层只放对所有类型都成立的字段，类型特有的全部进 `content`：

```ts
// 方位名（与 docs/toolset.md 一致；引擎内部按 16 方位环解析，见 positioning/types.ts）
type Direction =
  | 'n' | 'ne' | 'e' | 'se'
  | 's' | 'sw' | 'w' | 'nw'
  | 'auto';

type ComponentSpec = {
  id: string;
  type: string;                 // 已注册的 Component 类型（§类型注册表）
  direction?: Direction;
  action?: 'close';
  content: object;              // 形状由该类型的注册项拥有
};

// 已注册的类型及各自拥有的 content 形状：
type ComponentContent = {
  text_card:  { title: string; text: string };       // text 是 markdown 子集（§渲染）
  quick_jump: { label: string; target: string };
  data_chart: { title: string; chart: { kind: 'line' | 'bar' | 'pie'; labels: string[];
                                         series: { name: string; data: number[] }[] } };
  todobox:    { title: string; items: { text: string; done: boolean }[] };
};
```

- `id` 由 pet 生成（如 `cmp-001`），同一 id 重复调用 = **原地更新**（而非 toggle 关闭）。创建和更新共享同一 spec 结构。
- 类型特有字段不出现在顶层：`content` 是它唯一的载体，所以新增类型是加一条注册项，而不是加一个顶层参数。
- `text_card.content.text` 承载的是 markdown 子集（§渲染），不是纯文本。
- 关闭 card：`action: "close"`（此时只需 `id`，忽略 `content`）。两级兼容：LLM 把 `action` 放在 args 顶层（与 `spec` 并列）时后端同样识别为关闭。
- `direction` 省略或 `auto` 时按「屏幕剩余空间最大的方位」自动选择。
- 文本量级约束（见 §渲染）是 system prompt 对 pet 的约束，渲染层不做硬校验。

## 类型注册表

一个 Component 类型是注册表里的一条注册项，而不是工具 schema 或 core 里的一个分支。注册项拥有：

| 部分 | 职责 |
|---|---|
| `contentSchema` | 该类型 `content` 的 JSON Schema 片段；工具 schema 的 `anyOf` 由这些片段生成 |
| `validate(content)` | 结构校验与它的错误文案 |
| `render(content)` | 该类型的 Card body |
| `sizeModel(content)` | 该类型如何贡献 Card 窗口尺寸：块配方、固定尺寸、或 cap + 滚动（[docs/card-window-size.zh.md](card-window-size.zh.md)） |
| `label` / 图标 | Cards Shelf 的一行 |

新增类型 = 新增一条注册项。调用协议、spec 顶层形状与 Card 文件形状都不变。`sizeModel` 必填——缺了它窗口尺寸就无法推导，尺寸契约不允许。

### ⟡ 一致性剖析

Component、Card、Surface 与 OS Window 不是同一层概念：Component 是 Agent 可调用的结构化内容；Card 是该内容作为持续工作产物的持久对象；Surface 是用户可见、可隐藏、可恢复的逻辑界面；OS Window 是宿主的物理容器，Surface 与瞬时弹出层都投影进它。把四者分开，Card 才能跨重启存在而 window 句柄不必持久化，effect 也只审计动作而不反推当前 Card。Chat 与 Card 统一为 Managed Surface（显示 / 隐藏 / 恢复语义统一，进 engine 占区），pet 自身也是 Managed Surface 兼锚点——其余 Surface 的偏移以它为原点度量，它的可见性统辖整组。Cards Shelf 也不是 Surface——它是 pet 锚定的瞬时管理弹出层（同 Menu 一类：失焦即关、不进 engine 占区、无持久状态），其真相在被管理的 `.card.json` 集合。

Card 文件以完整 JSON 持久化：`component` 是 Agent 正常读取与更新的 Component spec；`_meta` 是本地 Surface 管理状态（schema 版本、创建时刻、显示选择与布局），不进入 Agent 投影，也不会被普通同 id 更新覆盖。这样用户隐藏 Card 后，Agent 仍可更新其内容，却不能借一次普通更新偷偷覆盖用户的显示选择。

## Card 文件（.card.json）

一张 Card = `memory/cards/<id>.card.json` 一个完整 JSON 文件——内容（`component`）与 Surface 状态（`_meta`）同位。文件即 Card 的跨重启真相：启动恢复从文件读，不经 effect.jsonl replay（动作审计不反推 Card）。

```json
{
  "component": { "id": "todo-1", "type": "todobox", "content": { "title": "清单", "items": [] } },
  "_meta": {
    "schema": 2,
    "created": 1785860000000,
    "user_closed": false,
    "layout": { "direction": "sse", "offset": [30, 40], "manual": true, "size": [480, 240], "layoutVersion": "9a3f2b1c" }
  }
}
```

- `component`：Card 的当前内容。agent 经 `call_component` 写入（同 id 更新替换整个 component）；改变内容的用户交互把更新后的内容回写进来。因此该文件是渲染、尺寸计算与重启恢复的**单一内容来源**。
- `_meta`：本地 Surface 管理状态——不进入 Agent 投影（tool result 只回 ok/id），普通同 id 更新不覆盖：
  - `schema`：文件格式版本（当前 2）；版本 1 的文件把类型特有字段放在 `component` 顶层，加载时迁移到版本 2——类型特有字段移入 `content`。
  - `created`：创建时刻（epoch ms）；更新不刷新。
  - `user_closed`：显示选择——用户隐藏 = true（Cards Shelf 显隐切换经 `set_card_user_closed` 写入）；Agent 更新不覆盖。
  - `layout.direction`：spec.direction（`auto`/省略 = null；每次更新跟随 spec）。
  - `layout.offset`：相对 pet 中心的偏移（用户拖拽结束经 `update_card_layout` IPC 回写；自动布局 = null）。
  - `layout.manual`：用户亲手拖过 = true（place 保持偏移不重算）。
  - `layout.size`：投影的 Card 窗口尺寸 `[width, height]`，单位 CSS px，由尺寸计算产出；没有任何用户动作编辑它（[docs/card-window-size.zh.md](card-window-size.zh.md)）。
  - `layout.layoutVersion`：决定 `layout.size` 的输入标识（布局常量 + 字体标识）；与当前输入不一致即投影过期。
- dismiss（agent close / 用户 ×）：结束 Surface——删文件、出注册表、忘记布局。
- id 即文件相对路径，也是窗口名后缀：字符限 `A-Z a-z 0-9 _ - /`——因为 id 会拼成 `card-<id>` 当 Tauri 窗口标签，而 Tauri 拒收 `.`；可含 `/`（嵌套子目录），禁空段与 `..` 段（tool 校验拒绝）。
- 恢复：启动时扫描 `memory/cards/` 全部 `.card.json` 重建注册表；坏文件跳过（单文件病灶不带倒整体）。窗口重建由 pet 启动时 pull（readonly `list_cards` IPC，返回 component + `_meta`）：`user_closed=false` 的卡片重建窗口；`layout.manual` 的相对 pet 偏移先 seed 进 engine（`seedManual`），card 的 `requestPlace` 命中 manual 占区即原位恢复、不占自动布局。不用 push-at-startup——effect 广播在 webview 就绪前发出会丢，pull 没有时序漏洞。

## 卡片生命周期事件

每个 card 在其生命周期中产出自然语言事件。无论来源（agent 调用 / 用户操作），事件均进入 Event Buffer，Queue 放行时附带入 Context。格式：

| 事件 | 来源 | 自然语言 |
|---|---|---|
| `created` | agent | `card created: {type}「{title}」({id}) @ {YYMMDD-HH:MM}, → 存活 N` |
| `closed_by_user` | 用户 × | `用户关闭了 {type}「{title}」({id})`<br>`card closed: {type}「{title}」({id}), {start} / {end}, → 存活 N`（两行） |
| `closed_by_agent` | agent | `card closed: {type}「{title}」({id}), {start} / {end}, → 存活 N`（一行） |
| `user_action` | 用户交互 | `用户{勾选了/取消勾选了/新增了} {type} 条目「{text}」`（逐条过程） |

交互事件的前端协议：前端只上报结构化事实（action/cardId/字段），自然语言文本由 core 按 Harness 语言经 lifecycle 单源现写（[docs/i18n.zh.md](i18n.zh.md) §Harness 内部语言）；本表为 zh 语义样例。

- agent 更新 card 不产事件（tool result 已有完整 spec）。
- `start` / `end` 格式：`YYMMDD-HH:MM`。
- `存活 N` = 事件后剩余的 card 数量。

## 方位几何

- 锚点 = View 中心。卡片按 direction 向量偏移：`锚点 ± (View 半径 + 12px 间距 + 卡片半尺寸)`。
- 斜方位（如 top-right）= 两轴分别偏移。
- 出屏处理（[docs/window-follow.zh.md](window-follow.zh.md) §出屏与重叠）：不压人 > 完全可见；完全失踪（与所在屏零相交）才沿 16 方位环换向重试。
- 已弹出卡片经 engine 以 pet 相对偏移跟随 View 移动（[docs/window-follow.zh.md](window-follow.zh.md)；用户拖拽可改偏移基准）。

## 渲染

- 卡片结构：header（title + 关闭按钮 ×）+ body（按类型渲染）。
- `data_chart` 用内联 SVG（无依赖）：line=polyline，bar=rect，pie=path；hover 详情用 SVG `<title>` 原生 tooltip。
- `text_card` 把 `text` 当 markdown 渲染：段落、标题、有序与无序列表、引用块、围栏代码块、表格、分隔线，以及行内强调/加粗/行内代码/链接。图片与内联 HTML 不渲染——按原文显示——因为图片高度只有加载后才可知，而内联 HTML 是最大的注入面。
- 围栏代码块横向滚动而不折行；表格在 Card 宽度上限内折行单元格内容，超出上限时在 Card 内横向滚动。
- 四种类型的文本量级约束（system prompt 对 pet 的约束，渲染层不硬校验）：

| 类型 | 描述 | 文本量级 | 交互 |
|---|---|---|---|
| `text_card` | 标题 + markdown 子集，基础信息展示 | 200-500 字 | 复制/关闭 |
| `quick_jump` | 快捷跳转 | 50-100 字 | 点击跳转 |
| `data_chart` | 折线/柱/饼图 | 标题+图例 50 字 | hover 详情 |
| `todobox` | 待办列表，可勾选 | 30-100 字/条 | 勾选/新增 |

## 交互事件 → Event Buffer

concepts §Component / §Event Buffer：用户交互**不写 Context 的 user role，也不经 Queue**，写入 Harness 的 Event Buffer。每条记录携带两部分载荷：

- **自然语言**（必填）：操作过程描述
- **结构化状态快照**（可选）：仅 todobox 类交互时附带，同 card 在单次 flush 内去重合并为一份最终状态

改变内容的交互——勾选或新增 todobox 条目——同时把更新后的内容回写进 Card 文件（§Card 文件）。Event Buffer 记录仍是该动作的审计线索；文件仍是单一内容来源。

前端经 `bridge.pushEvent(desc, state?)` 上报：

| 交互 | 自然语言 | 结构化快照 |
|---|---|---|
| 关闭 | `用户关闭了 {type}「{title}」({id})` + 生命周期 `card closed:` 行 | — |
| 复制（text_card） | `用户复制了 text_card「{title}」的内容` | — |
| 跳转（quick_jump） | `用户点击 quick_jump 跳转到「{target}」` | — |
| 勾选/取消（todobox） | `用户勾选了 todobox 条目「{text}」` / `用户取消勾选了 todobox 条目「{text}」` | 同 card 去重合并：`{id, type, items: [{text, done}]}` |
| 新增（todobox） | `用户新增了 todobox 条目「{text}」` | 同上 |

**设计决定**：hover 详情（data_chart）不进 Event Buffer——concepts 列举的交互（关闭、跳转、勾选）都是显式动作，hover 过于频繁会淹没 Buffer。

跳转动作（quick_jump → 切 WT 标签页）在 C# sidecar 接入前仅上报事件，不执行真实切换。
