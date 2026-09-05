# 接入协议（Ambery Protocol）

[English](access-protocol.md) | 中文

> 概念定义见 concepts.md §5（Ambery Protocol）及其子概念。本文档定义 pet 对外的角色与事件契约——外部软件如何成为 Source、事件如何进出、动作如何分级。实现文档由 docs-spec 责任地图登记，本文档不逐一列举。

## 定位

Ambery 与外部软件之间的中间交流层只由本契约定义。hook 脚本、sidecar stdio 协议、终端适配器接口都是契约之下的实现实例——各自文档描述机制，不各自定义接入语义。

## 双角色

- **MCP client（拉取）**：pet 消费外部来源插件提供的数据——来源插件是向 Ambery 提供来源或动作的 MCP server。
- **MCP server（推送）**：pet 向 agent 与宿主软件暴露推入入口——外部世界主动告诉 pet「出事了」。

与普通 MCP 的差别：pet 有注意力核心——观察不是被动管道：Watch Schedule 主动调度观察节奏，Digest 强制消化更新流，值得说的事经 Queue 进入 Agent Loop 主动通知用户。

## 事件形态

```text
Source   = { kind, source_id, title, focused } + extras
Content  = text | position | progress
```

- `kind`：来源种类的机器标识（如 `wt` / `zellij` / `book`）。
- `source_id`：来源的稳定身份——同一来源的更新在同一 ID 下再进入（Context Slot 语义：连续性由 ID 承载，注意力由更新流驱动）。
- `title` / `focused`：规范化核心字段，宿主侧的保证。
- `extras`：动态扩展，键带 `<kind>_` 前缀，缺键优雅降级；某 extras 字段被多家来源证明有用后，晋升为规范化字段。
- `Content` 多形态：正文文本 / 位置（书页、播放点）/ 进度（百分比、集数）。

## 工具面

| 工具 | 方向 | 语义 |
|---|---|---|
| `list_sources` | 拉取 | 枚举宿主下的来源；失败 = 无观察，与「枚举成功但为空」严格区分 |
| `read_source` | 拉取 | 单源三态读：`Content`（存活证据）/ `Gone`（确证不存在）/ `Error`（无观察，信念不动） |
| `notify` | 推送 | 宿主或其 Hook 推入「出事了」——生命周期事件、书签变化、tab 事件 |
| `report_progress` | 推送 | 结构化位置 / 进度上报（Content 的 position / progress 载体） |
| `act` | 拉取 | 对来源执行动作，按 act 三类分级授权 |

## act 三类

动作按副作用范围分三类，权限递增，能力协商按类授予：

1. **observe**——无副作用的观察：读屏幕、读页面、查进度。
2. **mutate-source**——改源自身状态：暂停播放、翻页、回滚。
3. **mutate-user-env**——改用户环境：切 tab、切虚拟桌面、调整窗口；与 Platform Primitives 衔接（Ambery 自身界面的平台能力走同一类语义）。

## 进程与调节

来源插件进程分离：语言自由、崩溃隔离——一个插件崩溃不传染宿主。订阅与轮询的取舍、扫描错峰、背压、通知合并，全部集中在 Ambery 观测环的单一调节核心（Watch Schedule 定计划，Timer 执行）。本契约只定义面，不定义节奏策略。

## 交付语义

- **幂等**：同一事件重复交付不产生新事实。
- **去重**：同一事件多次到达合并为一次。
- **顺序**：同一 `source_id` 的更新按发生序交付。

Queue 与 Event Buffer 是 Harness 域器官，不属于本协议——协议只承诺交付语义，内部用哪个器官缓冲是实现细节。

## 传输

- **stdio**：本地插件的标准载体。
- **Streamable HTTP**：远程来源的载体（Ambery 作为 MCP server 的推送面同理）。

生命周期协商、能力变更通知、退避重连一律沿用 MCP 的既定件，不自造传输机制。
