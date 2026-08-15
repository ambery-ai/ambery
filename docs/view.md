# View 设计

> 概念定义见 concepts.md §3。本文档定物理实现与交互细节，concepts 未规定的取舍在这里记录。

## Config 字段

| 字段 | 生效 | agent 访问 | 行为 |
|---|---|---|---|
| `view_scale`（合法区间 [0.2, 4.0]） | 热 | 可见、可修改 | 立即重算 pet 尺寸、中心锚定与障碍区 |
| `badge_style` | 热 | 可见、可修改 | 立即更新未读角标样式 |
| `badge_side` | 热 | 可见、可修改 | 立即更新未读角标方位 |

## 名称

pet 名称是正式 Config 中的稳定身份值（字段 `name`）。所有需要称呼 pet 的 UI 与后续 Harness 身份文案都读取当前名称；既有 Chat 历史和已经生成的 Card 不回写。

- Config 首次初始化时写入正式默认名 **`Ambery`**（不按语言区分默认名，名称本身不参与翻译）。
- 初始化完成后，名称与 UI / Harness 语言独立：后续切换任一语言绝不自动改名。
- 名称不标记 `no_llm_visible`。本地用户与 LLM 都可经各自已有的 Config 入口显式读取、修改；LLM 修改仍受现有 query → update、校验、持久化与审计管道约束。校验：非空、≤ 64 字符。
- 不提供“按当前 Harness 语言重置默认名称”操作；语言切换不是改名操作。
- Harness 身份文案经 `{name}` 占位读取当前名称：`base_prompt` 内置默认与默认 AGENTS.md 的身份行携带 `{name}` 占位，拼装请求头时替换为当前 `name`（内置 base_prompt 原文在加载时升级为占位版本；用户改过的原样保留）。UI 侧 chat 标题与 placeholder 读取当前 `name`。

## 形态

- **Tauri 模式**：无边框、透明背景、always-on-top 的横向椭圆窗口，窗内仅颜文字，无其他 UI 元素。
- **浏览器测试模式**：`position: fixed` 的 DOM 元素模拟同一窗口，行为与 Tauri 模式保持一致，供 Chrome DevTools 测试显示逻辑。

两种模式共享同一套手势与事件，差异仅在拖拽/坐标的驱动层。

## 手势与 Chat 唤出

pet **无吸附态**（边缘吸附是 OS 式原始贴靠，与本应用自有的窗口方位布局引擎冲突）：

- **右键** = 唤出/关闭 Chat（派发 `chat:toggle`；Chat Panel 见 docs/chat-panel.md）。pet 原地不动——不瞬移、漂浮动画不受影响。
- **左键拖拽**恒可用（无锁定态）。

## 拖拽

- 浏览器模式：pointer events（pointerdown/move/up）更新 left/top。
- Tauri 模式：调用窗口拖拽 API（`startDragging`）。

## Component 锚点

Component 以 View 中心为锚点，向指定方位偏移弹出（方位由 pet 经 `call_component` 指定）。View 移动时，已弹出的 Component 以相对 pet 偏移跟随（docs/window-follow.md：engine 占区 + 布局记忆；`auto` 方位由 engine 按屏幕剩余空间现算）。方位几何细节见 docs/components.md。

## Surface 入口（pet 手势）

| 手势 | 去向 |
|---|---|
| 左键拖拽 | 移动 pet（空间位置表达） |
| 右键 | 唤出/关闭 Chat（`chat:toggle`） |
| 中键 | 进入 Cards Shelf（`shelf:toggle`） |

Cards Shelf 是 Card 集合的瞬时管理弹出层（`shelf` 静态窗口，无标题栏——上下文菜单式列表；**不属于 Surface**，见 §一致性剖析）：每张存活 Card 一行（类型图标 + 标题 + 显隐 / 删除两图标按钮），动作 = 显隐切换与 dismiss。显隐切换写 `_meta.user_closed`（`set_card_user_closed` IPC）并经 `shelf:visibility` 让 pet 开/藏对应 card 窗口；dismiss 走 closed_by_user 双行事件 + 删 `.card.json` + pet 销毁窗口。

Shelf 不当 Card 布局：不进 engine 占区、不跟随 pet、不可拖拽、无布局记忆。它是 pet 锚定的瞬时管理面板。

- **尺寸** = pet 当前物理尺寸 ×3（比例 3:1，打开时现算；钳制 180–480 × 120–240 防极端 scale 失真）
- **位置** = 左下角落在 pet 中心、向右上延伸（遮挡 pet，屏边界钳制）
- **开关**：中键 toggle——关着时弹出，开着时中键点 pet 或 shelf 任意位置都直接关闭
- **关闭**：中键 / 失焦即关（600ms 武装延迟）/ pet 拖拽或托盘连坐关；无标题栏也没有 ×
- Shelf 自身没有 userClosed 状态——瞬时语义下关闭不留任何可见性与布局痕迹

#### ⟡ 一致性剖析

pet 是用户进入 Surface 世界的锚点，而非它自身的一员：左键拖拽表达空间位置，右键进入 Chat，中键进入 Cards Shelf。Chat 是对话内容的 Surface，Card 是持久工作产物的 Surface；两者共享显示、隐藏与恢复语义（engine 占区、持久空间布局与跟随）。Cards Shelf 与 Menu 都不是 Surface——它们是 pet / shell 锚定的瞬时弹出层（失焦即关、不进 engine 占区、无布局记忆与持久可见性）：Menu 是设置入口，Cards Shelf 是 Card 管理入口，其真相分别在 Config 与 `.card.json`，不在弹出层自身。

## 事件

| 事件 | 载荷 | 说明 |
|---|---|---|
| `chat:toggle` | `{}` | 右键唤出/关闭 Chat |
| `view:moved` | `{ x, y }` | 拖拽结束后的中心坐标（供 Component 锚点计算） |
