//! Harness 内部语言（docs/i18n.md §Harness 内部语言）：供系统与 LLM 协作的人读文本。
//! 切换从下一次新的 LLM 交互开始生效——工具说明/参数说明在请求构建时现查表，系统事件
//! 文字在事件发生时现写，错误反馈在执行时现查；不改写已有 Context、历史 Chat 或已生成
//! 内容（持久化的 base_prompt / AGENTS.md 以首启时刻语言生成，属已生成内容）。
//! 机器契约（tool name / Config path / JSON key / 协议字段 / 枚举值）不在本表，永不翻译。
//!
//! 表体：(key, zh, en) 三元组；zh 为基准语言，en 为 AI 生成翻译（i18n.md §0.1.0 公开说明）。

/// Harness 语言（Config `harness_language` 的运行时形态）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn of(s: &str) -> Lang {
        if s == "en" {
            Lang::En
        } else {
            Lang::Zh
        }
    }
}

static TABLE: &[(&str, &str, &str)] = &[
    // ── 工具说明与参数说明（llm.rs tool_set）──
    ("tool.call_component.desc",
     "创建/更新/关闭卡片窗口。同 id 首次创建、后续原地更新；action=\"close\" 关闭（只需 id，忽略其他字段）。id 仅限 A-Z a-z 0-9 _ - . /",
     "Create/update/close a card window. Same id: first call creates, later calls update in place; action=\"close\" closes (only id needed, other fields ignored). id chars: A-Z a-z 0-9 _ - . /"),
    ("tool.call_component.spec", "ComponentSpec——按 type 选择分支填写对应字段", "ComponentSpec — fill the fields of the branch selected by type"),
    ("tool.call_component.id", "唯一标识：仅 A-Z a-z 0-9 _ - . /", "Unique id: A-Z a-z 0-9 _ - . / only"),
    ("tool.call_component.action", "设为 close 关闭卡片（此时只需 id，忽略其余字段）", "Set to close to close the card (only id is needed then; other fields ignored)"),
    ("tool.call_component.direction", "可选方位：auto/n/ne/e/se/s/sw/w/nw", "Optional direction: auto/n/ne/e/se/s/sw/w/nw"),
    ("tool.field.title", "卡片标题", "Card title"),
    ("tool.field.text", "卡片正文", "Card body text"),
    ("tool.field.label", "按钮标签", "Button label"),
    ("tool.field.target", "跳转目标", "Jump target"),
    ("tool.field.entries", "提交列表", "Commit list"),
    ("tool.field.diff", "可选的 diff 内容", "Optional diff content"),
    ("tool.field.chart", "图表定义", "Chart definition"),
    ("tool.field.items", "待办条目", "Todo items"),
    ("tool.fetch_terminal.desc",
     "按需读取指定实例的当前 Terminal Content。vd_switch 必填：false=不切桌面（读不到且目标可能在其他虚拟桌面时失败，提示重试）；true=目标在其他虚拟桌面时切过去读（不切回）",
     "Read the current Terminal Content of a given instance on demand. vd_switch required: false = never switch desktop (fails with a retry hint when unreadable because the target may be on another virtual desktop); true = switch over to read (does not switch back)"),
    ("tool.set_autonomy.desc",
     "覆盖 Autonomy 的表情/移动（ttlMs 后回落默认；全空=立即回落）。key 传状态 key 名（kaomoji 两池并集中的 key，如 idle/notify/processing）。once=true 按动画注册表 MotionDef.durationMs 自动取持续时间（一次播放收束），与 ttlMs 互斥",
     "Override Autonomy face/motion (reverts to default after ttlMs; all-empty = revert immediately). key takes a state key name (from the union of the kaomoji pools, e.g. idle/notify/processing). once=true derives the duration from the animation registry MotionDef.durationMs (one-shot), mutually exclusive with ttlMs"),
    ("tool.set_autonomy.once", "true=按 motion 的 MotionDef.durationMs 取 TTL；与 ttlMs 不能同时传", "true = TTL from the motion's MotionDef.durationMs; cannot be combined with ttlMs"),
    ("tool.edit_config.desc",
     "统一 Config 工具（受限投影，非法值被拒绝并返回错误）。action 必填：grep（pattern 正则搜 path/desc，不返回 value）→ query（精确 path；叶子带 value，容器默认 children 导航，view=object 读完整容器）→ update（path+value 写入，需更早 response 中仍有效的完整 query 快照）。可见顶层：kaomoji 表情状态映射、set_autonomy_default_ttl_ms Autonomy 默认持续时间、timer Timer 调度子树、view_scale/badge_style/badge_side View 外观、theme/themes 主题（当前主题名与主题 token 表）、ui_language/harness_language 语言（UI 与 Harness 内部文本语言）。路径未知先 grep，再 query；优先 view=children，必要时才对已定位的小 object 使用 view=object",
     "Unified Config tool (restricted projection; invalid values are rejected with errors). action required: grep (regex over path/desc, never returns value) → query (exact path; leaves carry value, containers default to children navigation, view=object reads a whole container) → update (path+value write, requires a still-valid full query snapshot from an earlier response). Visible top level: kaomoji face-state mapping, set_autonomy_default_ttl_ms Autonomy default duration, timer Timer scheduling subtree, view_scale/badge_style/badge_side View appearance, theme/themes (current theme name and theme token table), ui_language/harness_language (UI and Harness internal text languages). Unknown path: grep first, then query; prefer view=children; use view=object only for small located objects"),
    ("tool.edit_config.pattern", "grep 用：Rust regex，匹配 path 与中文 desc", "For grep: Rust regex matched against path and desc"),
    ("tool.edit_config.path", "query/update 用：精确点分路径", "For query/update: exact dot-separated path"),
    ("tool.edit_config.view", "query 容器视图：children 导航（默认）/ object 完整 JSON", "Container view for query: children navigation (default) / object full JSON"),
    ("tool.edit_config.value", "update 用：新值（JSON）", "For update: new value (JSON)"),
    ("tool.read_memory.desc",
     "读取持久化理解（Memory，跨 turn/压缩/重启保留）。name 省略 = 读 index.md 导航首页（全部记忆名称+描述）；index.md 与 AGENTS.md 可读不可写",
     "Read persistent understanding (Memory, kept across turns/compaction/restarts). Omit name to read the index.md navigation page (all memory names + descriptions); index.md and AGENTS.md are read-only"),
    ("tool.read_memory.name", "记忆名（小写 kebab）；省略读 index.md", "Memory name (lowercase kebab); omit to read index.md"),
    ("tool.write_memory.desc",
     "新建或完整替换一条持久化理解（Memory）。碎片化短小内容（≤4KiB）；必须附 description（进 index.md 汇总表）；无局部 patch，无删除（同名覆盖演进）",
     "Create or fully replace one persistent-understanding entry (Memory). Small fragmented content (≤4KiB); description required (goes into the index.md summary); no partial patches, no deletion (same-name overwrite evolves)"),
    ("tool.write_memory.name", "记忆名：小写字母开头，仅小写字母/数字/_/-", "Memory name: starts with a lowercase letter; lowercase letters/digits/_/- only"),
    ("tool.write_memory.content", "完整内容（Markdown，≤4096 字节）", "Full content (Markdown, ≤4096 bytes)"),
    ("tool.write_memory.description", "一句描述（单行、不含 |，≤80 字符；进 index.md）", "One-line description (single line, no |, ≤80 chars; goes into index.md)"),
    ("tool.cron_create.desc",
     "创建 Harness 持久化计划（跨重启保留；到点 message 作 system 输入唤醒你）。schedule 二选一：at=epoch ms 一次性 / every_ms 间隔周期。返回 id（可经 write_memory 记录供日后 cron_delete）",
     "Create a persistent Harness schedule (survives restarts; fires message as a system input that wakes you). schedule is either at=epoch ms one-shot or every_ms interval. Returns an id (record it via write_memory for later cron_delete)"),
    ("tool.cron_create.at", "epoch ms 一次性（须大于当前时刻）", "epoch ms one-shot (must be in the future)"),
    ("tool.cron_create.every_ms", "间隔周期 ms（>0，锚定创建时刻）", "interval in ms (>0, anchored at creation time)"),
    ("tool.cron_create.message", "到点注入 Queue 的内容（非空）", "Content injected into the Queue at fire time (non-empty)"),
    ("tool.cron_delete.desc", "删除一个 Harness 持久化计划（id 见 cron_create 返回；无 list tool）", "Delete a persistent Harness schedule (id from cron_create return; no list tool)"),
    ("tool.sleep.desc",
     "等待后继续既定工具序列（与 Cron 共用 Harness 调度器）：tool result 延迟 ms 返回，期间 Queue 串行点被占用；0 ≤ ms ≤ 300000（5 分钟）；不持久化",
     "Wait, then continue the planned tool sequence (shares the Harness scheduler with Cron): the tool result returns after ms; the Queue serial point stays occupied meanwhile; 0 ≤ ms ≤ 300000 (5 minutes); not persisted"),
    ("tool.sleep.ms", "等待毫秒数（0-300000）", "Milliseconds to wait (0-300000)"),

    // ── 系统 prompt 段头（overseer.rs assemble_system_prompt）──
    ("prompt.kaomoji-header",
     "## 颜文字映射（你的面部表情词汇表：仅用于 set_autonomy 工具，严禁写进对话文本）",
     "## Kaomoji mapping (your facial-expression vocabulary: for the set_autonomy tool only; never write these into chat text)"),

    // ── 生命周期簿记（lifecycle.rs）──
    ("lifecycle.created", "card created: {type}「{title}」({id}) @ {ts}, → 存活 {n}", "card created: {type} \"{title}\" ({id}) @ {ts}, → alive {n}"),
    ("lifecycle.closed", "card closed: {type}「{title}」({id}), {start} / {end}, → 存活 {n}", "card closed: {type} \"{title}\" ({id}), {start} / {end}, → alive {n}"),
    ("lifecycle.user-close", "用户关闭了 {type}「{title}」({id})", "User closed {type} \"{title}\" ({id})"),
    ("lifecycle.user-action", "用户{action} {type} 条目「{text}」", "User {action} {type} item \"{text}\""),

    // ── Component 交互事件文本（lifecycle.rs user_action_desc；前端只报结构化事实）──
    ("ev.copy", "用户复制了 {type}「{title}」的内容", "User copied the content of {type} \"{title}\""),
    ("ev.jump", "用户点击 {type} 跳转到「{target}」", "User clicked {type} to jump to \"{target}\""),
    ("ev.expand-diff", "用户展开了 {type}「{title}」的 diff", "User expanded the diff of {type} \"{title}\""),
    ("ev.todo-toggle", "用户{verb} {type} 条目「{text}」", "User {verb} the {type} item \"{text}\""),
    ("ev.verb-checked", "勾选了", "checked"),
    ("ev.verb-unchecked", "取消勾选了", "unchecked"),
    ("ev.todo-add", "用户新增了 {type} 条目「{text}」", "User added the {type} item \"{text}\""),

    // ── Hook / 事件簿记与注入（overseer.rs）──
    ("hook.register", "+ {name} 注册 → 存活 {alive}", "+ {name} registered → alive {alive}"),
    ("hook.closed", "− {name} 关闭 → 存活 {alive}", "− {name} closed → alive {alive}"),
    ("hook.closed-timer", "− {name} 关闭（Timer 判死）→ 存活 {alive}", "− {name} closed (Timer sweep) → alive {alive}"),
    ("hook.user-prompt", "[观察] 用户在 {name} 输入：{p}", "[observed] User input in {name}: {p}"),
    ("hook.notification", "[{name}] 请求注意：{m}", "[{name}] requests attention: {m}"),
    ("hook.stop.updated", "{name} 完成，Context 已更新（{len} 字）。评估是否通知。", "{name} finished; Context updated ({len} chars). Evaluate whether to notify."),
    ("hook.stop.hint", "{name} 完成：{hint}。评估是否通知。", "{name} finished: {hint}. Evaluate whether to notify."),
    ("hook.stop.empty", "{name} 完成，无汇报内容。评估是否通知。", "{name} finished with no report content. Evaluate whether to notify."),
    ("hook.stop.report", "[汇报] {name} 完成：{hint}", "[report] {name} finished: {hint}"),
    ("hook.sweep-line", "启动扫描: {located} tab 已定位（{marked} marker / {placeholder} 占位），claude.exe 进程 {n}，cloaked 窗口 {cloaked_n}", "Startup sweep: {located} tabs located ({marked} markers / {placeholder} placeholders), claude.exe processes {n}, cloaked windows {cloaked_n}"),
    ("timer.scan.updated", "{name} 兜底扫描发现变化，Context 已更新（{len} 字）。评估是否通知。", "{name}: fallback sweep found changes; Context updated ({len} chars). Evaluate whether to notify."),
    ("hook.sweep-cloaked", "（有窗口对其他桌面不可读，可开 WT「全桌面显示」）", " (some windows are unreadable from other desktops; consider enabling WT \"show on all desktops\")"),

    // ── 工具执行错误反馈（overseer.rs execute_tool 等）──
    ("err.grep-pattern", "grep 需要 pattern（string）", "grep requires pattern (string)"),
    ("err.regex", "regex 非法：{e}", "invalid regex: {e}"),
    ("err.query-path", "query 需要 path（精确点分路径）", "query requires path (exact dot-separated path)"),
    ("err.no-access", "路径 '{path}' 不可访问（no_llm_visible 子树）", "path '{path}' is not accessible (no_llm_visible subtree)"),
    ("err.unknown-path", "未知 path：'{path}'。先 grep 定位后再精确 query", "unknown path: '{path}'. grep to locate it, then query exactly"),
    ("err.bad-view", "非法 view：'{other}'。合法：children/object", "invalid view: '{other}'. Valid: children/object"),
    ("err.object-view-leaf", "view=object 仅适用容器；叶子请直接 query 读值", "view=object only applies to containers; query a leaf directly for its value"),
    ("err.update-path", "update 需要 path（精确点分路径）", "update requires path (exact dot-separated path)"),
    ("err.update-value", "update 需要 value（JSON）", "update requires value (JSON)"),
    ("err.unknown-path-map", "未知 path：'{path}'。新增 map entry 请 query(view=object) 读整map 后 update 整个 map", "unknown path: '{path}'. To add a map entry, query(view=object) the whole map, then update the entire map"),
    ("err.no-snapshot", "缺少已读快照：未读取目标当前值。请先 query", "missing read snapshot: the target's current value was not read. query first"),
    ("err.size-limit", "结果 {size} 字节超过 1 KiB；{hint}", "result is {size} bytes, over the 1 KiB limit; {hint}"),
    ("hint.grep-narrow", "收窄 pattern：加更长关键词或限定 flag（如 (?i)timer）", "narrow the pattern: longer keywords or an inline flag (e.g. (?i)timer)"),
    ("hint.children", "children 过多：对已定位的小 object 改用 view=object，或先 grep 收窄", "too many children: use view=object on a small located object, or grep to narrow first"),
    ("hint.object", "object 过大：改用 view=children 逐层导航，或对叶子精确 query", "object too large: navigate with view=children layer by layer, or query a leaf exactly"),
    ("err.tool-budget-turn", "超本 turn 工具调用预算（{n} 次/turn）", "over the per-turn tool budget ({n} calls/turn)"),
    ("err.tool-budget-response", "超单 response 工具调用预算（{n} 次/response）", "over the per-response tool budget ({n} calls/response)"),
    ("err.unknown-tool", "unknown tool: {name}", "unknown tool: {name}"),
    ("err.instance-required", "instance 必填", "instance is required"),
    ("err.vd-switch-required", "vd_switch 必填（false=不切桌面；true=目标在其他虚拟桌面时切过去读，不切回）", "vd_switch is required (false = never switch desktop; true = switch over to read when the target is on another virtual desktop, no switch back)"),
    ("err.fetch-unreadable", "读不到 {inst}：可能不存在，也可能在另一个虚拟桌面；确认存在的话用 vd_switch=true 重试", "cannot read {inst}: it may not exist, or it may be on another virtual desktop; if it exists, retry with vd_switch=true"),
    ("err.vd-switch-failed", "切换失败：全 VD 窗口标题无 {inst} 匹配（可能不存在，或它是 cloaked 窗口的背景 tab）", "switch failed: no window title matches {inst} across all virtual desktops (it may not exist, or be a background tab of a cloaked window)"),
    ("fetch.switched-empty", "（已切换到目标桌面，但仍读不到内容）", "(switched to the target desktop, but content is still unreadable)"),
    ("err.autonomy-key", "无效 key：'{key}'", "invalid key: '{key}'"),
    ("err.autonomy-motion", "motion '{motion}' 不合法，合法值：{valid}", "invalid motion '{motion}', valid: {valid}"),
    ("err.autonomy-once-ttl", "once 与 ttlMs 不能同时传（once 按 MotionDef.durationMs 自动取持续时间）", "once and ttlMs are mutually exclusive (once derives the duration from MotionDef.durationMs)"),
    ("err.direction", "direction '{dir}' 不合法，合法值：auto/n/ne/e/se/s/sw/w/nw", "invalid direction '{dir}', valid: auto/n/ne/e/se/s/sw/w/nw"),
    ("err.git-entries", "git_display entries 结构不合法：需 [{hash, msg, time}] 字符串数组", "invalid git_display entries: expected [{hash, msg, time}] strings"),
    ("err.chart", "data_chart chart 结构不合法：需 {kind: line/bar/pie, labels: string[], series: [{name, data: number[]}]}", "invalid data_chart chart: expected {kind: line/bar/pie, labels: string[], series: [{name, data: number[]}]}"),
    ("err.bad-action", "非法 action：'{action}'（合法：grep/query/update）", "invalid action: '{action}' (valid: grep/query/update)"),
    ("cron.schedule-conflict", "schedule 二选一：at 与 every_ms 不能同时传", "schedule is either/or: at and every_ms cannot be combined"),
    ("cron.schedule-missing", "schedule 二选一：{at: epoch_ms} 或 {every_ms: N}", "schedule is either/or: {at: epoch_ms} or {every_ms: N}"),
    ("sleep.ms-required", "ms 必填（0 ≤ ms ≤ 300000）", "ms is required (0 ≤ ms ≤ 300000)"),
    ("sleep.ms-over", "ms {ms} 超上限 {max}（5 分钟，设计常量）", "ms {ms} exceeds the {max} limit (5 minutes, design constant)"),
    ("mem.content-required", "content 必填（完整替换，无局部 patch）", "content is required (full replacement; no partial patches)"),
    ("err.component-type", "未知 Component type：'{typ}'，合法值：{valid}", "unknown Component type: '{typ}', valid: {valid}"),
    ("err.component-id", "spec.id '{id}' 不合法：窗口名只允许 A-Z a-z 0-9 _ - . /，不含空格、中文或特殊字符；路径段不得为空或 '..'", "invalid spec.id '{id}': window name allows only A-Z a-z 0-9 _ - . / (no spaces/CJK/special chars); path segments must not be empty or '..'"),
    ("err.component-missing", "type={typ} 缺少必填字段：{missing}。字段在 spec 顶层，不要包在 props 里", "type={typ} missing required fields: {missing}. Fields live at spec top level; do not wrap them in props"),
    ("err.todobox-items", "todobox items 结构不合法：需 [{text: string, done: boolean}]", "invalid todobox items structure: expected [{text: string, done: boolean}]"),
    ("msg.saved-restart", "已保存，重启应用后生效", "Saved; takes effect after restart"),
    ("msg.saved-hot", "已生效", "Applied"),

    // ── Context 压缩摘要（context.rs compress）──
    ("context.summary", "[历史摘要] {summary}", "[History summary] {summary}"),

    // ── Memory 工具错误（memory.rs）──
    ("mem.no-index", "index.md 不存在（Memory 尚未初始化）", "index.md does not exist (Memory not initialized yet)"),
    ("mem.bad-name", "名称 '{name}' 不合法：小写字母开头，仅小写字母/数字/_/-，≤ {max} 字符", "invalid name '{name}': starts with a lowercase letter; lowercase letters/digits/_/- only; ≤ {max} chars"),
    ("mem.not-found", "记忆 '{name}' 不存在（见 read_memory() 的 index）", "memory '{name}' does not exist (see the index from read_memory())"),
    ("mem.reserved", "'{name}' 是保留名（index/AGENTS 默认只读）", "'{name}' is reserved (index/AGENTS are read-only by default)"),
    ("mem.too-large", "内容 {size} 字节超过上限 {max}：碎片化理解，勿整段写入", "content is {size} bytes, over the {max} limit: keep understanding fragmented; do not write long blobs"),
    ("mem.desc-required", "description 必填（非空，进 index.md）", "description is required (non-empty; goes into index.md)"),
    ("mem.desc-oneline", "description 必须单行且不含 '|'", "description must be a single line without '|'"),
    ("mem.desc-too-long", "description {len} 字符，超过 {max}", "description is {len} chars, over {max}"),
    ("mem.write-failed", "写入失败：{e}", "write failed: {e}"),
    ("mem.index-failed", "index.md 更新失败：{e}", "index.md update failed: {e}"),
];

/// 查表：当前语言命中即用；en 缺失回退 zh；key 缺失回退 key 本身（开发期显形）
pub fn tr(lang: Lang, key: &str) -> &'static str {
    for (k, zh, en) in TABLE {
        if *k == key {
            return match lang {
                Lang::Zh => zh,
                Lang::En => if en.is_empty() { zh } else { en },
            };
        }
    }
    // 未登记 key：返回 key 本身（测试/开发期可见，不静默丢文案）
    Box::leak(key.to_string().into_boxed_str())
}

/// 查表 + {param} 插值
pub fn trf(lang: Lang, key: &str, params: &[(&str, String)]) -> String {
    let mut s = tr(lang, key).to_string();
    for (k, v) in params {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_integrity() {
        // zh 非空；key 唯一
        let mut keys = std::collections::HashSet::new();
        for (k, zh, _) in TABLE {
            assert!(!zh.is_empty(), "{k} zh 为空");
            assert!(keys.insert(k), "重复 key: {k}");
        }
    }

    #[test]
    fn lookup_and_fallback() {
        assert_eq!(tr(Lang::Zh, "tool.sleep.ms"), "等待毫秒数（0-300000）");
        assert_eq!(tr(Lang::En, "tool.sleep.ms"), "Milliseconds to wait (0-300000)");
        assert!(tr(Lang::Zh, "no.such.key").contains("no.such.key"));
    }

    #[test]
    fn interpolation() {
        let s = trf(Lang::En, "hook.stop.hint", &[("name", "a".into()), ("hint", "done".into())]);
        assert_eq!(s, "a finished: done. Evaluate whether to notify.");
    }
}
