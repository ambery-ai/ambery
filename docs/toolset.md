# Tool Set

ペット可调用的九个 function definitions（call_component / fetch_terminal / set_autonomy / edit_config / read_memory / write_memory / cron_create / cron_delete / sleep）。校验规则定义后端执行时的合法性与错误返回。

## 本文档范围

本文定义每个 tool 的参数 schema、返回结构与调用语义；工具轮次预算和触发终止策略见 `docs/agent-loop.md`。

## call_component

创建/更新/关闭卡片窗口。同一 id 首次创建、后续原地更新。Tool schema 以 `anyOf` 声明每种类型的完整字段。

| 参数 | 类型 | 必填 | 校验 |
|------|------|------|------|
| `spec.id` | string | ✓ | `[A-Za-z0-9_\-/.]+`，不为空。不含空格、中文、特殊字符 |
| `spec.type` | string | ✓ | `text_card` / `quick_jump` / `git_display` / `data_chart` / `todobox` |
| `spec.direction` | string | | `auto` / `n` / `ne` / `e` / `se` / `s` / `sw` / `w` / `nw` |
| `spec.title` | string | type = text_card/git_display/data_chart/todobox 时必填 | 非空 |
| `spec.text` | string | type = text_card 时必填 | 非空 |
| `spec.label` | string | type = quick_jump 时必填 | 非空 |
| `spec.target` | string | type = quick_jump 时必填 | 非空 |
| `spec.items` | array | type = todobox 时必填 | `[{text: string, done: boolean}]` |
| `spec.entries` | array | type = git_display 时必填 | `[{hash: string, msg: string, time: string}]` |
| `spec.diff` | string | type = git_display 时可选 | — |
| `spec.chart` | object | type = data_chart 时必填 | `{kind: "line"/"bar"/"pie", labels: string[], series: [{name, data}]}` |

**return**

| 情况 | 返回 |
|------|------|
| 创建成功 | `{"ok": true, "rendered": "<spec.id>"}` |
| 更新成功 | `{"ok": true, "updated": "<spec.id>"}` |
| 关闭成功 | `{"ok": true, "closed": "<spec.id>"}` |
| id 不合法 | `{"ok": false, "error": "spec.id '<id>' 不合法：…"}` |
| 缺少必填字段 | `{"ok": false, "error": "<type> 缺少必填字段：…"}` |
| type 不合法 | `{"ok": false, "error": "未知 Component type：'<type>'"}` |

**effect**：`RenderComponent(spec)`（创建/更新） ——前端创建/更新独立 Tauri 窗口渲染卡片

## fetch_terminal

按需读取指定实例的当前 Terminal Content。

| 参数 | 类型 | 必填 | 校验 |
|------|------|------|------|
| `instance` | string | ✓ | 非空字符串 |
| `vd_switch` | boolean | ✓ | `true` / `false` |

**return**

| 情况 | 返回 |
|------|------|
| 读到内容 | `{"ok": true, "instance": "<inst>", "content": "<全文>"}` |
| instance 为空 | `{"ok": false, "error": "instance 必填"}` |
| vd_switch 缺失 | `{"ok": false, "error": "vd_switch 必填（…）"}` |
| 读不到 | `{"ok": false, "error": "读不到 <inst>…}"}` |

**effect**：无注入——结果直接以 `tool` message 回 Context（读取副作用：原文存档 + 内存 prev 更新）

## set_autonomy

覆盖 Autonomy 的表情/移动。ttlMs 后回落默认。

| 参数 | 类型 | 必填 | 校验 |
|------|------|------|------|
| `key` | string | | `kaomoji.system` / `kaomoji.user` 并集中的唯一 key（如 `idle`/`notify`/`processing`） |
| `motion` | string | | `still` / `float` / `bounce` / `shake` |
| `ttlMs` | integer | | ≥ 0；与 `once: true` 不能同时传 |
| `once` | boolean | | `true` 时按 motion 的 `MotionDef.durationMs` 自动取持续时间；与 `ttlMs` 不能同时传 |

**return**

| 情况 | 返回 |
|------|------|
| 校验通过 | `{"ok": true}` |
| key 无效 | `{"ok": false, "error": "无效 key：'<v>'"}` |
| motion 不合法 | `{"ok": false, "error": "motion '<v>' 不合法，合法值：still/float/bounce/shake"}` |
| `once` 与 `ttlMs` 同传 | `{"ok": false, "error": "once 与 ttlMs 不能同时传（…）"}` |

**effect**：`SetAutonomy { face, motion, ttl_ms }` ——Autonomy 引擎覆盖表情/移动

## Memory 与 Cron

以下 tool 访问 Harness 的 Memory / Cron 持久化概念；它们不是让 Agent 直接读写任意文件或执行任意命令。Memory / Cron 的数据边界见 `docs/harness.md`。

| tool | 契约 |
|---|---|
| `read_memory` | 读取持久化理解 Markdown；name 省略 = 读 index.md 导航。完整 schema 见 `docs/memory.md` |
| `write_memory` | 新建或完整替换一条碎片化记忆；必须附 description，受单文件长度上限约束。完整 schema 见 `docs/memory.md` |
| `cron_create` | 创建 Harness 持久化计划（schedule: at/every_ms + message）。完整 schema 见 `docs/cron.md` |
| `cron_delete` | 删除一个持久化计划（id）。完整 schema 见 `docs/cron.md` |
| `sleep` | 经同一 Harness 调度器延迟 tool result（0–300000ms），等待后继续既定工具序列。完整 schema 见 `docs/cron.md` |

## edit_config

单一 Config 工具。所有行为由必填 `action` 显式区分；不以缺参、空值或失败写入切换模式。Config 投影、可见路径、`query` 快照与统一写入管道见 docs/config.md。

| action | 参数 | 语义 |
|---|---|---|
| `grep` | `pattern: string` | 用 Rust regex 搜 LLM 可见节点的 path 与中文 desc；返回按 path 排序的候选，不返回 value |
| `query` | `path: string`, `view?: "children" / "object"` | 按精确 path 查询一个节点；叶子带 value，容器默认展开一层，`view=object` 读取完整容器值 |
| `update` | `path: string`, `value: any` | 修改精确 path；需要更早 response 中、仍有效的完整 query 真值快照 |

**return**

| 情况 | 返回 |
|---|---|
| `grep` 无匹配 | `{"ok": true, "matches": []}` |
| `query` 成功 | `{"ok": true, "node": {…}, "children": […]}` |
| `update` 成功（热字段） | `{"ok": true, "path": "…", "msg": "已生效"}` |
| `update` 成功（冷字段） | `{"ok": true, "path": "…", "restartRequired": ["…"], "msg": "已保存，重启应用后生效"}` |
| 非法 action / 参数 / regex / path | `{"ok": false, "error": "…"}` |
| query 快照缺失或失效 | `{"ok": false, "error": "缺少已读快照：未读取目标当前值。请先 query"}` |
| 验证失败或只读降级 | `{"ok": false, "error": "…"}` |

**effect**：成功 update 时 `ConfigChanged` ——前端重载配置
