# Issues

***

**触发场景**: 启用 Tauri 多窗口后，在 pet 右键弹出 card 和 chat panel。

**表现**: Card 和 ChatPanel 在 pet 移动后留在原位，没有跟随 pet 重新定位；Card 窗口尺寸不随内容自适应，文字区域出现滚动条而非撑开窗口，且右下方内容被截断无法完整显示；全界面使用纯深色背景缺乏层次感；点击 card 上的"复制"按钮，剪贴板写入内容为 undefined。

## #1 Card 没有跟随 pet 移动 (2026-07-27) — fixed

Card 窗口弹出后位置一次性计算完成，pet 被拖到新位置时 card 停留在原地不动。

2026-07-27 二次尝试（打回）：改为 engine.hideAll/restoreAll delta 方案——pet.ts dragDebounce 的 done 回调中 `engine.restoreAll` 返回新位置后 `emit("cards:show", center)`，cards.ts 接收后 setPosition + show。打回原因：实测 pet 移动后 card 窗口**消失**而非跟随。疑点：(1) `cards:hide` 触发后 `cards:show` 可能未被 emit（`r.find(id.startsWith("card-"))` 未匹配）；(2) `cards:show` handler 收到 payload 但 `document.querySelector(".component")` 返回 null；(3) setPosition 抛出异常被 catch 吞掉导致窗口保持隐藏。需加 console.log 探针二分定位。

## #2 Card 窗口尺寸不适应内容，右下方被截断 (2026-07-27) — fixed

Card 窗口初始尺寸写死为 300×200（tauri.conf.json），但卡片实际渲染内容可能超出该尺寸，导致右下方超出部分无法在窗口中看到。卡片内容区域出现文字滚动条而非根据内容自动撑开窗口。Card 应当支持动态大小：根据内容测量结果实时调整窗口尺寸，避免内容截断和滚动条。

2026-07-27 首次尝试（打回）：CSS 改 min-width + 删 max-height/overflow 方向正确，但 `positionWindow()` 改用 `scrollWidth`/`scrollHeight` 测量——证据：Tauri 隐藏窗口（adapter.hide()）中两者返回 0，导致 `setSize(4,4)` 窗口缩成不可见。`offsetWidth`/`offsetHeight` 在隐藏窗口中有值但存在鸡生蛋蛋：卡片被窗口约束后测量值始终是约束后的值（300×200 窗口内的卡片 offset 永远 ≤ 300×200），无法通过测量撑大窗口。需要先放宽窗口尺寸再测量收缩的方案。

2026-07-27 二次尝试（打回）：窗口放大到 520×440 + DPR 修正，但测量时序仍然错误——`positionWindow()` 在窗口隐藏时测 `offsetWidth`，此时卡片被窗口 520×440 约束且 `max-width:480px`，测得永远是 480 而非内容真实需求。正确顺序：先让卡片脱离窗口宽度约束（如 `width:max-content`）自然渲染 → 测真实内容尺寸 → 提交引擎布局 → resize 窗口。当前顺序是"被约束→测约束值→提交→resize"，永远测不到内容真实大小。

## #3 界面主题单一，缺少可配置的亮色模式 (2026-07-27) — fixed

所有窗口（pet / card / chat / menu）统一使用深色背景，card 面板与背景融为一体，文字和面板之间没有足够的层次区分。两个建议方向：(1) 优化现有深色模式——为 card/chat 面板增加颜色层次（面板底色、边框、阴影），与 pet 主界面拉开视觉距离；(2) 增加可配置的 light mode（亮色模式），用户可在 config 中切换主题。

## #4 复制按钮复制 card 文本得到 undefined (2026-07-27) — fixed

Card 面板已有"复制"按钮，点击后调用 `navigator.clipboard.writeText(spec.text)`，但剪贴板实际写入内容为 undefined（字面字符串 "undefined"），说明 `spec.text` 字段为空或不存在。需检查组件渲染时传入的 spec 数据路径，确保 text 字段正确传递到复制逻辑。

***

**触发场景**: Chat panel 中发送消息。

**表现**: 发送消息后聊天区域没有任何回显（既没有用户消息也没有 AI 回复），用户无法判断消息是否被处理；有新消息到达时 pet 上没有任何视觉提示；回复不是流式输出的，而是一次性出现的。

## #5 聊天新消息无 pet 角标提示 (2026-07-27) — fixed

当 chat panel 中有新的 AI 回复到达但 chat 窗口被隐藏/关闭时，用户完全不知道有新消息。应当在 pet 上增加一个未读计数角标（数字），样式可选气泡或纯数字，方位可选（默认正右边纯数字），让用户在 chat 不可见时也能感知到有回复到来。建议：在 bridge 中暴露一个新消息计数器，pet 侧订阅后渲染为 DOM 角标（绝对定位在 view.el 右侧），点击角标时 emit chat:toggle 打开聊天面板并清零计数。

## #6 聊天发送后无回显和状态指示 (2026-07-27) — fixed

发送消息后聊天区域既没有回显用户刚发的消息，也没有任何"处理中"的状态指示（如加载动画、省略号等）。用户不知道消息是否已被提交、是否正在被 AI 处理、还是已经静默失败。用户消息应当立即出现在聊天历史中，同时显示一个处理中指示器（如输入框右侧 loading spinner），收到回复后清除。

## #7 聊天需要流式输出，thinking 只显示透明气泡 (2026-07-27) — fixed

AI 回复是一次性渲染到聊天区域的，缺少字符逐步蹦出的流式效果。LLM thinking 过程不显示具体文字（隐私），但应当展示一个透明/半透明气泡动画来表示"正在思考"，让用户感知到 AI 在活动。

2026-07-29 reopen——首次修复只落地了 loading 动画 + 透明气泡，流式输出本体 deferred 未做，回复仍整段一次性出现。打回原因：用户实测「chat 基本正常，但还没有连续显示」。流式部分现由 docs/streaming.md 定稿（Streaming Delta：LLM SSE chunk → AssistantDelta 逐片推送前端，reasoning 走 ThinkingBubble/ThinkingModal；Delta 纯显示优化不经 Queue/Context，完整回复最后写 Context）。

2026-07-29 修复——按 docs/streaming.md 四连落地：①core 流式骨架（Llm trait Send+Sync 化 + complete_streaming 默认回落；Effect::AssistantDelta/AssistantDone；effect_sink 旁路直推不进 effects Vec）；②OpenAiClient SSE（stream:true + 字节级 \n\n 事件缓冲；content/reasoning 两路；tool_calls 分片按 index 聚合）；③Bridge onAssistantDelta/onAssistantDone（TauriBridge/RemoteBridge 双通道）；④chat 流式渲染（content 逐片追加、ThinkingBubble 虚线气泡 + 点击展开思维链模态、done 收尾、重渲保留在飞气泡）。82 core 测试 + mock IPC 测试绿，用户实测确认 thinking 气泡与逐字蹦出正常。

***

**触发场景**: 用户希望移动 card 窗口或 chat panel 窗口到想要的位置。

**表现**: Card 和 chat panel 窗口无标题栏、无法被单独拖动，只能通过 pet 右键 toggle 显示/隐藏。用户不能手动将 panel 拖到更合适的位置。

## #8 Card 和 Chat panel 窗口应当支持独立拖动 (2026-07-27) — fixed

Card 和 Chat panel 窗口目前无标题栏无法被单独拖动，只能通过 pet 右键 toggle 显示/隐藏，用户不能手动将 panel 拖到更合适的位置。两个窗口都需要支持独立拖动：拖动时暂停 pet 跟随（解除与 pet 中心的偏移绑定），松手后记录新位置到布局引擎（engine.place 更新 center 坐标），后续 pet 移动时以新位置为偏移基准重新跟随。

***

**触发场景**: 2026-07-27 grill 发现——审视当前 cards 架构。

**表现**: 所有 card 共用一个 Tauri 窗口（`tauri.conf.json` 仅一个 `label: "cards"`），而非每个 card 一个独立窗口。这导致 #1（跟随）、#2（独立尺寸）、#8（独立拖动）均无法在单窗模式下正确实现。

## #9 Card 应每个独立为一个 Tauri 窗口 (2026-07-27) — fixed

当前 `tauri.conf.json` 只有一个 cards 窗口，ComponentManager 在其中做 DOM 流式布局。正确设计应当是每个 card 一个独立 Tauri 窗口（类似 chat 窗口），各自有独立的 engine entry、位置、尺寸、生命周期。`positionWindow` 当前用 `querySelector(".component")` 只取第一个 DOM 元素测量，无法区分多 card。改为多窗口后：(1) 每个 card 窗口独立监听 `pet:moved` delta 跟随；(2) 每个 card 窗口按自身内容独立测量 resize；(3) 关闭时清理各自的 engine entry。

2026-07-27 实现中：删静态 cards 窗口，pet.ts `onRenderComponent` → 动态 `WebviewWindow(label=card-${spec.id})`，新增 `card-window.ts` 单窗入口。后端校验 `fe4ec01` 已拦截非法 id。

2026-07-27 context.jsonl 实证：LLM 生成的 `call_component` spec 结构错误——将 `title`/`text` 嵌套在 `props` 对象里而非顶层。根因：工具 schema 中 `spec` 字段是裸 `{"type":"object"}`，LLM 不知道 ComponentSpec 的准确结构，自行发明了 `props` 包装。需在 tool schema 补充各 component 类型的字段定义 + 后端校验 text_card 必须有 `title` 和 `text`。

2026-07-28：capabilities 授权 + race condition 修复后 card 窗口正常弹出，基本功能验证通过。

2026-07-29：Tauri IPC 问题单独裂出 #9.5。

## #9.5 core-server → Tauri IPC 迁移 (2026-07-29) — fixed

`core-server.md` 声明"仅保留 /hook HTTP，前端走 Tauri IPC"，但实际代码仍使用全量 HTTP+WS（`RemoteBridge`），因 Tauri IPC 过程中新 Tauri commands（`get_state`/`get_config`/`append_user`/`push_event`/`get_config_schema`）的 `State` 提取失败——`invoke()` 未到达 Rust handler。旧有 `toggle_pet`/`quit_app` 的 `invoke()` 正常工作，证明 IPC 通道本身可用，问题在新命令的 `tauri::State<SharedTauriState>` 提取链路。当前回退 HTTP 桥，Tauri IPC 骨架保留在 `main.rs` 中（fe39ad9、9c764f1、f308051、27bad37）。需修复 State 提取后切回 Tauri IPC，然后删 HTTP 路由仅留 `/hook`。

2026-07-29 修复——三个根因分层处置：①State 提取：mock_app IPC 二分测试（tauri::test）证实 newtype+Arc 链路已通（invoke 到达 handler + 提取成功；race 期 wait_state 返回 not ready 不挂死），当年未二分完就回退了；②emit 推送链路实锤未接：run_core 的 handle 参数从未使用（effects 只进 WS 不进 Tauri 事件）——sender 改为 WS+handle.emit("effect") 双发；③前端恢复 TauriBridge（invoke+listen，get_context/context_changed 新名，竞态期保底），createBridge Tauri 模式启用 IPC，Tauri 模式 thin router 仅留 /hook（实测 /state/queue/context 全 404、/hook 200）。menu 设置面板「连不上 core」顺带修复：新增 set_config command，menu.ts fetch→invoke。用户实测确认：menu 正常、chat 正常、card 弹出正常。

## #10 已消亡实例未标记 closed，僵死数据污染 Event Buffer (2026-07-28) — fixed

`timer-probe`×2 + `full-body-check` 三个实例 UIA 侧车实证已不存在，但 `work-agents.jsonl` 仍为 `processing`，`last_seen` 自首次注册后从未被 timer 更新。timer 每 5s 扫描，但 `None→closed` 判定未触发，导致实例永不被标 `Closed`。连锁效应：每次 `run_trigger` 的 compression 路径调 `panorama()` 时，这些僵死实例经过滤（仅排除 Closed）被纳入全景同步 → 写入 Event Buffer → flush 到 LLM context → pet 基于虚假数据报告"filter-test 有 3 个 Processing"。缺陷点：(1) timer 扫描未触发 closed；(2) `panorama()` 过滤条件仅排除 `Closed`，未考虑 `last_seen` 远超 timer 周期的僵死实例。

***

**触发场景**: 2026-07-29 grill 发现——Queue 概念与实现严重偏差。

**表现**: concepts.md §10c 定义 Queue 为"对话队列"，角色仅为对话消息 thread。但用户的预期模型是 Queue 作为**所有信息的串行化关口**：hook 内容、user 消息、component 交互事件全部先入 Queue 排队，Queue 再分发给 Context 和 LLM。当前实现有多条并行管道：终端内容走 Context 直通（不经过 Queue）、component 事件走 Event Buffer 暂存、user 消息直接写 Queue。Queue 丧失了"唯一串行入口"的语义。

## #11 Queue 概念预期违背 (2026-07-29) — fixed

concepts.md §10c 与实际代码之间存在重大架构偏差。用户模型中 Queue 是全局串行化关口——所有输入（hook 内容、user 消息、component 交互）先入 Queue，再由 Queue 分发给 Context 和 LLM。当前实现中 Queue 仅扮演对话线程角色，终端内容经 Context 管道直达 LLM、Event Buffer 独立于 Queue 存在。这导致：(1) 多条并行管道各自推送，时序不可控；(2) Queue 不承载「串行化」语义，无法保证 LLM 调用的顺序一致性；(3) Event Buffer 的存在本身就是 Queue 未承担统一入口职责的补丁。

2026-07-29 grill 定案——Queue 链路：`输入 → Queue（排队） → Queue 放行 → Context 写输入 → LLM → 输出 → Context 写输出 → Queue 放行下一条`。Queue 是处理节奏控制器——不放行输出，不放行下一条直到当前完整处理结束。

2026-07-29 grill 定案——Event Buffer/Context/LLM 链路：`Component 交互 → Event Buffer（积压） → LLM 触发时合并为 system 消息 → Queue → Context 写输入 → LLM → assistant 回复 → Context 写输出`。两边输入最终汇入同一条 Queue→Context→LLM→Context 管道。

2026-07-29 修复——先文档后代码两轮对齐。文档：concepts §3a/§5/§10a/§10b/§10d/§13 修正 + Data Flow 重画，harness/storage/agent-loop 等 10 文件同步（Queue=输入串行化关口、Context=完整消息数组、EventBuffer=放行附带、Compression 作用于 Context、storage 新增 queue.jsonl 排队轨迹节）。代码四连：①重命名归位——消息数组正名 Context/ContextMessage，终端内容存档正名 ContentArchive/ContentRecord；②真 Queue 输入排队器——QueueInput+FIFO，handle_hook/handle_real_hook/handle_timer_scan 改生产者入队即返（不再持锁等 LLM），spawn_queue_consumer 单消费者串行放行（放行→Context 写输入→LLM→写输出→下一条），queue.jsonl append-only 留痕；③EventBuffer 附带语义——merge 锚到放行点，system 输入合并为一条消息，user 输入不污染 user role；④IPC/HTTP 面更名——GET /context、context_changed、get_context、ContextMessage。78 测试绿 + 前端构建绿，生产冒烟用户确认 harness 可用。

***

**触发场景**: 用户拖动 card/chat 窗口到合适位置；通过托盘按钮切换 pet 显示/隐藏。

**表现**: 窗口被从 A 拖到 B 后，pet 一移动窗口就弹回 A；重启后拖过的位置也丢失；pet 隐藏期间新 card 窗口仍然弹出。

## #12 窗口拖动位置不持久、不作为跟随基准 (2026-07-29) — open

card/chat 窗口从自动布局位置 A 拖到 B 后，新位置只生效于当下：(1) pet 移动时窗口跟随的偏移基准仍是 A——pet 一动窗口就恢复显示在 A 处，而不是以用户拖到的 B 为新偏移基准跟随；(2) 拖到的位置没有持久化，重启后丢失。#8 预期的「松手后记录新位置到布局引擎，后续 pet 移动时以新位置为偏移基准重新跟随」未真正落地。建议：拖动松手即更新布局引擎中该窗口的偏移基准，并持久化（重启后恢复）。

## #13 pet 隐藏后新 card 窗口仍然弹出 (2026-07-29) — open

托盘切换为隐藏后（pet/chat 隐藏、cards:hide 广播已发出的存量窗口），后续 ペット call_component 触发的新 card 窗口仍然会创建并显示。隐藏状态应对 card 窗口全局生效：隐藏期间新 card 不弹出（延迟到恢复显示时呈现，或直接抑制创建）。

***

**触发场景**: 用户点击 card 窗口右上角 × 按钮；拖动 card 到屏幕边缘。

**表现**: × 按钮点击后 card DOM 移除但 Tauri 窗口未关闭、引擎占区未清除。拖动 card 到屏幕边缘后，OS 自动调整窗口位置，engine 记录的仍是拖拽松手前的坐标，后续 pet 移动时恢复位置错乱，多 card 重叠。

## #14 card × 按钮无法关闭窗口 (2026-07-29) — open

card 窗口右上角 × 按钮点击后，ComponentManager 移除了 card DOM，MutationObserver 检测到无 `.component` 后隐藏窗口——但 Tauri 窗口本身未关闭（`win.close()`），引擎占区未清除（`engine.remove()` 未调用）。导致窗口泄漏、引擎 occupied 残留。

## #15 card 拖到屏幕边缘后引擎记录错乱导致重叠 (2026-07-29) — open

拖动 card 到屏幕边缘时 OS 自动调整窗口位置，但 `startDragging()` 松手时记录的位置是 OS 调整前的坐标。后续 pet 移动时 `engine.restoreAll` 用错误坐标恢复 → 多 card 挤在一起重叠。需要在松手时用 `win.outerPosition()` 获取 OS 调整后的实际位置更新 engine。
