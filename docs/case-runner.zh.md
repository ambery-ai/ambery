# Case Runner

[English](case-runner.md) | 中文

Storage 快照驱动的回归测试与概念观测工具；兼承接 CLI 决策源（docs/debug-agent.md）与完整 router（docs/core-server.md）。

## 原则

> **本文档范围**——本文只定义 runner 基础设施：运行机制、沙盒隔离、`.case` 格式、step、observe、health、导出与 CLI；不定义 agent 应具备的能力、能力计划或 Case 覆盖策略，见 `docs/capability-evaluation-project.md`。

> **快照即真相**——case 的 data 节保留 JSONL 原文，行序/时间戳/字段不丢；细节见 §快照即真相。

> **边界隔离**——一次性沙盒（生产永不写）+ headless（不启动真实 OS 界面）；细节见 §边界隔离。

> **前端 headless 观测**——观测边界与接入形态（headless JS + RemoteBridge 连 case-runner 内嵌 core + mock 窗口层，即 `app/test/` vitest 套件）；细节见 §前端 headless 观测。

> **壳类比**——case-runner 类比 Tauri 壳：进程主体内嵌 core（run_core 同款），按需拉起 TS 测试进程，TS 走 RemoteBridge 连内嵌 core——与「壳内嵌 core + 壳驱动 WebView」同构。对应 Tauri 多窗口（每窗口一个独立 Renderer），一个 TS 测试进程模拟一个窗口的 JS 运行时；多窗口场景用多个 TS 测试进程（多 node），各自经 RemoteBridge 连共享的内嵌 core。TS 不是常驻环境，是 case-runner 流程的一环。

> **LLM 模式**——两种平级（debug / real），case 头部显式声明，无隐含默认；细节见 §LLM 模式。

> **核心概念 5 个**——case / observe / chat(=user+real llm) / toolcall / worker 是第一类对象。

> **可观测体系**——所有模块可观测、observe 两类输出、effects 进 observe；细节见 §可观测体系。

> **Tauri 运行时动作可观测**——非只读 Tauri 运行时动作统一进 effect 流；细节见 §Tauri 运行时动作可观测。

> **case 隐私边界**——case 绝不携带敏感信息（apikey / 项目名等）；细节见 §case 隐私。

## 哲学

case = storage 快照 + meta，无内嵌断言。runner 不判对错——它观测概念结构的行为：
Context 能否 replay、panorama 包含谁、timer scan 产出了什么、queue/event_buffer 状态如何。

**边界隔离**：runner 只读写一次性沙盒（`%TEMP%\ambery-case-<case_id>/`，开跑即重建）：storage 快照、运行期追加、config 落盘全部在沙盒内；生产 storage/config **永不写**。**headless**：绝不启动真实 OS 界面（窗口/显示器/输入），前端/OS 层永不在 case 观测范围。唯一例外：real LLM 模式只读生产 config 的 llm 段（providers）+ 经 env key 发网络请求。debug 模式零网络、完全确定性。

## 前端 headless 观测

case-runner 的观测范围覆盖前端 TS 能力：headless JS runtime 跑真实前端模块（vitest + jsdom）+ RemoteBridge 连内嵌 core + mock 窗口层（§运行形态）。

### 观测边界

| effect | 观测方式 |
|---|---|
| `render_component` / `close_component`（后端） | 沙盒 effect.jsonl |
| `window_opened` / `window_closed` / `window_visible` / `window_hidden` / `window_moved` / `window_resized`（前端窗口层） | 已实现：headless/browser 窗口层经 `tauri_runtime_actions` 产生 window_* effect，经 RemoteBridge `POST /effect` 入动作流（`app/test/window-case.test.ts`） |
| `window`（observe target） | 已实现：打印窗口动作序列，并断言不变量——已关闭窗口的 `render_component` 必须伴随新的 `window_opened`，违反即 FAIL |
| 前端非窗口逻辑（store / 窗口接线） | `frontend-case.test.ts`（headless JS + mock 窗口层） |

### 前端读取架构（store 收敛 + invoke 规则）

**store 机制**（`app/src/store.ts`）：
- 一个前端 store 持有 core 拥有的可读状态：`config` / `top_state` / `context` / `cards`；
- store 由 bridge 读方法刷新（基线拉一次 + 事件提示时按需重拉/直写），组件只 `store.<getter>` 取基线 + `store.onX(cb)` 订阅变化，不直接调 Tauri API；
- store 边界判据：**core 拥有 + 多窗口/组件读 + 变化驱动 UI + 体积可控**；前端局部瞬态（面板开关/输入框/拖拽中位置）与意图（invoke 写指令）不进 store。判据反例：设置面板 schema（menu 单消费者、体积大）不进 store，经 bridge `getConfigSchema()` 直读；
- `BrowserMockBridge` 即「持有 + 通知」形态的 store；每窗口各自一份 store 实例，经各自 bridge 喂入。

**invoke 规则**：
- 读取 → 一律走 store（store 外读取走 bridge 读方法）；写入 → 一律走动作层（`tauri_runtime_actions.ts`）或 bridge 写方法；
- 主代码逻辑不出现 `invoke("get_*")` / 裸 `invoke` 散调；
- invoke 只允许出现在两个收口：**store 的刷新**（bridge 读方法实现）+ **动作层的写指令**。

### 窗口决策上提

窗口存在性由 `ensure_card_window` / `close_card_window` command 决定，Rust 侧权威注册表（`CardWindowRegistry`）同步决策 create / reuse / close，返回 `opened` / `reused` / `closed` / `absent`。

1. **关闭路径收口**：`close_card_window` 统一收口 agent close / shelf dismiss / 用户 × 三路径；destroy 不经 `onCloseRequested`，杜绝 preventDefault 僵尸。
2. **移除走事件循环**：`destroy()` 经 dispatcher 分发后异步生效——瞬时视图仍有「将死窗口」窗口期。
3. **`Closing` 状态吸收窗口期**：close 标 `Closing` → destroy → 等物理移除（兜底超时）→ 出表。
4. **ensure 语义**：见 `Closing` 等其出表再重建；见 `Alive` 但注册表无窗则自愈重建。

### 运行形态（headless JS + RemoteBridge）

`ambery-case frontend` 子命令内嵌 core，拉起 node + vitest + jsdom 子进程（沙盒 storage/config；bind 0 取空闲端口避让生产 47600）；真实前端模块经 RemoteBridge 连内嵌 core，`shim.ts` 提供 mock 窗口层；`frontend-case.test.ts` 首套，覆盖 store 基线与回读、Queue 放行回读、同 id 不重复/不复活（DOM 层）、统一关闭、windowed 不订全局流。

接入观测的形态（§壳类比）：

```
headless 前端 case =
  case-runner（`ambery-case frontend`：内嵌 core，拉起 vitest 子进程）
  + 真实前端模块（vitest + jsdom）：store / RemoteBridge / pet 浏览器分支 / ChatPanel / ComponentManager
  + RemoteBridge（createBridge 无 __TAURI_INTERNALS__ 时自动选中，HTTP+WS 连内嵌 core）
  + shim.ts：端口接线 + core 就绪等待 + 沙盒 effect.jsonl 读取 + MockWindow 注册表（不拦截 Tauri API）
  断言面：DOM 状态 + store 投影 + 沙盒 effect.jsonl（/debug/effect 注入驱动，不落 jsonl）
```

webview 必须挂真实窗口，Tauri/wry 无 headless webview（见 wry discussion #373）。

## 布局

workspace 根 `Cargo.toml`（members：core / observe-derive / ambery-case；exclude app/src-tauri 壳独立构建）：

```
ambery-case/                    ← workspace member
├── Cargo.toml                    ← deps: ambery-core (features=["case-runner"])
├── cases/                        ← .case 文件（两段式），一个 case 一个文件（gitignore）
│   └── closed-stale-cache.case
├── src/
│   ├── main.rs                   ← CLI 入口（运行/health/export 参数、沙盒 setup、step 循环）
│   ├── runner.rs                 ← step 执行器（load / timer_scan / hook / trigger / user / tool_call / store / terminal / terminal_gone / observe 打印）
│   └── export.rs                 ← 实时 storage → case 导出管线
└── README.md
```

```
ambery-core (feature "case-runner"):
  src/
    case.rs                       ← 两段式解析 + CaseStep/meta + CaseObserve 组装 + pre_parse_check
    eval.rs                       ← 求值引擎（Parser trait/四 parser/变量/类型/DirectToString，case-eval-system.md）
    observe.rs                    ← Observable trait + 各模块投影（observability.md）
    lib.rs                        ← cfg(feature = "case-runner") 暴露；Harness derive(Observe) 覆盖断言
observe-derive/                   ← proc-macro（derive(Observe)，case-runner feature 可选依赖）
```

> step 执行器在 ambery-case（binary 侧）而非 core：沙盒/终端剧情状态/打印是 CLI 关注点；
> core 只承载概念（解析/求值/观测），保持库纯粹。

## Case 文件格式（两段式，`.case` 后缀）

**快照即真相**：case 的 data 节保留 JSONL 格式原文（不解析为对象），行序原样、时间戳精确、不丢字段。真实 storage 的某时刻切片导入 → 手工最小化 → 封存为 case；读通道状态（哪个实例屏幕上有内容、哪个 tab 已消亡）不由数据表达，**由 steps 剧情表达**。

**JSON 头 + 纯 JSONL 数据区**。头部是正常 pretty JSON（meta / config / steps），
数据区按 `{"__section":"=== name ==="}` marker 行分节，每节就是 JSONL **原文**——
每行一条、零转义、可扫读可定点删改。文件整体**不是合法 JSON**（故用 `.case` 后缀，
工具链不会当 JSON 误解析）。

> **灵感来源**：glTF 的 `.glb`（Binary glTF）——JSON chunk 描述结构，BIN chunk 原文
> 载荷，JSON 不把二进制转义内嵌。本格式同构：JSON 头描述场景（meta/steps），JSONL
> 节原文承载快照，而非把 JSONL 转义成一个巨大的 JSON 字符串。

解析规则（唯一一条）：**首个 `{"__section":` 行（无缩进、行首）之前 = JSON 头；
之后按 marker 行分节，数据行归当前节**。marker 行本身是合法 JSONL（零外来语法）。

```json
{
  "meta": {
    "case_id": "closed-stale-cache",
    "created": "2026-07-28T14:30:00Z",
    "llm_mode": "debug",
    "notes": "UIA实证已不存在，storage 仍标记 processing。重现：timer scan 时 terminal_reader 返回旧缓存内容 → 不触发 closed"
  },
  "config": { "timer.interval_ms": 5000, "timer.tick_ms": 5000 },
  "steps": [
    { "load": {} },
    { "terminal": { "instance": "timer-probe", "content": "旧缓存内容——UIA 已不存在但 read_tab 仍返回" } },
    { "observe": ["agents", "panorama"] },
    { "timer_scan": {} },
    { "observe": ["agents", "panorama"] }
  ]
}
{"__section":"=== work_agents ==="}
{"hash":"d5ca2b62","name":"timer-probe","project":"filter-test","status":"processing","last_seen":1785123664753}
{"hash":"1a2887ef","name":"timer-probe","project":"filter-test","status":"processing","last_seen":1785123708559}
{"__section":"=== context ==="}
{"type":"message","role":"system","content":"实例全景同步...","ts":1785163138795}
{"__section":"=== queue ==="}
```

#### ⟡ 一致性剖析

Case 的 JSON 头只描述场景与步骤；Storage 快照必须按其真实格式作为原始载荷进入容器，而不是把 Markdown 等非 JSONL 文件转义塞入 JSON 字符串。JSONL 与 Markdown 分属各自原始区和边界规则，才能同时保持手工最小化、可读 diff 与文件级 replay。Case 若要验证某个持久概念，就应能够携带它的最小快照；导出以 inclusion bool 加类别过滤控制范围，避免“可观察”变成默认携带全部私人数据。

### 头部（JSON）

| 字段 | 必填 | 说明 |
|------|------|------|
| `meta.case_id` | ✓ | 唯一标识，建议 kebab-case |
| `meta.created` | ✓ | ISO 8601 |
| `meta.notes` | | 场景描述、预期行为、备注 |
| `meta.llm_mode` | ✓ | debug / real（无默认，缺声明不合法；见 §LLM 模式） |
| `config` | | 全字段覆盖（统一管道 apply_config_by_path 逐 path 应用） |
| `steps` | | 线性 step 序列（见下） |

#### LLM 模式

两种平级模式，无隐含默认；case 头部 meta 声明 `llm_mode`（debug / real）：

- `debug`：DebugAgent，零网络，决策由外部决策源注入（沉默 / 脚本 / CLI），保持确定性
- `real`：OpenAI 兼容真实端点——「我需要真模型」是 case 的固有输入，记入 meta `llm_mode`；实际 provider 从生产 providers 合并（声明的 provider 必须是生产 providers 的**子集**，只能选生产已配置的，不能引入新 provider）；key 只取环境变量

缺声明 `llm_mode` → case 不合法（报错），不隐含退回任一模式。metrics 类 case（token 影响 / 回答准确度）依赖 real；debug 下 user step 按决策源行为（沉默或脚本）。

**no_case_visible**：`llm.providers.*` 等敏感字段（含 base_url / api_key_env）在 case 里**禁止出现**——落盘/覆盖校验拒绝，case 绝不携带 apikey。

`tool_call` 是**直接执行**后端 tool 的机械 step：它绕过 LLM，不写 assistant(tool_calls) / tool result 到 Context，也不模拟“更早 response 看见 query result”这一协议。因此它适合测试 tool 的独立执行效果，不适合验证 agent 的多 response 工具策略。

剧情中途改配置不需要专用 step——需要测试单次后端写入时，可用 `tool_call` 调 `edit_config`；它与 pet 的生产修改共用 Config 管道。需要测试 agent 先读后写、读取快照、预算或 tool result 链路时，使用显式 real / scripted LLM response，让 `run_trigger` 走完整 assistant tool_calls → Context tool result → 下一 response 流程，而不是拼接两个 `tool_call` steps。

### 数据节（Storage 快照）

JSONL 节保留原文；每节均可为空——空节保留 marker，「刻意为空」与「忘了填」可区分。Memory / Cron 也可作为初始快照进入 case，但 export 默认不导出；必须同时经过显式 inclusion bool 与该类别的过滤器，才写入 case。

| 节 | 对应 storage 文件 | 导出边界 |
|------|-------------------|----------|
| `work_agents` | work-agents.jsonl | **默认过滤**；显式 inclusion + `--instances` 等过滤后才存在 |
| `context` | context.jsonl | 常规快照过滤 |
| `queue` | queue.jsonl（Queue 输入排队记录） | 常规快照过滤 |
| `memory` | memory/ | **默认过滤**；`--keep-memory` + `--memory <name,...>`；仅选择的普通记忆与显式选择的 `AGENTS` 进入快照，`index.md` 在沙盒重建 |
| `cron` | cron.jsonl | **默认过滤**；`--keep-cron` + `--cron-ids <id,...>`；每个选中 id 保留完整生命周期事件链 |

memory 节是 Markdown 原文区（§一致性剖析：JSONL 与 Markdown 分属各自原始区和边界规则）：每个文件以 `{"__file":"<path>"}` 标记行开头（合法 JSONL，与 `__section` 同款标记哲学），其后到下一标记的行为该文件原文——空行保留、零转义。path 白名单：`AGENTS.md` 或 `notes/<name>.md`（保留名不可作 note 路径）；`index.md` 不进 case（沙盒按已选普通记忆重建），`cards/` 不经此节（契约另定）。原文行不得以 `{"__` 行首开头——与节/文件标记语法撞车会静默断文件，parse 直接拒绝。cards/ 不经此节（Card 文件契约见 docs/components.md §Card 文件；case 不携带卡片快照）。

### case 隐私

case 是可共享/归档的场景快照，隐私细节：

- **LLM 模式记 meta**：`llm_mode`（debug / real）在 case 头部 meta，case 不携带 provider 配置 / key
- **no_case_visible**：`llm.providers.*` 等敏感字段（含 base_url / api_key_env）在 case 里禁止出现（落盘/覆盖校验拒绝）——case 绝不携带 apikey
- **work_agents 默认过滤**：含项目名 `project·sid8`（暴露项目结构），默认不导出；只有显式声明才存在
- **Memory 默认过滤**：长期理解可能含跨项目事实与协作偏好；必须 `--keep-memory` + `--memory <name,...>` 双重显式选择。普通记忆按 name 选；Memory `AGENTS.md` 只有显式写 `AGENTS` 才保留原文；派生的 `index.md` 不导出，在沙盒按已选普通记忆重建
- **Cron 默认过滤**：计划 message 可能含工作内容；必须 `--keep-cron` + `--cron-ids <id,...>` 双重显式选择。选中计划保留 create / fire / delete 完整生命周期事件，不能按时间窗裁断因果链
- **context / queue 保留**：真实对话/输入快照（"快照即真相"），共享给信任方
- 文档/示例不出现真实 provider 名（隐私）

### steps

steps 是线性序列，每个 step 有类型 + 可选参数。runner 按序执行。

| step | 参数 | 作用 |
|------|------|------|
| `load` | — | replay storage 快照，重建所有概念结构：Harness::load → 内存 queue/context/filtered_content/event_buffer/agents |
| `terminal` | `{instance, content}` | 读通道剧情：设定该实例屏幕内容（timer_scan 时 terminal_reader 返回它） |
| `terminal_gone` | `{instance}` | 读通道剧情：该实例 tab 消亡（terminal_reader 返回 None）——消亡是显式动词，扫读剧本一眼可见 |
| `timer_scan` | — | 跑一轮 timer 周期：TimerWheel due 取到期实例 → Some 走 handle_timer_scan（变化检测入队）→ None 判 mark_instance_closed；最后 drain_queue 放行 |
| `hook` | `{event, name, project, content, ts?}` | 注入 mock hook 事件：handle_hook（按事件分层：session_start 静默簿记 / stop 入队）→ drain_queue 放行 |
| `trigger` | — | drain_queue：放行全部待放行输入 → LLM/effects |
| `user` | `{text, ts?}` | 用户消息入队（enqueue user role）→ drain_queue 放行 |
| `tool_call` | `[name, args_json]` | 绕过 LLM 直接执行 tool call：execute_tool → result + effects；不写 tool 协议消息，不能验证多 response 工具交互 |
| `store` | `{"<name>": {"type":"expr\|var\|int\|str", "value":"<字符串>"}}` | 设用户变量：value 经对应 parser 求值 → to_string → 存 string（见 case-eval-system.md） |
| `observe` | `[{"target":"agents"}, {"target":"context","lines":"[$tail-50,$tail]"}]` | 记录概念结构当前快照（统一对象列表；路径类 target 可带 lines 读取，见 case-eval-system.md） |

读通道默认状态：未设定的实例一律返回 `None`（= tab 不复存在）。僵尸类 case 不需要
任何 `terminal` step——默认全消亡，timer_scan 直接走 closed 判定。

### observe 输出

**可观测体系**：所有概念模块都可观测（覆盖机制见 `docs/observability.md`）；observe 输出两类（值：agents / queue / event_buffer / usage / answer / panorama / memory / cron / cards + 现算 filtered_content；路径：context / effects 给文件指针+摘要）；effects 进 CaseObserve（穷尽 match 编译期强制）+ effect.jsonl 全量持久化（含 assistant_delta/done，docs/storage.md §effect.jsonl）；filtered_content 不持久化，从 terminal-content.jsonl 原文 digest 现算。

**Tauri 运行时动作可观测**——`(Tauri runtime actions − readonly) ⊆ effects`：Tauri 运行时动作涵盖 WebView 的 `@tauri-apps/api` 与 Rust 壳的 `tauri` API；两侧所有非只读动作统一进 effect 流，只读调用走 observe 直接观测。

| 类别 | 例子 | 去向 |
|---|---|---|
| 非 readonly（有副作用） | WebView invoke 写（append_user / push_event / set_config）、WebviewWindow 创建/关闭、setSize / setPosition / emit；Rust 壳等价的窗口 show/hide/终结与 emit | → effect 流（调用侧记录到 core；一个运行时动作一条 effect，高频同类动作按规则打包） |
| readonly（无副作用） | WebView invoke 查询（get_state / get_context / get_config / get_config_schema）、读显示器 / outerPosition / getByLabel、listen；Rust 壳查询 | → observe 观测（不进 effect） |

**子集表示**：非 readonly 的 Tauri runtime actions ⊂ effects；一次调用产生多个运行时动作时分别记为对应 effect，readonly 走 observe 值/路径观测。

每个 observe step 按请求项分节打印（内容级，全文不截断）：

| observe 项 | 类别 | 产出内容 |
|------------|------|----------|
| `agents` | 值 | 全量 agent 条目，含 hash / name / project / status / last_seen |
| `panorama` | 值 | `panorama()` 投影文本（非 Closed 实例摘要；无存活实例显示 `(无存活实例)`） |
| `context` | 路径 | 无 lines：`文件路径 \| N 行 \| M tokens（真值 P + est 增量 D）`（真值锚点与 est 增量分开标注）；带 lines：context.jsonl 切片原文（含行号） |
| `filtered_content` | 值（现算） | Filtered 内容存档全量（instance / source / 归一全文 / ts）——agent 实际读到的终端内容 |
| `queue` | 值 | Queue 当前待放行输入（role / content） |
| `event_buffer` | 值 | Event Buffer 当前积压原文 |
| `usage` | 值 | 最近一次 LLM 调用真值（prompt_tokens / completion_tokens / ts；无真值显示 `(无)`）——「scan/回答对 token 的影响」的直接量规 |
| `memory` | 值 | Memory index 摘要（条目 name / description / 总数）；不默认展开 Markdown 正文 |
| `cron` | 值 | 当前持久化计划投影（id / schedule / message / next_due）；不含进程内、非持久化的 sleep waiter |
| `cards` | 值 | Card 注册表投影（id / type / title / created / user_closed / layout 摘要）；不展开 component 全文（原文在沙盒 memory/cards/） |
| `effects` | 路径 | 无 lines：`文件路径 \| N 条`；带 lines：effect.jsonl 切片原文（含行号，origin / kind / payload / ts） |
| `answer` | 值 | 最后一条 assistant 消息原文（无则 `(无)`）——「回答准确度」扫读位 |

值类 target 带 lines 不合法（health pre-parse 静态拦截）。CLI 输出格式：

```
── observe @ step 4 ──
agents:
  timer-probe (d5ca2b62) [processing] last_seen=1785123664753

panorama:
  实例全景同步（归零重 diff，1 个存活实例）：
  - timer-probe [Processing] project=filter-test

context: %TEMP%/ambery-case-x/context.jsonl | 12 行 | 3100 tokens（真值 3000 + est 增量 100）

effects: %TEMP%/ambery-case-x/effect.jsonl | 4 条

context: …/context.jsonl | 切片 [74,122]（共 122 行）   ← 带 lines "($cursor,$tail]" 时
  74: {"type":"message","role":"user","content":"…","ts":…}
```

## 导出工具：实时 storage → case

使用真实 storage 文件构造 case，通过细粒度过滤减少体量、最小化手修量。

### 管线

```
实时 storage → 过滤 → 最小化 → JSON 输出 → 手修 meta/notes → case health → 就绪
```

### 过滤器参数

过滤阶段只控制哪些行进入 case，不修改行内容。`work_agents`、`context`、
`queue` 跨文件联动过滤——以 `--instances` 为核心，波及所有包含 instance 引用的文件。

| 参数 | 作用 |
|------|------|
| `--instances a,b` | 只保留指定 instance 的行。work-agents：按 name 匹配；context/queue：按 content 中包含的 instance 标记过滤 |
| `--window 30m` | 只保留相对最新行 N 分钟窗口内的行 |
| `--before 1785164703801` | 只保留该 ts 之前的行 |
| `--after 1785164651768` | 只保留该 ts 之后的行 |
| `--keep-last N` | work-agents 每 hash / context 的 content 行每 instance 只保留最后 N 行 |
| `--keep-agents` | 默认不导出 work_agents 节（含项目名，隐私）；显式指定才导出完整 work_agents 节 |
| `--keep-memory` | 打开 Memory 导出资格；必须配合 `--memory <name,...>`，单独指定报错；不会默认带入任何 Memory 文件 |
| `--memory name-a,AGENTS` | Memory 文件过滤器（仅与 `--keep-memory` 同用）：普通记忆按 name 选；保留值 `AGENTS` 可显式带入用户维护的 Memory 导航原文；`index.md` 不可选、不导出，在沙盒按已选普通记忆重建 |
| `--keep-cron` | 打开 Cron 导出资格；必须配合 `--cron-ids <id,...>`，单独指定报错；不会默认带入任何计划 |
| `--cron-ids id-a,id-b` | Cron 计划过滤器（仅与 `--keep-cron` 同用）：按计划 id 选择，选中 id 的 create / fire / delete 行完整保留，不受时间窗逐行裁断 |
| `--trim-context` | 跨文件过滤后深度裁剪（需配合 --instances）：content 行只留保留实例（孤行清理）；message/queue 行按实例名提及过滤；autonomy/session/head/compact_boundary 装配留痕丢弃（case replay 不入内存；多 run 切片类 case 勿用） |

### 最小化参数

最小化阶段在过滤后对行内容做瘦身（不改结构），减少手修量：

| 参数 | 作用 |
|------|------|
| `--dedup` | 相邻 content 完全相同的行只保留最早一条 |
| `--strip-content N` | context 每行 content 截断到 N 字符（保留完整 ts、type、role 等元数据） |
| `--dry-run` | 预览过滤后各文件行数，不生成 case 文件 |

### 手修

导出生成的 JSON 中，`data` 字段均可手改——删冗余行、增补注释、用 `terminal` / `terminal_gone`
step 构造读通道剧情。手修完成 → 立即跑 case health 验证。

### 示例

```bash
# 从当前 storage 导出场景（--keep-agents 显式保留 work_agents；默认隐私过滤）
ambery-case export --case-id closed-stale-cache \
  --instances timer-probe,full-body-check \
  --keep-last 1 --keep-agents --trim-context

# 预览行数
ambery-case export --case-id closed-stale-cache --instances timer-probe --dry-run
# → work_agents: 2 行, context: 45 行（已 trim）, queue: 8 行（dry-run 预览，未生成 case）

# 验证 case 合法性
ambery-case cases/closed-stale-cache.case --health
# → PASS
```

## case health

手修完成后强制运行，验证 case 在当前代码版本下是否合法。

### health 检查项

1. **两段式可解析**：JSON 头是合法 JSON；marker 行符合 `{"__section":"=== name ==="}`（无缩进、行首）。
   数据区按节分两类：JSONL 节（work_agents / context / queue / cron）每行（含 marker 行）可由
   `serde_json::from_str` 解析；memory 节是 Markdown 原文区，按 `{"__file":...}` 标记分文件
   （路径白名单与标记冲突由 parse 强制），不适用逐行 JSON 校验
2. **Harness::load 不 panic**：所有 storage 行 replay 成功，无 schema 错误
3. **概念结构完整**：
   - Agent：每个 entry 有 hash / name / project / status
   - Context：每行有 type / ts / content（message 型必有 role）
   - Queue：每行有 role / ts
4. **observe 可执行**：`observe()` 返回所有概念结构快照无 error
5. **pre-parse 预检**（case-eval-system.md §checkhealth，静态不执行）：所有表达式（observe 的 lines / store 的 value）try_parse 语法合法；变量引用有效（`$tail` 预定义、用户变量使用前已 store）；store 类型合法（expr/var/int/str）；类型可落（Output 实现 DirectToString）；observe target 合法（可观测模块）

### 退出码

```
0 = PASS（所有检查通过）
1 = FAIL（检查失败，打印具体失败行）
2 = USAGE（参数错误）
```

## CLI

```
ambery-case <case>                    # 执行所有 steps，observe 输出到 stdout
ambery-case <case> --step-num 2       # 仅执行到第 N 步（含 observe）
ambery-case <case> --health           # 验证 case 合法性
ambery-case serve [--brain-addr <url> | --silent]    # 完整 router 宿主（浏览器调试 RemoteBridge；docs/core-server.md）
ambery-case frontend [--brain-addr <url> | --silent] # 前端 headless 模式：内嵌 core + 拉起 vitest 子进程（§壳类比）
ambery-case export [--storage DIR] [--instances a,b] [--window 30m] \
              [--before TS] [--after TS] [--keep-last N] [--keep-agents] [--trim-context] \
              [--keep-memory --memory name-a,AGENTS] [--keep-cron --cron-ids id-a,id-b] \
              [--dedup] [--dry-run] [--case-id ID]   # 从实时 storage 导出 case（stdout）
```

`serve` / `frontend` 共享 `core::host` 装配骨架（Config/Harness/LLM/Terminal Adapter → AppState → 完整 router）；端口默认 47600、`AMBERY_PORT` 覆盖（frontend 默认 bind 0 取空闲端口避让生产）。llm `active=debug` 时两者都必须显式给 `--brain-addr` 或 `--silent`（同 §LLM 模式 debug 决策源规则）。

## 用例：debug_brain.py 当 LLM（debug 模式）

debug_brain.py 是本地 OpenAI 兼容 HTTP 服务器（docs/debug-agent.md），当 LLM 用需先手动起它，再让 case-runner 以 debug 模式连：

```bash
# 终端 1：起 brain（打印监听端口；--port 可选，默认 47777）
python scripts/debug_brain.py --port 47777

# 终端 2：case-runner debug 模式，--brain-addr 连它
ambery-case <case> --brain-addr http://127.0.0.1:47777
```

- debug 模式必须显式给 `--brain-addr <url>` 或 `--silent`，缺省报错（本文 §LLM 模式）。
- brain 内置最小阈值决策源：hook 内容 ≥ 80 字 → 回通知 tool；否则沉默。

## 构建

```bash
# workspace 根（推荐）
cargo run -p ambery-case -- ambery-case/cases/closed-stale-cache.case          # 执行所有 steps
cargo run -p ambery-case -- ambery-case/cases/closed-stale-cache.case --health # case 合法性校验

# crate 内亦可（workspace 感知）
cd ambery-case
cargo build
cargo run -- cases/closed-stale-cache.case
```

`ambery-core` 需 `case-runner` feature 编译：`cargo build --features case-runner`（`ambery-case` 已自动启用）。
