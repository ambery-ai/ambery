# Window Follow（窗口跟随语义）

> 布局算法（方位最优解）见 docs/window-positioning.md。本文档定**坐标系选型、
> 职责分层与跟随/恢复的状态语义**——pet 移动、拖拽、隐藏时，窗口们该怎么动。

## 坐标系：pet 相对

engine 内部 **pet 固定 (0,0)**，所有窗口只存「相对 pet 中心的偏移 (dx, dy)」。

- 偏移是 pet 移动的**不变量**：pet 移动只改 `petCenter` 一个值，occupied 全体不动。
  没有 translateAll、没有快照、没有 stale。
- 进出 engine 的换算统一在两个口子：返回屏幕坐标 `= petCenter + offset`；
  拖拽回写 `offset = 屏幕中心 − petCenter`。
- 记忆 = 用户对布局的覆盖意图（相对偏移），不是布局本身。未拖过的窗口没有记忆，
  自动布局重算即可。

## 职责分层

| 层 | 管什么 | 关键物 |
|---|---|---|
| engine | **纯位置**：布局计算、偏移存储 | `occupied[]`（offset/尺寸/manual）、`place()`、`updateCenter()`、`restorePositions()` |
| 各窗口 | **可见性 + 用户意图** | 私有 `userClosed` 布尔 |
| pet.ts | **编排** | 系统藏广播、恢复广播（现算新中心） |

## 状态语义

| 状态位 | 置位时机 | 效应 |
|---|---|---|
| `manual` | 用户拖拽松手（outerPosition 回写偏移） | `place()` 不重排，保持偏移；pet 移动以该偏移为跟随基准 |
| `userClosed` | 用户 × / toggle 关闭 | 恢复广播**不**恢复它（A 语义）；toggle 开时复位 |

**隐藏三分**：用户主动关（意图，置 userClosed）、pet 拖动临时藏（系统藏，不动标志位）、
托盘连坐藏（系统藏，同上）。后两者共享恢复路径：pet 回来 → 广播新中心 →
各窗口自查 userClosed 决定是否显示。

#### ⟡ 一致性剖析

除 pet 外的 Chat 与 Card 统一是 Managed Surface（显示 / 隐藏 / 恢复语义统一）。Managed Surface 的持久真相只分四组：身份、内容、Surface 意图（生命周期与显示选择）、空间布局（direction 与 auto / manual offset）；OS 窗口句柄、creating / closing 与系统临时隐藏均属运行期，不进入 Surface。定位占区不是 `hide()` 的隐式副作用，而由持久意图现算：用户隐藏释放占区但保留布局记忆，系统临时隐藏保留占区，dismiss 结束 Surface 并忘记其布局。窗口种类只能以内容来源和生命周期 policy 区分，不能各自发明 hide / close / placement 语义。Cards Shelf 不属于 Managed Surface：它是 pet 锚定的瞬时管理弹出层（与 Menu 同类——失焦即关、pet 拖拽连坐关、不进 engine 占区、无持久空间布局与显示选择），其持久真相不在窗口，而在它管理的 `.card.json` 集合。

## 恢复：现算，无快照

`restorePositions(petCenter)` = `occupied.map(o => petCenter + o.offset)`，调用时现算。
**无快照机制（hideAll/restoreAll）**——偏移常驻为真后，任何时刻都能直接派生。

## 显示器几何（monitor 缓存表）

全部显示器的逻辑矩形**缓存成表，一次读取**（拓扑低频变化，不每次查询）：

```ts
monitors: [{ x, y, width, height, scaleFactor }]  // 逻辑像素（size/scaleFactor）
monitorOf(petCenter) → 所在屏矩形                  // 出屏判定 / 高度 cap 的唯一直径
```

- **来源**:Tauri 走 `availableMonitors()`（全部屏含 scaleFactor)；浏览器 = 视口（`window.innerWidth/Height`，卡片 DOM 的世界就是视口——用 window.screen 会把视口外位置误判为可见）
- **缓存时机**：启动一次
- **自愈刷新**:pet 中心不在任何缓存矩形内（换屏/改拓扑/热插拔）→ 自动重读，永远收敛
- **消费通道统一**：`adapter.getScreenHeight()` 为唯一取口，其 Tauri 实现内部
  读本表——分支不各自 `window.screen` / `currentMonitor()`（见「Adapter 遵守」原则节）

## 坐标单位契约

**engine 世界 = 环境帧**（Tauri = 物理像素，browser = CSS 逻辑像素）——engine 内部不关心
单位，只要求同一环境内全链路一致。换算只允许发生在两个口子：

| 口子 | 方向 | 规则 |
|---|---|---|
| 测量入口（DOM → engine） | CSS → 物理 | DOM 测量（getBoundingClientRect / offsetWidth）**必须 ×dpr 才进 engine**（Tauri 环境）；browser(dpr=1）天然一致 |
| adapter 出口（engine → OS） | 物理 → OS | engine 输出直接给 `setPosition/setSize`（PhysicalPosition/Size），不经二次换算 |

已有遵守：card-window 测量 `offsetWidth × dpr` ✓、`outerPosition`（物理）回写 ✓、
monitors 表存物理原始矩形 ✓。**禁止**:DOM/CSS 值（未 ×dpr）写入 engine 的
occupied/offset（多 DPI 屏下偏移错位的根源）。

逻辑像素需求（样式上限如 cap=屏高×0.5）不属于 engine 世界——走
`adapter.getScreenHeight()`（逻辑像素）单独通道，不混入 engine 坐标。

## 出屏与重叠

**不压人 > 完全可见**：宁允许卡片部分出屏，绝不做「拉回屏内」的事后 clamp——
事后 clamp 只看视口不看障碍，会把引擎算好的位置拉到别的窗口上（重叠元凶：
chat.ts 与 component-manager.ts 的定位不做事后 clamp）。

**完全失踪**（卡片矩形与所在屏零相交）不允许，兜底 = **全 16 方位环重试**
（算法层零改动，computeCDSegments / ternarySearch 不感知视口）：

```
place(dir) 完全出屏 → 沿 16 方位环 ±1、±2 … ±8 换向重试，首个「非完全出屏」即止
（±1~±3 只覆盖同半球——pet 贴底边时可用方向在北半球，永远试不到）；
全失败 → 接受最初原方向的结果（出屏但诚实，不压人，不做位置修正）。
```

**部分可见即可**：重试接受标准为「非完全出屏」，不要求完全在屏内；
完全失踪才触发换向。

**视口单位**：engine 世界 = 物理像素（petCenter、窗口尺寸、setPosition 同为物理）;
browser 的「屏」= 浏览器视口（卡片 DOM 活在视口里，不是 OS 屏，用 window.screen
会把视口外的位置误判为可见）。多屏语义暂取 pet 所在屏。

## 拖拽回写

```
mousedown（× 除外） → startDragging（OS 模态，边缘被钳制）
onMoved 防抖 → outerPosition（OS 钳制后的真实位置）
→ offset = (outerPos + size/2) − petCenter → updateCenter(id, offset) + manual=true
```

## 原则：语义单源，分支只做翻译

**同一语义只允许一处实现；分支路径（Tauri / browser / 新壳）只做事件到
统一 API 的翻译。** 宁可将分支合并共用，不可各自实现——同语义两处实现必然腐化
分叉。

推论：
- 新增分支壳时，第一件事是找语义的统一 API，而不是在新壳里重写
- 发现「这边改了那边没改」时，正确动作是合并到单源，而不是补齐副本

## 原则：Adapter 遵守——分支不写逻辑，共用下沉

**跨窗口/跨分支共用的能力，一律下沉到单点（Adapter 层 / engine / 共享模块），
分支窗口只消费接口，不各自实现。** 分支里只该有「本窗口特有」的薄事件接线；
几何（屏幕矩形、坐标换算）、状态（monitor 缓存、可见性语义）这类共用物，
写在分支里一份就是埋一颗分叉的地雷。

落到本项目的实例：
- 屏幕几何（显示器缓存表、逻辑像素换算）= 单点（Adapter/共享模块），pet/card/chat
  都从这里取，不各自 `window.screen` / `currentMonitor()`
- 窗口操作（尺寸/位置/显隐）= WindowAdapter 双模式单点，分支不直接碰 Tauri API
- 可见性语义（intentClose/systemHide/systemRestore）= ChatPanel 单点（见上节）
- 新能力立项时先问「放哪个单点」，而不是「在哪个分支写」

## 两路径统一

恢复/隐藏语义**单源**在 ChatPanel 的统一 API，分支只做事件翻译：

| API | 语义 | Tauri（chat-window） | browser（DOM ChatPanel） |
|---|---|---|---|
| `intentClose()` | 用户意图关：userClosed=true + 隐藏 | × / toggle 关 | × 按钮 |
| `intentOpen()` | 用户意图开：userClosed=false + 显示 | toggle 开 | （右键唤出） |
| `systemHide()` | 系统藏：只隐藏，不动 userClosed | `chat:hide` 事件 | view:drag-start |
| `systemRestore(center)` | 系统恢复：userClosed=false 才重定位+显示 | `chat:show` 事件 | view:moved |

禁止在分支里各自实现恢复规则。
