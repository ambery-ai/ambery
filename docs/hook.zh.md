# Hook 契约（真实 Claude Code 接入）

[English](hook.md) | 中文

> 概念定义见 concepts.md §9/§9b。本文档定真实 hook 契约：事件分层、marker 定位、启动扫描、安装。
> mock 契约（docs/agent-loop.md §Mock Hook 契约）保留为 debug 手段。
> **设计原则：不做技术限制，越开放越好**——能力给足（agent 可切桌面、三模式可配），默认值保守，选择权全在用户。

## 链路形态

```
Claude Code 事件 → settings.json "command" hook → ambery-hook.ps1
  → 读 stdin JSON payload → 输出 sessionTitle（定位标记）+ POST /hook（fire-and-forget）
AmberyBackend → register-on-first-sight → 按事件分层处理
内容一律走读通道补（sidecar UIA），hook 只当触发信号（docs/sidecar.md）
```

hook 脚本永远 fire-and-forget（async + 短 timeout + 失败静默）——backend 不在线绝不可卡用户的 CLI；丢失的 hook 靠 register-on-first-sight 自愈。

## Payload（POST /hook）

| 字段 | 来源 | 说明 |
|---|---|---|
| `event` | hook_event_name | `session_start` / `user_prompt` / `stop` / `session_end` / `notification` |
| `session_id` | payload | **实例身份 = hash**（同名不同命，docs/storage.md） |
| `cwd` | payload | project = basename |
| `kind` | 脚本捎带 | `"claude"`（filter per-instance 策略的输入，docs/filter.md） |
| `prompt` | UserPromptSubmit | 用户输入全文 |
| `message` | Notification | 通知文本 |
| `last_assistant_message` | Stop | 可选参考（内容以读通道为准） |

## 事件分层

| 事件 | backend 行为 | 落点 | 状态迁移 |
|---|---|---|---|
| `session_start` | 初见注册 + 定位探测（见下）；EventBuffer 记 `+ {name} 注册` | **EventBuffer**（最小文字） | → Idle |
| `user_prompt` | prompt 观察注入（`[观察] 用户在 {name} 输入：…`） | **Queue**（触发，pet 可沉默） | → Processing |
| `stop` | 双模式（`stop_hook_mode`，见下） | **Queue**（触发） | → Idle |
| `session_end` | 清定位缓存；EventBuffer 记 `− {name} 关闭` | **EventBuffer**（最小文字） | → **Closed**（真信号） |
| `notification` | message 注入 | **Queue**（触发） | — |

**stop 三模式**（`stop_hook_mode`，本地可配、`no_llm_visible`、热更新——每次 stop 现读）：

- `"queue_only"`（**默认 B**）：stop 只把 hint（payload 的 `last_assistant_message` 摘要）注入 Queue——宠物凭 hint 判「沉默/好奇」，好奇才 `fetch_terminal` 按需读（UIA 读只在需要时发生）
- `"auto_read"`（A）：stop 到达即 UIA 抓屏 → filter → 归一结果更新内存变化检测基准，注入 Queue 的是评估提示（「完成，Context 已更新（N 字）」形态）——归一全文不进 Queue/Context（docs/storage.md §filtered_content）；宠物要全文经 `fetch_terminal` 按需读。`read_tab` 在目标 tab **已选中时不切换**（C# 侧 alreadySelected 短路，无 200ms 等待）；未选中才切换（**不切回**）。`read_active_tab` 是非侵入只读变体（不切换、不排队，调试/当前窗口快读用）；**tab 切换限流：全局 5 秒内最多一次**，窗口期内的切换读请求排队等窗口（UIA Mutex 下自然串行）。读往返整体走 `spawn_blocking`，不阻塞 tokio worker（docs/sidecar.md §阻塞边界）
- `"message"`（C）：stop 把 `last_assistant_message` **全文**作为内容直接注入 Queue——agent 的汇报原文直达宠物（零 UIA，宠物读的是 agents 自己说的，不是屏幕）。形态：`[汇报] {name} 完成：{全文}`，全量不截断；为空时降级 hint 形态（「完成，无汇报内容」）

**agent 的 VD 切换能力**（开放原则）：不是独立 tool，是 `fetch_terminal` 的**必填字段**——打断性决策不能成为被遗忘的默认，每次调用显式面对：

```
fetch_terminal(instance, vd_switch: bool)   // 必填,忘传报错(失败信息即教学)
  vd_switch=false: 目标在当前 VD → 正常读
                   目标 cloaked → 调用失败,错误提示「目标在另一个虚拟桌面,用 vd_switch=true 重试」
  vd_switch=true:  目标 cloaked → 切到目标桌面 → 读 → 不切回(留在目标桌面,用户自己决定何时回)
                   目标在当前 VD → 字段无效,正常读
```

Timer/stop 的自动路径永远不切（后台无打断原则）。

**SessionStart 的 source 变体**：`startup` 正常注册；`resume` 同 session_id → 同 sid8 → 自然 upsert 复用（不出第二条）；`clear`/`compact` 不动身份，EventBuffer 记一笔。

**register-on-first-sight**：任何事件到达时未知 session_id 先落注册（first_seen = 后端初见时刻）再走事件语义——start 丢失（backend 当时不在线）只是「初见恰好是 stop」的普通情况，无特例代码。

**Processing 由「用户派活」驱动**，不是「CLI 开着」驱动：SessionStart → Idle，UserPromptSubmit → Processing，Stop → Idle，SessionEnd → Closed。Timer 的 None 消亡推断降为无 hook 实例的兜底。

## marker 定位（Hook → Tab）

**不变量：marker 前缀不可变，描述部分可演进。** 两个 hook 的 sessionTitle 输出都遵守：

```
SessionStart:      "<project>·<sid8>"
UserPromptSubmit:  "<project>·<sid8> | <prompt 前 N 字>"
```

**UserPromptSubmit 必须重发（不是可选）**：claude 会按 prompt 内容自动命名会话（实测：tab 名会按 prompt 内容自动生成）——marker 不自发重申就会被自动命名冲掉。这也是 marker 的**自愈机制**：title 被覆盖后，用户下一个 prompt 即复活。

claude 应用 sessionTitle 后 tab 名自带 project+sid8 → sidecar `find_tab`（Contains 匹配，✳ 前缀与 `| 描述` 后缀不影响命中）精确命中。session_title ↔ WT tab 名对应链成立（.last-title 缓存值与 UIA tab 名两对一致）；WT 窗口标题 = 活动 tab 标题。

**定位缓存**：注册表条目可带 `{hwnd, index}`——它是快照的普通字段（与 status 同待遇：append 即「更新」，投影得当前值，无原地修改）。惰性重试——session_start 时 tab 可能尚未改名（异步应用），之后每次读取（timer/stop/fetch）未命中就再按 marker 找，找到后快照自然带上；session_end 的 closed 快照置 null。

## 启动扫描

backend 启动一次性：list_windows → list_tabs，按 **claude 检测规则**（实测 54/54 命中、0 误伤）：

- tab 标题以 `✳` 开头（活动中的 claude 会话的活动 glyph），或标题 == `claude`（未命名会话）
- 其中**带 marker 的**（`·<sid8>`）解出 project+sid8 直接注册；**无 marker 的以占位身份入册**（hash = `uia:<tab标题>`，kind=claude）——启动即见全景；后续真身份补登（register-on-first-sight）时按标题关联：占位条目标 closed，真身份条目接管（append 日志，无原地改）
- **三方对账**（一行 EventBuffer 如实报告）：
  - `N` = Windows 进程列表中的 claude.exe 数（含子进程，**启发式参考值**，非会话数）
  - `M` = UIA 已定位的 claude tab 数
  - `K` = **cloaked 窗口数**（EnumWindows + `DwmGetWindowAttribute(DWMWA_CLOAKED)`；K>0 说明有窗口对其他 VD 不可读 → 提示开启 WT「全桌面显示」，docs/sidecar.md §视野模型）

装 hook 前开的旧会话不猜身份，等它们下一个事件的 register-on-first-sight。信息形态与 session_start 一致（EventBuffer）。

**timer 开关**：`timer.interval_ms ≤ 0 = 禁用`（docs/timer.md）。真实 hook 接入初期建议禁用——只留 hook 驱动，避免全量实例周期性扫描的 LLM 触发频率。

## sidecar 常驻（简化语义）

app 启动自动发现 exe 并启用（路径发现：`AMBERY_SIDECAR` env > 仓库约定位置 sidecar/bin/…/ambery-uia-sidecar.exe），进程惰性拉起（首次请求时 spawn）。**死了即弃，下次请求现拉起**（冷启实测 ~200ms）——无管道保活预检、无心跳，客户端实现 ~55 行。崩溃处理 = 每次请求最多两次尝试（拉一次、重试一次），仍失败返回 None（读通道降级回 Context）。

## 安装 / 卸载（scripts/install-hooks.ps1）

- **install**：hook 脚本复制到 `~/.claude/hooks/ambery-hook.ps1`；`~/.claude/settings.json` 追加 SessionStart / UserPromptSubmit / Stop / SessionEnd / Notification 五条 command 条目（**追加**，不动用户现有 hook）；改前备份 `settings.json.bak`
- **uninstall**：按标记移除五条条目 + 删脚本，settings 其余部分原样
- hook 脚本进仓库（`scripts/ambery-hook.ps1`，通用无隐私）；**真实样本/实测数据不进仓库**（隐私，实测归用户）

## 显式不做

- PreToolUse / PostToolUse / PreCompact / SubagentStop：当前粒度不需要
- Notification dedup：v1 全触发，AGENTS.md 教宠物沉默是常态；实测嫌吵再加时间窗（config 可配）
- opencode hook：体系不同，延期（docs/filter.md 开放问题）
- hook 自带内容（transcript 解析）不采用——读通道唯一（隐私面 + 双内容形态）
