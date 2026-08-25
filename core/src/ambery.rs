//! AmberyBackend：触发循环 + tool 执行 + hook 处理。

use crate::content::RecordSource;
use crate::filter::{Change, Filter};
use crate::lifecycle::Lifecycle;
use crate::llm::{tool_set, Llm};
use crate::context::{ContextMessage, Role, ToolCall};
use crate::timer::TimerWheel;
use crate::{
    default_agents_md, AgentEntry, AgentStatus, Config, Harness,
    TerminalContentRecord, AGENTS_MD_FILE,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// 错误留存档位（错误通知的唯一分派轴）：前端按留存路由，不按来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorRetention {
    /// 瞬时：消息流气泡，一次一现
    Transient,
    /// 常驻：chat 顶部 banner，直至用户关闭（恢复再失败视为新条件重开）
    Persistent,
}

impl ErrorRetention {
    /// wire/记录形态（effect 载荷与前端路由共用同一对字符串）
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Persistent => "persistent",
        }
    }
}

/// 副作用：经 WS 广播给前端
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    RenderComponent(Value),
    /// 显式关闭卡片（Component 持续管理协议：action="close"）
    CloseComponent(String),
    SetAutonomy {
        face: Option<String>,
        motion: Option<String>,
        ttl_ms: Option<u64>,
        /// 一次播放收束：前端按 MotionDef.durationMs 取 TTL；与 ttl_ms 互斥
        once: bool,
    },
    /// llm_changed=true 时 server 广播前重建 LlmBackend
    ConfigChanged { llm_changed: bool },
    /// 错误通知（错误即通知模型）：message 用户可读文本；
    /// action 可选——banner 点击分派目标 id（如 "setup" 配置引导），无则纯告知
    Error {
        message: String,
        retention: ErrorRetention,
        action: Option<String>,
    },
    /// 流式增量：LLM 回复片段——纯显示优化，不经 Queue/Context。
    /// 不走 effects Vec（实时性），经 effect_sink 旁路直推。
    AssistantDelta {
        content: Option<String>,
        reasoning_content: Option<String>,
    },
    /// 一轮触发结束（loading 收尾信号，完整回复已写 Context）
    AssistantDone,
}

impl Effect {
    /// 动作流记录的 kind/payload：
    /// **穷尽 match**——新增变体此处编译错（编译期强制进动作流）。
    /// 记录字段 snake_case（storage 约定），与 WS 下发的 camelCase 形态无关。
    pub fn effect_kind_payload(&self) -> (&'static str, Value) {
        match self {
            Effect::RenderComponent(spec) => ("render_component", json!({ "spec": spec })),
            Effect::CloseComponent(id) => ("close_component", json!({ "id": id })),
            Effect::SetAutonomy { face, motion, ttl_ms, once } => (
                "set_autonomy",
                json!({ "face": face, "motion": motion, "ttl_ms": ttl_ms, "once": once }),
            ),
            Effect::ConfigChanged { llm_changed } => {
                ("config_changed", json!({ "llm_changed": llm_changed }))
            }
            Effect::Error { message, retention, action } => (
                "error",
                json!({ "message": message, "retention": retention.as_str(), "action": action }),
            ),
            Effect::AssistantDelta { content, reasoning_content } => (
                "assistant_delta",
                json!({ "content": content, "reasoning_content": reasoning_content }),
            ),
            Effect::AssistantDone => ("assistant_done", json!({})),
        }
    }
}

/// claude 检测（实测 54/54 命中、0 误伤）：✳ 前缀（活动 glyph）或标题 == claude
fn is_claude_title(t: &str) -> bool {
    let t = t.trim_start();
    t.starts_with('✳') || t == "claude"
}

/// 去标题 glyph/空白（marker 解析与占位名共用）
fn strip_glyphs(t: &str) -> String {
    t.trim_start_matches(['✳', ' ']).trim().to_string()
}

pub struct AmberyBackend<L: Llm> {
    pub harness: Harness,
    pub config: Config,
    llm: L,
    /// Filter 按实例 kind 的缓存（Filter 唯一按实例 hook kind 选择——
    /// 无全局策略、无默认回退；缺失或不受支持的 kind 在处理前直接拒绝）
    filter_cache: std::collections::HashMap<String, Arc<dyn Filter + Send + Sync>>,
    /// Timer：每实例兜底扫描调度
    pub timers: TimerWheel,
    /// Terminal Adapter：定位/读取/遗忘
    /// 统一接口；None = 无终端访问（Hook 驱动核心体验仍可用）
    pub terminal: Option<Arc<dyn crate::terminal::TerminalAdapter>>,
    /// Platform Primitives：虚拟桌面切换
    /// 等 OS 层能力；None = 无（fetch_terminal 的 vd_switch=true 路径报切换失败）
    pub primitives: Option<Arc<dyn crate::terminal::PlatformPrimitives>>,
    /// sidecar 在读通道链中时，Timer 读到 None 才判定 tab 消亡（closed）；
    /// 纯 MapAdapter（case-runner）下 None 只是「未注入」，不能当消亡证据
    pub sidecar_enabled: bool,
    /// 流式 delta 旁路：run_trigger 每收到 delta 即发——
    /// 显示优化事件（AssistantDelta/AssistantDone）不进 effects Vec，由 server 层接广播
    pub effect_sink: Option<Arc<dyn Fn(&Effect) + Send + Sync>>,
    /// 冷字段启动快照：待重启 = 保存值与启动快照不同，
    /// 两者重新相同即清除。快照在 backend 启动时取（TimerWheel 等运行行为按启动值构建）
    pub config_cold_snapshot: Vec<(&'static str, Value)>,
    /// edit_config query 快照：只有携带更新目标
    /// 完整当前值的 query（叶子直查 / 容器 view=object）才留快照；快照关联自己的
    /// tool result message ID（tool_call_id）与 response 序号
    query_snapshots: Vec<QuerySnapshot>,
    /// LLM response 序号（tool 循环每轮 +1）：update 要求快照来自更早的 response
    response_seq: u64,
    /// 工具调用预算（冷字段，启动时读取——
    /// 运行中改 config 不影响本进程行为，经待重启状态如实上报）
    tool_budget_response: usize,
    tool_budget_turn: usize,
    /// 变化检测 prev（每实例上次归一全文）：**内存态，重启丢**（filtered_content 不持久化
    /// 定案）——scan/hook/fetch 读后更新
    filtered_prev: std::collections::HashMap<String, String>,
}

/// edit_config query 快照条目
#[derive(Debug, Clone)]
struct QuerySnapshot {
    path: String,
    tool_call_id: String,
    seq: u64,
}

/// path 相交判定（快照覆盖/写入失效共用）：相等或互为前缀
fn paths_intersect(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{b}.")) || b.starts_with(&format!("{a}."))
}

/// 叶子级 diff：返回两值间变化的叶子 path（object 递归；增删 key 记该层 path）
fn diff_paths(prefix: &str, a: &Value, b: &Value) -> Vec<String> {
    match (a, b) {
        (Value::Object(ma), Value::Object(mb)) => {
            let mut out = Vec::new();
            let keys: std::collections::BTreeSet<&String> = ma.keys().chain(mb.keys()).collect();
            for k in keys {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                match (ma.get(k), mb.get(k)) {
                    (Some(x), Some(y)) => out.extend(diff_paths(&p, x, y)),
                    _ => out.push(p),
                }
            }
            out
        }
        _ if a == b => vec![],
        _ => vec![prefix.to_string()],
    }
}

/// grep/query 结果的节点类型显示名
fn node_type_name(ty: &crate::config::reflect::NodeType) -> &'static str {
    use crate::config::reflect::NodeType as T;
    match ty {
        T::Bool => "bool",
        T::Int { .. } => "int",
        T::Float { .. } => "float",
        T::Str => "str",
        T::Enum { .. } => "enum",
        T::Map => "map",
        T::Object => "object",
        T::Other => "other",
    }
}

/// Terminal Adapter 的阻塞 locate/read 往返移到 blocking 线程池。
/// 独立于 AmberyBackend 泛型参数，server timer 可在不长期持锁的情况下调用。
pub async fn read_terminal_via(
    terminal: Option<std::sync::Arc<dyn crate::terminal::TerminalAdapter>>,
    inst: &str,
) -> Option<String> {
    match read_terminal_outcome(terminal, inst, None).await {
        crate::terminal::ReadOutcome::Content(text) => Some(text),
        // agent 面不区分 Gone/Error：统一「读不到」（三态为内部语义）
        _ => None,
    }
}

/// 三态读出口：优先按已定位 tab 读（timer 判死语义），未定位过才现 locate。
/// locate 失败无法区分 marker 未命中与传输失败 → Error（信念不动，永不判死）。
pub async fn read_terminal_outcome(
    terminal: Option<std::sync::Arc<dyn crate::terminal::TerminalAdapter>>,
    inst: &str,
    known_tab: Option<crate::TabRef>,
) -> crate::terminal::ReadOutcome {
    let Some(terminal) = terminal else {
        return crate::terminal::ReadOutcome::Error("no terminal adapter".into());
    };
    let inst = inst.to_string();
    tokio::task::spawn_blocking(move || {
        match known_tab.or_else(|| crate::terminal::join_instance(terminal.as_ref(), &inst)) {
            None => crate::terminal::ReadOutcome::Error("unlocatable".into()),
            Some(tab) => terminal.read(&tab),
        }
    })
    .await
    .unwrap_or_else(|_| crate::terminal::ReadOutcome::Error("read task join failed".into()))
}

/// timer 读判结果（server timer 与 case-runner 共用）
pub enum TimerJudgment {
    /// Content → 走 handle_timer_scan
    Scan(String),
    /// 判死：read 直接 Gone（位置级强证据），或枚举对账确认 marker 缺席
    Close,
    /// 信念不动：Error / 枚举对账失败 / tab 仍在原位（载荷为诊断）
    Skip(String),
    /// 自愈：枚举对账在新位置找到 marker（位置漂移非死亡）→ 回写实例记录
    Relocated(crate::TabRef),
}

/// timer 读判：Content → Scan；Gone → Close；Error → 枚举对账——
/// 全量枚举确认 marker 缺席才 Close（观察非推断）；新位置找到 → Relocated；
/// 枚举本身失败 → 信念不动 Skip。
pub async fn judge_timer_read(
    terminal: Option<std::sync::Arc<dyn crate::terminal::TerminalAdapter>>,
    inst: &str,
    known_tab: Option<crate::TabRef>,
) -> TimerJudgment {
    let Some(terminal) = terminal else {
        return TimerJudgment::Skip("no terminal adapter".into());
    };
    match read_terminal_outcome(Some(terminal.clone()), inst, known_tab).await {
        crate::terminal::ReadOutcome::Content(c) => TimerJudgment::Scan(c),
        crate::terminal::ReadOutcome::Gone => TimerJudgment::Close,
        crate::terminal::ReadOutcome::Error(e) => {
            let inst_owned = inst.to_string();
            let reconcile = tokio::task::spawn_blocking(move || {
                terminal.enumerate().map(|tabs| {
                    tabs.into_iter()
                        .find(|t| {
                            t.title
                                .as_deref()
                                .map(|s| s.contains(&inst_owned))
                                .unwrap_or(false)
                        })
                        .map(|t| t.tab)
                })
            })
            .await
            .ok()
            .flatten();
            match reconcile {
                None => TimerJudgment::Skip(format!("{e}; reconcile enumerate failed")),
                Some(Some(tab)) if Some(tab) != known_tab => TimerJudgment::Relocated(tab),
                Some(Some(_)) => TimerJudgment::Skip(e),
                Some(None) => TimerJudgment::Close,
            }
        }
    }
}

impl<L: Llm> AmberyBackend<L> {
    pub fn new(harness: Harness, config: Config, llm: L) -> Self {
        let timers = TimerWheel::new(config.timer.interval_ms, config.timer.stagger_ms);
        // 工具调用预算（冷字段，启动时捕获）
        let tool_budget_response = config.max_tool_calls_in_one_response;
        let tool_budget_turn = config.max_tool_calls_per_turn;
        let cfg_v = serde_json::to_value(&config).unwrap_or(Value::Null);
        let config_cold_snapshot = crate::config::meta::cold_paths()
            .into_iter()
            .map(|p| {
                (
                    p,
                    crate::config::meta::value_at(&cfg_v, p).cloned().unwrap_or(Value::Null),
                )
            })
            .collect();
        let mut backend = Self {
            harness,
            config,
            llm,
            filter_cache: std::collections::HashMap::new(),
            timers,
            terminal: None,
            primitives: None,
            sidecar_enabled: false,
            effect_sink: None,
            config_cold_snapshot,
            query_snapshots: Vec::new(),
            response_seq: 0,
            tool_budget_response,
            tool_budget_turn,
            filtered_prev: std::collections::HashMap::new(),
        };
        // 启动调度（兜底覆盖）：TimerWheel 不 replay，
        // 对投影中全部存活实例批量 reset——无 hook 实例（僵尸）也进兜底扫描集，
        // 否则它们永不入调度、永不判 closed（#10 reopen：调度盲区）
        let now = crate::server::now_ms();
        let alive: Vec<String> = backend
            .harness
            .agents
            .iter()
            .filter(|a| a.status != AgentStatus::Closed)
            .map(|a| a.name.clone())
            .collect();
        for name in alive {
            backend.timers.reset(&name, now);
        }
        backend
    }

    /// 统一配置修改管道（「修改入口」）：CLI/面板/LLM tool 共用。
    /// set_by_path 写入 → serde 反序列化验证 → 动态 enum 校验 → 热应用 → persist。
    /// restart_required = 运行时 diff 如实上报（不假装生效，行为即真相）。
    pub fn apply_config_by_path(
        &mut self,
        path: &str,
        value: Value,
    ) -> Result<ConfigOutcome, String> {
        if self.config.read_only {
            return Err("只读降级模式：config 写被禁止".into());
        }
        // null 语义：null 只允许写到叶子
        // （null = 缺失 → 回自身 default）；object/map/动态 entry 拒绝 null 更新
        if value.is_null() {
            let is_leaf = crate::config::meta::node_meta(path)
                .map(|m| matches!(m.kind, crate::config::meta::NodeKind::Leaf))
                .unwrap_or(false);
            if !is_leaf {
                return Err(
                    "null 只允许写到叶子（回自身 default）；object/map/动态 entry 拒绝 null 更新"
                        .into(),
                );
            }
        }
        let mut v = serde_json::to_value(&self.config).map_err(|e| e.to_string())?;
        crate::config::reflect::set_by_path(&mut v, path, value.clone())?;
        // null 归一（工具写入与保存遵守同一归一化）——null 叶子移除后
        // 由 serde default 回填（= 回自身 default）
        crate::config::migrate::normalize_nulls(&mut v);
        let new: Config = serde_json::from_value(v).map_err(|e| format!("验证失败: {e}"))?;
        // 统一 validation（目标子树→祖先；任一失败原子拒绝整次更新）
        let new_v = serde_json::to_value(&new).map_err(|e| e.to_string())?;
        let verrs = crate::config::meta::validate_for_update(&new_v, path);
        if !verrs.is_empty() {
            return Err(format!(
                "验证失败: {}",
                verrs
                    .iter()
                    .map(|(p, m)| format!("{p}: {m}"))
                    .collect::<Vec<_>>()
                    .join("；")
            ));
        }
        // 动态 enum 校验（OPTIONS 注册表，验证集中一份）
        if let (Some(opts), Value::String(s)) =
            (crate::config::reflect::valid_options(&new, path), &value)
        {
            if !opts.contains(s) {
                return Err(format!("{path}: '{s}' 不在合法选项 {opts:?} 中"));
            }
        }
        // 先落盘后换内存（顺序不分叉）：persist 失败时内存/快照/滤镜全部不动，
        // 调用方看到的 Err 与真实状态一致（原子拒绝）
        new.save(self.harness.config_dir())
            .map_err(|e| format!("persist 失败: {e}"))?;
        let old = std::mem::replace(&mut self.config, new);
        // 一次成功写入使相交路径的 query 快照失效（统一管道单点——LLM/CLI/面板/外部载入全部经此，无关路径不受影响）
        self.query_snapshots.retain(|r| !paths_intersect(&r.path, path));
        // 其余字段每轮现读/经 effective_* 出口现取，天然热（Filter 按实例 kind 现选现缓存）
        let llm_changed = self.config.llm != old.llm;
        Ok(ConfigOutcome {
            effects: vec![Effect::ConfigChanged { llm_changed }],
            llm_changed,
            restart_required: self.restart_required(),
        })
    }

    /// 待重启状态：冷字段保存值与启动快照不同；
    /// 两者重新相同即清除。如实上报，不假装生效（行为即真相）
    pub fn restart_required(&self) -> Vec<String> {
        let cur = serde_json::to_value(&self.config).unwrap_or(Value::Null);
        self.config_cold_snapshot
            .iter()
            .filter(|(p, snap)| crate::config::meta::value_at(&cur, p) != Some(snap))
            .map(|(p, _)| p.to_string())
            .collect()
    }

    // ── edit_config 三 action（单工具、显式动作、渐进披露）──

    /// 响应体积护栏：聚合结果 UTF-8 JSON ≤ 1 KiB；
    /// 不截断、不自动缩小，错误说明实际大小、上限与收窄方向。single_leaf 例外直返。
    /// hint 传 i18n 表 key（错误反馈按 Harness 语言现查）
    fn guard_1k(&self, out: Value, single_leaf: bool, hint_key: &str) -> Value {
        if single_leaf {
            return out;
        }
        let size = out.to_string().len();
        if size > 1024 {
            let lang = crate::i18n::Lang::of(&self.config.harness_language);
            json!({
                "ok": false,
                "error": crate::i18n::trf(lang, "err.size-limit", &[("size", size.to_string()), ("hint", crate::i18n::tr(lang, hint_key).to_string())])
            })
        } else {
            out
        }
    }

    /// query 快照入册（容量上限：超 64 条丢弃最旧——长会话无界增长保护）
    fn push_snapshot(&mut self, path: &str, call_id: &str) {
        const MAX_SNAPSHOTS: usize = 64;
        if self.query_snapshots.len() >= MAX_SNAPSHOTS {
            self.query_snapshots.drain(..self.query_snapshots.len() - MAX_SNAPSHOTS + 1);
        }
        self.query_snapshots.push(QuerySnapshot {
            path: path.to_string(),
            tool_call_id: call_id.to_string(),
            seq: self.response_seq,
        });
    }

    /// grep：Rust regex 搜 LLM 可见节点的 path 与中文 desc；返回 path+type+desc
    /// （不返回 value），按 path 字典序；合法 regex 无匹配 = 成功空数组
    fn edit_config_grep(&self, args: &Value) -> (Value, Vec<Effect>) {
        let lang = crate::i18n::Lang::of(&self.config.harness_language);
        let Some(pattern) = args.get("pattern").and_then(Value::as_str) else {
            return (json!({ "ok": false, "error": crate::i18n::tr(lang, "err.grep-pattern") }), vec![]);
        };
        let re = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return (json!({ "ok": false, "error": crate::i18n::trf(lang, "err.regex", &[("e", e.to_string())]) }), vec![]),
        };
        let nodes = crate::config::reflect::config_nodes_llm(&self.config);
        let mut hits: Vec<_> = nodes
            .iter()
            .filter(|n| re.is_match(&n.path) || n.desc.as_deref().is_some_and(|d| re.is_match(d)))
            .collect();
        // 按完整 path 字典序稳定排列
        hits.sort_by(|a, b| a.path.cmp(&b.path));
        let matches: Vec<Value> = hits
            .iter()
            .map(|n| {
                json!({
                    "path": n.path,
                    "type": node_type_name(&n.ty),
                    "desc": n.desc,
                })
            })
            .collect();
        let out = self.guard_1k(
            json!({ "ok": true, "matches": matches }),
            false,
            "hint.grep-narrow",
        );
        (out, vec![])
    }

    /// query：精确 path 查一个节点，统一 node + children；叶子直查与容器
    /// view=object 携带完整当前值 → 留快照（关联 tool_call_id 与 response 序号）
    fn edit_config_query(&mut self, args: &Value, call_id: &str) -> (Value, Vec<Effect>) {
        let lang = crate::i18n::Lang::of(&self.config.harness_language);
        let Some(path) = args.get("path").and_then(Value::as_str) else {
            return (json!({ "ok": false, "error": crate::i18n::tr(lang, "err.query-path") }), vec![]);
        };
        if !crate::config::meta::llm_visible(path) {
            return (
                json!({ "ok": false, "error": crate::i18n::trf(lang, "err.no-access", &[("path", path.to_string())]) }),
                vec![],
            );
        }
        let nodes = crate::config::reflect::config_nodes_llm(&self.config);
        let Some(node) = nodes.iter().find(|n| n.path == path) else {
            return (
                json!({ "ok": false, "error": crate::i18n::trf(lang, "err.unknown-path", &[("path", path.to_string())]) }),
                vec![],
            );
        };
        let is_container = matches!(
            node.ty,
            crate::config::reflect::NodeType::Object | crate::config::reflect::NodeType::Map
        );
        let view = args.get("view").and_then(Value::as_str);
        enum QView {
            Children,
            Object,
        }
        let qview = match view {
            None | Some("children") => QView::Children,
            Some("object") => QView::Object,
            Some(other) => {
                return (
                    json!({ "ok": false, "error": crate::i18n::trf(lang, "err.bad-view", &[("other", other.to_string())]) }),
                    vec![],
                );
            }
        };
        // 待重启 msg（query 到待重启变更的具体值时如实说明）
        let pending_msg = |p: &str| {
            if self.restart_required().iter().any(|r| paths_intersect(r, p)) {
                Some(crate::i18n::tr(lang, "msg.saved-restart"))
            } else {
                None
            }
        };
        let msg = pending_msg(path);
        let mut with_msg = |mut out: Value| {
            if let Some(m) = msg {
                out["msg"] = json!(m);
            }
            out
        };
        match (is_container, qview) {
            (false, QView::Object) => (
                json!({ "ok": false, "error": crate::i18n::tr(lang, "err.object-view-leaf") }),
                vec![],
            ),
            (false, QView::Children) => {
                // 叶子：node 带 value，children []；完整值 → 留快照
                self.push_snapshot(path, call_id);
                let out = json!({
                    "ok": true,
                    "node": { "path": path, "type": node_type_name(&node.ty), "desc": node.desc, "value": node.value },
                    "children": [],
                });
                // 单叶子精确查询是体积护栏例外：完整直返不截断
                (with_msg(out), vec![])
            }
            (true, QView::Children) => {
                // 容器 children 视图：node + 直接 children（叶子 child 带 value，
                // 容器 child 不递归携带 value）；导航视图，不产生快照
                let prefix = format!("{path}.");
                let children: Vec<Value> = nodes
                    .iter()
                    .filter(|n| {
                        n.path.starts_with(&prefix) && !n.path[prefix.len()..].contains('.')
                    })
                    .map(|n| {
                        let mut c = json!({
                            "path": n.path,
                            "type": node_type_name(&n.ty),
                            "desc": n.desc,
                        });
                        if !matches!(
                            n.ty,
                            crate::config::reflect::NodeType::Object
                                | crate::config::reflect::NodeType::Map
                        ) {
                            c["value"] = n.value.clone();
                        }
                        c
                    })
                    .collect();
                let out = self.guard_1k(
                    json!({
                        "ok": true,
                        "node": { "path": path, "type": node_type_name(&node.ty), "desc": node.desc },
                        "children": children,
                    }),
                    false,
                    "hint.children",
                );
                (with_msg(out), vec![])
            }
            (true, QView::Object) => {
                // 容器 object 视图：完整当前 JSON，不返回 children
                let out = self.guard_1k(
                    json!({
                        "ok": true,
                        "node": { "path": path, "type": node_type_name(&node.ty), "desc": node.desc, "value": node.value },
                    }),
                    false,
                    "hint.object",
                );
                // 快照只在完整值真正交付时入册（guard 拒了 = LLM 没拿到当前值，
                // 不能让 update 门禁凭错误 result 放行）
                if out["ok"] == json!(true) {
                    self.push_snapshot(path, call_id);
                }
                (with_msg(out), vec![])
            }
        }
    }

    /// update：需更早 response 中仍有效的完整 query 快照；走统一修改管道
    fn edit_config_update(&mut self, args: &Value) -> (Value, Vec<Effect>) {
        let lang = crate::i18n::Lang::of(&self.config.harness_language);
        let Some(path) = args.get("path").and_then(Value::as_str) else {
            return (json!({ "ok": false, "error": crate::i18n::tr(lang, "err.update-path") }), vec![]);
        };
        if !crate::config::meta::llm_visible(path) {
            return (
                json!({ "ok": false, "error": crate::i18n::trf(lang, "err.no-access", &[("path", path.to_string())]) }),
                vec![],
            );
        }
        let Some(value) = args.get("value").cloned() else {
            return (json!({ "ok": false, "error": crate::i18n::tr(lang, "err.update-value") }), vec![]);
        };
        // path 存在性：静态注册节点，或可见 map 的已有 entry（新建 entry = 不存在 →
        // 走完整 map update 协议：不提供 add/delete action）
        let exists = crate::config::meta::node_meta(path).is_some()
            || crate::config::reflect::config_nodes_llm(&self.config)
                .iter()
                .any(|n| n.path == path);
        if !exists {
            return (
                json!({ "ok": false, "error": crate::i18n::trf(lang, "err.unknown-path-map", &[("path", path.to_string())]) }),
                vec![],
            );
        }
        // 快照有效性：存在 r 满足
        // r.path = P ∧ r.toolResultMessageId ∈ C ∧ r 来自更早 response ∧ 其后无相交成功写入
        let ctx = self.harness.context.messages();
        let valid = self.query_snapshots.iter().any(|r| {
            r.path == path
                && r.seq < self.response_seq
                && ctx
                    .iter()
                    .any(|m| m.tool_call_id.as_deref() == Some(r.tool_call_id.as_str()))
        });
        if !valid {
            return (
                json!({ "ok": false, "error": crate::i18n::tr(lang, "err.no-snapshot") }),
                vec![],
            );
        }
        match self.apply_config_by_path(path, value) {
            Ok(outcome) => {
                // 动作流记录：LLM edit_config 路径 =
                // config_changed/backend；前端 set_config 路径在端点记 config_update/frontend
                let llm_changed = outcome.llm_changed;
                let _ = self.harness.log_effect(
                    crate::EffectOrigin::Backend,
                    "config_changed",
                    json!({ "llm_changed": llm_changed }),
                    crate::server::now_ms(),
                );
                let restart = outcome.restart_required.clone();
                let mut r = json!({ "ok": true, "path": path });
                if !restart.is_empty() {
                    r["restartRequired"] = json!(restart);
                    r["msg"] = json!(crate::i18n::tr(lang, "msg.saved-restart"));
                } else {
                    r["msg"] = json!(crate::i18n::tr(lang, "msg.saved-hot"));
                }
                (r, outcome.effects)
            }
            Err(e) => (json!({ "ok": false, "error": e }), vec![]),
        }
    }

    /// llm_changed 后由 server 重建具体 LlmBackend 注入（ambery 泛型擦除不认识它）
    pub fn replace_llm(&mut self, llm: L) {
        self.llm = llm;
    }

    /// 外部自动载入的应用：与一次全文 update
    /// 相同的热应用——替换 live Config（read_only 运行时降级标记保留，不被文件覆盖）、
    /// filter 按策略重建；冷字段 pending 由 restart_required() 按启动快照发散判定；
    /// 与实际变更路径相交的 agent 已读快照标记 dirty。
    /// 返回 llm_changed（true 时调用方重建 LlmBackend 注入）。
    pub fn apply_external_config(&mut self, new_cfg: Config) -> bool {
        let old_v = serde_json::to_value(&self.config).unwrap_or(Value::Null);
        let new_v = serde_json::to_value(&new_cfg).unwrap_or(Value::Null);
        let changed = diff_paths("", &old_v, &new_v);
        let old_llm = self.config.llm.clone();
        let read_only = self.config.read_only;
        self.config = new_cfg;
        self.config.read_only = read_only;
        self.query_snapshots
            .retain(|r| !changed.iter().any(|p| paths_intersect(&r.path, p)));
        self.config.llm != old_llm
    }

    /// 现拼 system prompt 请求头（Config 引用的各概念数据运行时拼装）
    /// = base_prompt（Config）+ AGENTS.md（Storage，热生效）+ kaomoji 表。
    /// 内容稳定、天然 cache 友好，不落 Queue。
    fn assemble_system_prompt(&self) -> String {
        // {name} 占位替换为当前 pet 名称（Harness 身份文案读取当前名称）
        let mut s = self
            .config
            .base_prompt
            .replace("{name}", &self.config.name);
        s.push_str("\n\n");
        s.push_str(&self.read_agents_md());
        // kaomoji 表为什么不放进 AGENTS.md：
        // ① 表体是运行时数据（edit_config 可改 kaomoji 映射），须每轮现拼保持最新，
        //    写死在 AGENTS.md 会变成两个真相源；
        // ② 段头用途说明与组装表共位（贴着表解释「这是什么」），且作为不变量护栏——
        //    AGENTS.md 是用户可编辑文件，说明写在那里可能被无意删改。
        //    AGENTS.md 行为准则里已有禁令散文，此处是贴着表的强化，故意重复。
        s.push_str("\n\n");
        s.push_str(crate::i18n::tr(crate::i18n::Lang::of(&self.config.harness_language), "prompt.kaomoji-header"));
        s.push('\n');
        // 请求头只带系统池（用户表情池按需经 edit_config 查询，不自动注入）
        let mut keys: Vec<_> = self.config.kaomoji.system.keys().collect();
        keys.sort();
        for k in keys {
            let v = &self.config.kaomoji.system[k];
            s.push_str(&format!("- {k}: {} ({})\n", v.face, v.motion));
        }
        s
    }

    /// AGENTS.md 每轮现读（热生效：改完下一个触发就用）；读不到回退内置默认
    /// （fallback 按当前 Harness 语言）；{name} 占位替换为当前 pet 名称
    fn read_agents_md(&self) -> String {
        std::fs::read_to_string(self.harness.config_dir().join(AGENTS_MD_FILE))
            .unwrap_or_else(|_| default_agents_md(crate::i18n::Lang::of(&self.config.harness_language)))
            .replace("{name}", &self.config.name)
    }

    /// 存活实例数（投影口径：status ≠ Closed）——生命周期簿记的 post-count（#16 ①）
    fn alive_count(&self) -> usize {
        self.harness
            .agents
            .iter()
            .filter(|a| a.status != AgentStatus::Closed)
            .count()
    }

    /// 经 Terminal Adapter 读实例终端：
    /// locate → read 配对（同 adapter 内完成）；无 adapter / 定不到 / 读不到 = None。
    /// 同步 UIA 往返放 spawn_blocking——sidecar 的线程 sleep/进程 IO 不阻塞 tokio worker。
    /// （pub：server timer 任务与 case-runner timer_scan step 共用此入口）
    pub async fn read_terminal(&self, inst: &str) -> Option<String> {
        read_terminal_via(self.terminal.clone(), inst).await
    }

    /// 读原文 → 存档 → Filter → note（读通道一条龙；fetch_terminal 与 stop auto_read 共用）
    async fn read_terminal_filtered(
        &mut self,
        inst: &str,
        source: RecordSource,
        ts: i64,
    ) -> Option<String> {
        let raw = self.read_terminal(inst).await?;
        let _ = self.harness.append_terminal_content(TerminalContentRecord {
            instance: inst.into(),
            raw: raw.clone(),
            source,
            ts,
        });
        let filtered = self.filter_for(inst).map(|f| f.digest(&raw).render());
        if let Some(f2) = &filtered {
            self.note_filtered(inst, f2.clone());
        }
        filtered
    }

    /// 状态 key 推导（key 切换由后端根据 Hook/Timer 驱动）：
    /// notify（有未决通知）> processing（任一实例在跑）> idle。
    /// 返回 `[face: key, motion: key]`——写默认推导 key；覆盖状态 LLM 从自己的
    /// tool_calls 历史已知。
    fn state_key(&self, pending_notifications: usize) -> String {
        let key = if pending_notifications > 0 {
            "notify"
        } else if self
            .harness
            .agents
            .iter()
            .any(|a| a.status == AgentStatus::Processing)
        {
            "processing"
        } else {
            "idle"
        };
        let motion = self
            .config
            .kaomoji_resolve(key)
            .map(|k| k.motion.as_str())
            .unwrap_or("still");
        format!("[face: {key}, motion: {motion}]")
    }

    /// 入队一条输入（hook 内容 = system，user 消息 = user）。
    /// 生产者只入队不触发——放行由 drain_queue / server 消费者任务驱动。
    /// source = 输入来源（effort 档位与双队列的一等公民）
    pub fn enqueue(
        &mut self,
        role: Role,
        content: String,
        source: crate::queue::QueueSource,
        ts: i64,
    ) -> std::io::Result<()> {
        // 动作流记录：user 消息入队 = user_message/frontend
        // （单点覆盖 post_user / append_user / case user step；hook 的 system 输入不进）
        if role == Role::User {
            let _ = self.harness.log_effect(
                crate::EffectOrigin::Frontend,
                "user_message",
                json!({ "text": content }),
                crate::server::now_ms(),
            );
        }
        self.harness
            .enqueue_input(crate::queue::QueueInput { role, content, source, ts })
    }

    /// 前端非 readonly 调用上报单点：
    /// record_effect command 与 POST /effect 共用——写 effect.jsonl（origin=frontend）。
    /// fire-and-forget：记录失败不影响调用方。
    pub fn record_frontend_effect(&self, kind: &str, payload: Value) {
        let _ = self.harness.log_effect(
            crate::EffectOrigin::Frontend,
            kind,
            payload,
            crate::server::now_ms(),
        );
    }

    /// 放行一条输入：Context 写输入 → run_trigger（一轮完整处理）
    /// Event Buffer 附带：放行时 merge 清空——system 输入与之合并为
    /// 一条消息；user 输入则 buffer 以独立 system 消息先行附带（与 user role 严格分离）。
    pub async fn release_one(
        &mut self,
        input: crate::queue::QueueInput,
        pending_notifications: usize,
    ) -> std::io::Result<Vec<Effect>> {
        let ts = input.ts;
        let merged = self.harness.event_buffer.merge_and_clear();
        match (input.role, merged) {
            (Role::System, Some(buf)) => {
                // 附带合并为一条 system 消息
                self.harness.append_context(ContextMessage::new(
                    Role::System,
                    format!("{}\n\n{}", input.content, buf),
                    ts,
                ))?;
            }
            (role, maybe_buf) => {
                if let Some(buf) = maybe_buf {
                    self.harness
                        .append_context(ContextMessage::new(Role::System, buf, ts))?;
                }
                self.harness
                    .append_context(ContextMessage::new(role, input.content, ts))?;
            }
        }
                self.run_trigger(ts, input.source, pending_notifications).await
    }

    /// 放行循环：有输入就一轮一条处理完（server 消费者任务与测试共用）
    pub async fn drain_queue(&mut self, pending_notifications: usize) -> std::io::Result<Vec<Effect>> {
        let mut effects = vec![];
        while let Some(input) = self.harness.queue.release() {
            effects.append(&mut self.release_one(input, pending_notifications).await?);
        }
        Ok(effects)
    }

    /// Compression 检查+执行（#16 真值触发）：轮次开头与 tool 循环内
    /// 共用。判定式 = 最近 usage 真值 + 其后新增消息 est 增量 vs
    /// window − reserve；无真值 → 全量 est；无窗口事实 → 不压缩。
    /// turn_start = 当前 turn 输入消息下标——压缩不切断在飞 turn（min_tail_start 收口）。
    async fn maybe_compress(&mut self, turn_start: usize, ts: i64) -> std::io::Result<()> {
        let trigger_tokens = match self.harness.last_usage {
            Some(u) => {
                u.prompt_tokens as usize
                    + self
                        .harness
                        .context
                        .est_tokens_since(self.harness.last_usage_msg_len)
            }
            None => self.harness.context.total_tokens(),
        };
        let compress = self
            .config
            .effective_compression_limit()
            .is_some_and(|limit| trigger_tokens > limit);
        if !compress {
            return Ok(());
        }
        let pre_tokens = trigger_tokens; // 同尺：触发瞬间的真值锚点+增量（#16 ④）
        let t0 = std::time::Instant::now();
        // summarize 返回（摘要, usage 真值）；摘要调用也留真值（#16）。
        // 摘要失败不炸轮：跳过本次压缩（下轮再评估），transient 错误通知即时下发
        let (summary, summary_usage) = match self.llm.summarize(self.harness.context.messages()).await {
            Ok(s) => s,
            Err(err) => {
                self.emit_error(format!("LLM 压缩摘要失败：{err}"), ErrorRetention::Transient, None);
                return Ok(());
            }
        };
        if let Some(u) = summary_usage {
            self.harness.log_usage(u, ts)?;
        }
        let keep = self.config.context_compression_keep_recent_messages;
        let lang = crate::i18n::Lang::of(&self.config.harness_language);
        self.harness.context.compress(summary.clone(), keep, turn_start, lang, ts);
        let post_tokens = self.harness.context.total_tokens(); // 同尺：压缩后 est（真值下轮刷新）
        self.harness.log_compact_boundary(
            summary,
            pre_tokens,
            post_tokens,
            t0.elapsed().as_millis() as u64,
            ts,
        )?;
        // 归零：diff 基准清空 + 全景一条重建 LLM 认知
        self.filtered_prev.clear();
        if let Some(p) = crate::panorama(&self.harness.agents) {
            self.harness
                .append_context(ContextMessage::new(Role::System, p, ts))?;
        }
        Ok(())
    }

    /// effort 档位解析：user_chat→low、
    /// hook_stop_content→high、其余→medium（Config effort.* 可覆盖）；user_chat 消息
    /// 命中关键词则本次临时改写（多命中取最长关键词，确定性）
    fn resolve_effort(&self, source: crate::queue::QueueSource) -> Option<crate::llm::Effort> {
        use crate::queue::QueueSource as S;
        if source == S::UserChat {
            let user_text = self.harness.context.messages().last().and_then(|m| {
                if m.role == Role::User {
                    m.content.as_deref()
                } else {
                    None
                }
            });
            if let Some(text) = user_text {
                let mut best: Option<(usize, crate::llm::Effort)> = None;
                for (kw, e) in &self.config.effort.keywords {
                    if !kw.is_empty() && text.contains(kw.as_str()) {
                        let len = kw.chars().count();
                        if best.is_none_or(|(l, _)| len > l) {
                            best = Some((len, *e));
                        }
                    }
                }
                if let Some((_, e)) = best {
                    return Some(e);
                }
            }
        }
        match source {
            S::UserChat => self.config.effort.user_chat.or(Some(crate::llm::Effort::Low)),
            S::HookStopContent => self.config.effort.hook_stop_content.or(Some(crate::llm::Effort::High)),
            _ => self.config.effort.default.or(Some(crate::llm::Effort::Medium)),
        }
    }

    /// 错误通知单点：动作流落盘 + effect_sink 即时下发
    /// （与 delta 同旁路——不等轮末 effects Vec，轮次中段错误即时可见）
    fn emit_error(&self, message: String, retention: ErrorRetention, action: Option<String>) {
        let e = Effect::Error { message, retention, action };
        let (kind, payload) = e.effect_kind_payload();
        let _ = self.harness.log_effect(
            crate::EffectOrigin::Backend,
            kind,
            payload,
            crate::server::now_ms(),
        );
        if let Some(sink) = &self.effect_sink {
            sink(&e);
        }
    }

    /// 一轮触发
    /// 调用前输入已写 Context、Event Buffer 已在放行点附带（release_one）。
    /// pending_notifications：未决通知数（server 层计数传入，推导 notify key 用）
    /// source：放行输入的来源（effort 档位解析输入，
    /// ——工具循环内后续调用沿用）
    pub async fn run_trigger(
        &mut self,
        ts: i64,
        source: crate::queue::QueueSource,
        pending_notifications: usize,
    ) -> std::io::Result<Vec<Effect>> {
        // effort 档位：本次触发解析一次，工具循环内沿用
        let effort = self.resolve_effort(source);
        // 1. 现拼 system prompt 请求头（不落 Context）；变化才写 head 快照
        let head = self.assemble_system_prompt();
        if self.harness.last_head.as_deref() != Some(head.as_str()) {
            self.harness.log_head(head.clone(), ts)?;
        }
        // 2. Autonomy 状态：每轮一条写 context.jsonl，最新一条挂请求末端
        let autonomy = self.state_key(pending_notifications);
        self.harness.log_autonomy(autonomy.clone(), ts)?;
        // 3. Compression（auto-compact / #16 真值触发）：轮次开头检查
        //    （tool 循环内 tool result 追加后再查）
        let turn_start = self.harness.context.messages().len().saturating_sub(1);
        self.maybe_compress(turn_start, ts).await?;
        // 4. tool 循环（请求 = 请求头 + Context 全部消息 + Autonomy 末端）
        //    流式：complete_streaming 边收边经 effect_sink 发 AssistantDelta
        //    预算：call 按声明顺序串行执行；
        //    已提出 calls（含未执行者）都计入 turn 预算；超出任一预算的 call 不执行，
        //    但仍写入对应的失败 tool result
        // 工具说明按 Harness 语言现查表（切换从下一次 LLM 交互起生效）
        let lang = crate::i18n::Lang::of(&self.config.harness_language);
        let tools = tool_set(lang);
        let mut effects = vec![];
        let mut turn_proposed = 0usize; // 本 turn 已提出 calls（执行 + 未执行）
        loop {
            self.response_seq += 1; // 每个 LLM response 一号（edit_config 快照的新旧判定）
            let mut request = Vec::with_capacity(self.harness.context.messages().len() + 2);
            request.push(ContextMessage::new(Role::System, head.clone(), ts));
            request.extend_from_slice(self.harness.context.messages());
            request.push(ContextMessage::new(Role::System, autonomy.clone(), ts));
            let sink = self.effect_sink.clone();
            let harness = &self.harness;
            let on_delta = move |d: &crate::llm::Delta| {
                let e = Effect::AssistantDelta {
                    content: d.content.clone(),
                    reasoning_content: d.reasoning_content.clone(),
                };
                // 动作流记录（delta 全量记录）
                let (kind, payload) = e.effect_kind_payload();
                let _ = harness.log_effect(
                    crate::EffectOrigin::Backend,
                    kind,
                    payload,
                    crate::server::now_ms(),
                );
                if let Some(sink) = &sink {
                    sink(&e);
                }
            };
            let out = match self
                .llm
                .complete_streaming(&request, &tools, effort, &on_delta)
                .await
            {
                Ok(out) => out,
                Err(err) => {
                    // LLM 调用失败 = transient 错误通知；本轮到此为止，
                    // 统一走结尾 AssistantDone 收尾（loading 不悬挂）
                    self.emit_error(format!("LLM 调用失败：{err}"), ErrorRetention::Transient, None);
                    break;
                }
            };
            // usage 真值留痕（#16：每轮一条，覆盖刷新 last_usage）
            if let Some(u) = out.usage {
                self.harness.log_usage(u, ts)?;
            }
            if out.tool_calls.is_empty() {
                // 沉默语义：空 content 不追加
                if let Some(content) = out.content.filter(|c| !c.is_empty()) {
                    let mut msg = ContextMessage::new(Role::Assistant, content, ts);
                    // thinking 全保真留痕（记录≠回放：build_body 仅 tool_calls
                    // 消息带 reasoning_content 进请求，纯文本回复不花 token）
                    msg.reasoning_content = out.reasoning_content.clone();
                    self.harness.append_context(msg)?;
                }
                break;
            }
            let mut assistant_msg = ContextMessage::assistant_tool_calls(out.tool_calls.clone(), ts);
            // thinking 模型：存思维链，回放时必须带回
            assistant_msg.reasoning_content = out.reasoning_content.clone();
            self.harness.append_context(assistant_msg)?;
            let mut executed_in_response = 0usize;
            for call in &out.tool_calls {
                turn_proposed += 1; // 提出即计入 turn 预算
                let over_response = executed_in_response >= self.tool_budget_response;
                let over_turn = turn_proposed > self.tool_budget_turn;
                if over_response || over_turn {
                    // 超预算的 call 不执行，但仍写入对应的失败 tool result
                    let reason = if over_turn {
                        crate::i18n::trf(lang, "err.tool-budget-turn", &[("n", self.tool_budget_turn.to_string())])
                    } else {
                        crate::i18n::trf(lang, "err.tool-budget-response", &[("n", self.tool_budget_response.to_string())])
                    };
                    self.harness.append_context(ContextMessage::tool_result(
                        &call.id,
                        json!({ "ok": false, "error": reason }).to_string(),
                        ts,
                    ))?;
                    continue;
                }
                let (result, mut eff) = self.execute_tool(call).await;
                executed_in_response += 1;
                effects.append(&mut eff);
                self.harness
                    .append_context(ContextMessage::tool_result(&call.id, result.to_string(), ts))?;
                // tool result 追加后再做 Compression 检查；
                // 在飞 turn 由 turn_start 保护不被切断
                self.maybe_compress(turn_start, ts).await?;
            }
            if turn_proposed >= self.tool_budget_turn {
                // 预算耗尽收尾：以空 tools 正常请求一次最终文字
                // 回复（不能再发起 tool call）；不追加特殊 system 记录，不开启新 turn
                self.response_seq += 1;
                let mut request = Vec::with_capacity(self.harness.context.messages().len() + 2);
                request.push(ContextMessage::new(Role::System, head.clone(), ts));
                request.extend_from_slice(self.harness.context.messages());
                request.push(ContextMessage::new(Role::System, autonomy.clone(), ts));
                let sink = self.effect_sink.clone();
                let harness = &self.harness;
                let on_delta = move |d: &crate::llm::Delta| {
                    let e = Effect::AssistantDelta {
                        content: d.content.clone(),
                        reasoning_content: d.reasoning_content.clone(),
                    };
                    // 预算收尾的 delta 同样入流（流式与
                    // 非流式收尾同记；与主路径同一记录点形态）
                    let (kind, payload) = e.effect_kind_payload();
                    let _ = harness.log_effect(
                        crate::EffectOrigin::Backend,
                        kind,
                        payload,
                        crate::server::now_ms(),
                    );
                    if let Some(sink) = &sink {
                        sink(&e);
                    }
                };
                let out = match self
                    .llm
                    .complete_streaming(&request, &[], effort, &on_delta)
                    .await
                {
                    Ok(out) => out,
                    Err(err) => {
                        // 收尾调用失败同主路径：transient 错误通知 + 本轮收尾
                        self.emit_error(format!("LLM 调用失败：{err}"), ErrorRetention::Transient, None);
                        break;
                    }
                };
                if let Some(u) = out.usage {
                    self.harness.log_usage(u, ts)?;
                }
                if let Some(content) = out.content.filter(|c| !c.is_empty()) {
                    let mut msg = ContextMessage::new(Role::Assistant, content, ts);
                    msg.reasoning_content = out.reasoning_content.clone();
                    self.harness.append_context(msg)?;
                }
                break;
            }
        }
        // 一轮完毕：loading 收尾（完整回复已写 Context）
        // 动作流记录（done 也入流）
        let _ = self.harness.log_effect(
            crate::EffectOrigin::Backend,
            "assistant_done",
            json!({}),
            crate::server::now_ms(),
        );
        if let Some(sink) = &self.effect_sink {
            sink(&Effect::AssistantDone);
        }
        Ok(effects)
    }

    /// 启动扫描：全 VD 枚举 → claude 检测 →
    /// marker 解注册 / 无 marker 占位入册（uia:<标题>）→ N/M/K 三方对账进 EventBuffer。
    /// call = sidecar 请求转发（参数化便于测试注入）
    pub async fn startup_sweep(
        &mut self,
        call: &(dyn Fn(&Value) -> Option<Value> + Send + Sync),
        ts: i64,
    ) -> std::io::Result<()> {
        let Some(resp) = call(&json!({ "cmd": "list_windows" })) else {
            return Ok(());
        };
        let (mut located, mut marked, mut placeholder, mut cloaked_n) = (0usize, 0usize, 0usize, 0usize);
        let mut seen_titles: Vec<String> = Vec::new();
        for w in resp["windows"].as_array().cloned().unwrap_or_default() {
            let title = w["title"].as_str().unwrap_or("").to_string();
            let cloaked = w["cloaked"].as_bool().unwrap_or(false);
            if cloaked {
                cloaked_n += 1;
            }
            if !is_claude_title(&title) {
                continue;
            }
            if cloaked {
                // cloaked 窗口只有窗口级标题（= 活动 tab 标题），登记无 tab 定位
                seen_titles.push(title.clone());
                self.sweep_register(&title, None, ts, &mut marked, &mut placeholder)?;
                located += 1;
                continue;
            }
            let hwnd = w["hwnd"].as_i64().unwrap_or(0);
            let Some(tabs) = call(&json!({ "cmd": "list_tabs", "hwnd": hwnd })) else {
                continue;
            };
            for t in tabs["tabs"].as_array().cloned().unwrap_or_default() {
                let name = t["name"].as_str().unwrap_or("").to_string();
                if !is_claude_title(&name) {
                    continue;
                }
                seen_titles.push(name.clone());
                let tab_ref = Some(crate::TabRef {
                    hwnd,
                    index: t["index"].as_i64().unwrap_or(0),
                });
                self.sweep_register(&name, tab_ref, ts, &mut marked, &mut placeholder)?;
                located += 1;
            }
        }
        // 占位尸体清理：uia: 占位条目的标题已不在可见集 → closed（append 日志）
        let ghosts: Vec<AgentEntry> = self
            .harness
            .agents
            .iter()
            .filter(|a| {
                a.hash.starts_with("uia:")
                    && a.status != AgentStatus::Closed
                    && !seen_titles.iter().any(|t| strip_glyphs(t) == a.name)
            })
            .cloned()
            .collect();
        for g in ghosts {
            self.harness.upsert_agent(AgentEntry {
                status: AgentStatus::Closed,
                last_seen: ts,
                ..g
            })?;
        }
        // N/M/K 三方对账（N 为启发式参考值，K 是硬信号）
        let n = call(&json!({ "cmd": "count_processes", "name": "claude" }))
            .and_then(|r| r["count"].as_i64())
            .unwrap_or(0);
        let lang = crate::i18n::Lang::of(&self.config.harness_language);
        let mut line = crate::i18n::trf(
            lang,
            "hook.sweep-line",
            &[
                ("located", located.to_string()),
                ("marked", marked.to_string()),
                ("placeholder", placeholder.to_string()),
                ("n", n.to_string()),
                ("cloaked_n", cloaked_n.to_string()),
            ],
        );
        if cloaked_n > 0 {
            line.push_str(crate::i18n::tr(lang, "hook.sweep-cloaked"));
        }
        self.harness.event_buffer.push(line);
        Ok(())
    }

    /// claude 检测（实测 54/54 命中、0 误伤）：✳ 前缀（活动 glyph）或标题 == claude
    fn sweep_register(
        &mut self,
        title: &str,
        tab: Option<crate::TabRef>,
        ts: i64,
        marked: &mut usize,
        placeholder: &mut usize,
    ) -> std::io::Result<()> {
        let clean = strip_glyphs(title);
        // marker 解析：<project>·<sid8>（sid8 = 末尾 8 位）
        if let Some((project, sid8)) = clean.rsplit_once('·') {
            if sid8.chars().count() == 8 && !project.is_empty() {
                *marked += 1;
                let hash = sid8.to_string();
                let prev = self.harness.agents.iter().rev().find(|a| a.hash == hash);
                return self.harness.upsert_agent(AgentEntry {
                    hash: hash.clone(),
                    name: format!("{project}·{sid8}"),
                    project: project.into(),
                    kind: Some("claude".into()),
                    status: AgentStatus::Idle,
                    tab: tab.or_else(|| prev.and_then(|a| a.tab)),
                    first_seen: prev.map(|a| a.first_seen).unwrap_or(ts),
                    last_seen: ts,
                });
            }
        }
        *placeholder += 1;
        let hash = format!("uia:{clean}");
        let prev = self.harness.agents.iter().rev().find(|a| a.hash == hash);
        self.harness.upsert_agent(AgentEntry {
            hash,
            name: clean.clone(),
            project: "unknown".into(),
            kind: Some("claude".into()),
            status: AgentStatus::Idle,
            tab,
            first_seen: prev.map(|a| a.first_seen).unwrap_or(ts),
            last_seen: ts,
        })
    }

    /// 真实 hook：session_id 身份 + register-on-first-sight + 事件分层。
    /// mock hook（handle_hook）保留为 debug 手段，两条路径并存。
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_real_hook(
        &mut self,
        event: &str,
        session_id: &str,
        cwd: &str,
        kind: Option<&str>,
        prompt: Option<&str>,
        message: Option<&str>,
        last_assistant_message: Option<&str>,
        ts: i64,
    ) -> std::io::Result<()> {
        let project = std::path::Path::new(cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let hash = crate::sid8(session_id);
        let name = crate::instance_name(project, &hash);
        // 事件文字按 Harness 语言现写（事件发生时刻的语言生效，此后成历史不改写）
        let lang = crate::i18n::Lang::of(&self.config.harness_language);
        // register-on-first-sight：未知 session_id 先落注册（first_seen = 后端初见时刻），
        // 已有条目沿用 first_seen / tab / kind（快照字段不被事件覆盖）
        let prev = self.harness.agents.iter().rev().find(|a| a.hash == hash);
        let first_seen = prev.map(|a| a.first_seen).unwrap_or(ts);
        let tab = prev.and_then(|a| a.tab);
        let kind = kind
            .map(String::from)
            .or_else(|| prev.and_then(|a| a.kind.clone()));
        // Filter 按实例 kind：缺失或不受支持的 kind 在实例状态更新、
        // 读、Filter 与 Queue 之前直接拒绝（事件整体不处理）
        if !Self::kind_supported(kind.as_deref()) {
            eprintln!("[hook] {name} kind 缺失或不受支持（{kind:?}），事件拒绝处理");
            return Ok(());
        }
        // Hook 到达 → Timer 重排
        self.timers.reset(&name, ts);
        let mut upsert = |status: AgentStatus, tab: Option<crate::TabRef>| {
            self.harness.upsert_agent(AgentEntry {
                hash: hash.clone(),
                name: name.clone(),
                project: project.into(),
                kind: kind.clone(),
                status,
                tab,
                first_seen,
                last_seen: ts,
            })
        };
        match event {
            // 静默簿记（EventBuffer，pet 不醒）；post-count 标注（#16：LLM 免对账）
            "session_start" => {
                // 定位探测：无 tab 快照时按 marker 现找并回写
                let located = tab.or_else(|| {
                    self.terminal
                        .as_ref()
                        .and_then(|t| crate::terminal::join_instance(t.as_ref(), &name))
                });
                upsert(AgentStatus::Idle, located)?;
                let alive = self.alive_count().to_string();
                self.harness
                    .event_buffer
                    .push(crate::i18n::trf(lang, "hook.register", &[("name", name.clone()), ("alive", alive)]));
            }
            "session_end" => {
                // closed 快照 tab=null
                upsert(AgentStatus::Closed, None)?;
                let alive = self.alive_count().to_string();
                self.harness
                    .event_buffer
                    .push(crate::i18n::trf(lang, "hook.closed", &[("name", name.clone()), ("alive", alive)]));
            }
            // Queue 注入（放行后触发）
            "user_prompt" => {
                upsert(AgentStatus::Processing, tab)?;
                let p = prompt.unwrap_or("").trim().to_string();
                self.enqueue(Role::System, crate::i18n::trf(lang, "hook.user-prompt", &[("name", name.clone()), ("p", p)]), crate::queue::QueueSource::HookUserPrompt, ts)?;
            }
            "notification" => {
                let m = message.unwrap_or("").trim().to_string();
                self.enqueue(Role::System, crate::i18n::trf(lang, "hook.notification", &[("name", name.clone()), ("m", m)]), crate::queue::QueueSource::HookNotification, ts)?;
            }
            "stop" => {
                upsert(AgentStatus::Idle, tab)?;
                let hint = last_assistant_message.unwrap_or("").trim();
                // stop_hook_mode 三模式（Config 热生效）；
                // source 按模式与产物语义分标
                let (text, source) = match self.config.stop_hook_mode.as_str() {
                    // A：stop 到达即读通道全量（tab 切换限流见 timer，此处只读）
                    "auto_read" => {
                        let content = self
                            .read_terminal_filtered(&name, RecordSource::Hook, ts)
                            .await;
                        match content {
                            Some(filtered) => {
                                let len = filtered.chars().count().to_string();
                                (crate::i18n::trf(lang, "hook.stop.updated", &[("name", name.clone()), ("len", len)]),
                                 crate::queue::QueueSource::HookStopContent)
                            }
                            // 读失败回落 hint——语义同 queue_only 产物
                            None => (crate::i18n::trf(lang, "hook.stop.hint", &[("name", name.clone()), ("hint", hint.to_string())]),
                                     crate::queue::QueueSource::HookStopHint),
                        }
                    }
                    // C：汇报原文直达（零 UIA）
                    "message" => {
                        if hint.is_empty() {
                            (crate::i18n::trf(lang, "hook.stop.empty", &[("name", name.clone())]),
                             crate::queue::QueueSource::HookStopReport)
                        } else {
                            (crate::i18n::trf(lang, "hook.stop.report", &[("name", name.clone()), ("hint", hint.to_string())]),
                             crate::queue::QueueSource::HookStopReport)
                        }
                    }
                    // B（默认）：hint 注入，宠物按需 fetch
                    _ => {
                        if hint.is_empty() {
                            (crate::i18n::trf(lang, "hook.stop.empty", &[("name", name.clone())]),
                             crate::queue::QueueSource::HookStopHint)
                        } else {
                            (crate::i18n::trf(lang, "hook.stop.hint", &[("name", name.clone()), ("hint", hint.to_string())]),
                             crate::queue::QueueSource::HookStopHint)
                        }
                    }
                };
                self.enqueue(Role::System, text, source, ts)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// mock hook
    pub async fn handle_hook(
        &mut self,
        event: &str,
        instance: &str,
        project: &str,
        content: &str,
        ts: i64,
    ) -> std::io::Result<()> {
        // 读取链：原文先存 terminal-content.jsonl，再 Filter 存 context.jsonl
        self.harness.append_terminal_content(TerminalContentRecord {
            instance: instance.into(),
            raw: content.to_string(),
            source: RecordSource::Hook,
            ts,
        })?;
        // Filter：Content → Context 链路，存归一后文本，字数按归一后计。
        // mock hook 语义 = 模拟 claude CLI 实例，
        // 故 kind 恒为 claude（Filter 按实例 kind 选择）
        let filtered = crate::filter::by_name("claude")
            .expect("claude filter 必须存在")
            .digest(content)
            .render();
        // Hook 到达 → Timer 重排（近期有 Hook 的实例不该被补扫）
        self.timers.reset(instance, ts);
        match event {
            // 静默簿记（EventBuffer，pet 不醒 mock 契约对齐真实分层）
            "session_start" => {
                self.harness.upsert_agent(AgentEntry {
                    hash: crate::agent_hash(instance, project, ts),
                    name: instance.into(),
                    project: project.into(),
                    kind: Some("claude".into()),
                    status: AgentStatus::Idle,
                    tab: None,
                    first_seen: ts,
                    last_seen: ts,
                })?;
                self.note_filtered(instance, filtered);
                let alive = self.alive_count().to_string();
                self.harness
                    .event_buffer
                    .push(crate::i18n::trf(
                        crate::i18n::Lang::of(&self.config.harness_language),
                        "hook.register",
                        &[("name", instance.to_string()), ("alive", alive)],
                    ));
            }
            "stop" => {
                // 同名不同命：沿用该名字最近一条未 closed 的生命周期（hash/first_seen）
                let (hash, first_seen) = self
                    .harness
                    .agents
                    .iter()
                    .rev()
                    .find(|a| a.name == instance && a.status != AgentStatus::Closed)
                    .map(|a| (a.hash.clone(), a.first_seen))
                    .unwrap_or_else(|| (crate::agent_hash(instance, project, ts), ts));
                self.harness.upsert_agent(AgentEntry {
                    hash,
                    name: instance.into(),
                    project: project.into(),
                    kind: Some("claude".into()),
                    status: AgentStatus::Idle,
                    tab: None,
                    first_seen,
                    last_seen: ts,
                })?;
                self.note_filtered(instance, filtered.clone());
                let len = filtered.chars().count().to_string();
                self.enqueue(
                    Role::System,
                    crate::i18n::trf(
                        crate::i18n::Lang::of(&self.config.harness_language),
                        "hook.stop.updated",
                        &[("name", instance.to_string()), ("len", len)],
                    ),
                    crate::queue::QueueSource::MockHook,
                    ts,
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    /// 提取到期的兜底扫描实例
    /// 提取到期的兜底扫描实例；
    /// timer.interval_ms ≤ 0 = 禁用（真实 hook 接入初期只留 hook 驱动，设计决定）
    pub fn due_timer_scans(&mut self, now: i64, batch: usize) -> Vec<String> {
        if self.config.timer.interval_ms <= 0 {
            return vec![];
        }
        self.timers.due(now, batch)
    }

    /// 实例记录里已定位的 tab（最新存活记录）——timer「按已定位 tab 读」的取口，
    /// server timer 与 case-runner 共用
    pub fn located_tab(&self, inst: &str) -> Option<crate::TabRef> {
        self.harness
            .agents
            .iter()
            .rev()
            .find(|a| a.name == inst && a.status != crate::AgentStatus::Closed)
            .and_then(|a| a.tab)
    }

    /// 枚举对账自愈：marker 在新位置找到 → 回写最新存活记录的 tab（位置漂移非死亡）
    pub fn heal_instance_tab(&mut self, inst: &str, tab: crate::TabRef) {
        let entry = self
            .harness
            .agents
            .iter()
            .rev()
            .find(|a| a.name == inst && a.status != crate::AgentStatus::Closed)
            .cloned();
        if let Some(a) = entry {
            let _ = self.harness.upsert_agent(AgentEntry {
                tab: Some(tab),
                last_seen: crate::server::now_ms(),
                ..a
            });
        }
    }

    /// 实例 kind 解析（Filter 按实例 kind 选择）
    fn resolve_kind(&self, instance: &str) -> Option<String> {
        self.harness
            .agents
            .iter()
            .rev()
            .find(|a| a.name == instance && a.status != AgentStatus::Closed)
            .and_then(|a| a.kind.clone())
    }

    /// filtered_content 现算（不持久化）：
    /// terminal-content.jsonl 原文逐条 digest 出归一全文（agent 实际读到的终端内容）；
    /// 逐条按实例 kind 选择 Filter，kind 缺失/不受支持的记录不入归一视图
    pub fn filtered_content(&self) -> Vec<crate::FilteredContent> {
        self.harness
            .terminal_content_records()
            .unwrap_or_default()
            .iter()
            .filter_map(|r| {
                let kind = self.resolve_kind(&r.instance)?;
                let f = crate::filter::by_name(&kind)?;
                Some(crate::FilteredContent {
                    instance: r.instance.clone(),
                    filtered_content: f.digest(&r.raw).render(),
                    source: r.source,
                    ts: r.ts,
                })
            })
            .collect()
    }

    /// 某实例最新一条归一全文（fetch_terminal 回退/追问，从原文现算）
    pub fn filtered_content_latest(&self, instance: &str) -> Option<crate::FilteredContent> {
        let kind = self.resolve_kind(instance)?;
        let f = crate::filter::by_name(&kind)?;
        self.harness
            .terminal_content_records()
            .unwrap_or_default()
            .iter()
            .rev()
            .find(|r| r.instance == instance)
            .map(|r| crate::FilteredContent {
                instance: r.instance.clone(),
                filtered_content: f.digest(&r.raw).render(),
                source: r.source,
                ts: r.ts,
            })
    }

    /// Filter 按实例 kind 现选现缓存（唯一按实例 hook kind 选择；
    /// 无全局策略、无默认回退）。kind 缺失或不受支持 → None（调用方直接拒绝）
    fn filter_for(&mut self, instance: &str) -> Option<Arc<dyn Filter + Send + Sync>> {
        let kind = self
            .harness
            .agents
            .iter()
            .rev()
            .find(|a| a.name == instance && a.status != AgentStatus::Closed)
            .and_then(|a| a.kind.clone())?;
        if let Some(f) = self.filter_cache.get(&kind) {
            return Some(f.clone());
        }
        let f = crate::filter::by_name(&kind)?;
        self.filter_cache.insert(kind.clone(), f.clone());
        Some(f)
    }

    /// kind 合法性（注册/事件前置判据）：kind 存在且受支持
    fn kind_supported(kind: Option<&str>) -> bool {
        kind.map_or(false, |k| crate::filter::by_name(k).is_some())
    }

    /// 变化检测 prev 登记（每实例最新已知归一全文；内存态，重启丢）；
    /// 读路径顺带定位回写（未命中再按 marker 找，找到回写注册表快照）
    fn note_filtered(&mut self, instance: &str, filtered: String) {
        self.filtered_prev.insert(instance.to_string(), filtered);
        let needs_locate = self
            .harness
            .agents
            .iter()
            .rev()
            .find(|a| a.name == instance && a.status != AgentStatus::Closed)
            .map(|a| a.tab.is_none())
            .unwrap_or(false);
        if !needs_locate {
            return;
        }
        let located = self
            .terminal
            .as_ref()
            .and_then(|t| crate::terminal::join_instance(t.as_ref(), instance));
        if let Some(tabref) = located {
            if let Some(a) = self
                .harness
                .agents
                .iter()
                .rev()
                .find(|a| a.name == instance && a.status != AgentStatus::Closed)
                .cloned()
            {
                let _ = self.harness.upsert_agent(AgentEntry {
                    tab: Some(tabref),
                    last_seen: crate::server::now_ms(),
                    ..a
                });
            }
        }
    }

    /// Timer 兜底扫描处理：
    /// Filter → 变化检测 → Substantive 才注入 Queue 评估；Minor/Unchanged 只存档不打扰
    pub async fn handle_timer_scan(
        &mut self,
        instance: &str,
        content: &str,
        ts: i64,
    ) -> std::io::Result<()> {
        // 原文先存档，再 Filter + 变化检测
        self.harness.append_terminal_content(TerminalContentRecord {
            instance: instance.into(),
            raw: content.to_string(),
            source: RecordSource::Timer,
            ts,
        })?;
        // Filter 按实例 kind：缺失/不受支持在 Filter 与 Queue 之前拒绝
        // （原文存档不受影响；判死读通道在调用方，不经此函数）
        let Some(filter) = self.filter_for(instance) else {
            eprintln!("[timer-scan] {instance} kind 缺失或不受支持，内容处理拒绝");
            return Ok(());
        };
        let filtered = filter.digest(content).render();
        // 变化检测 prev 存内存（重启丢）
        let prev = self.filtered_prev.get(instance).cloned().unwrap_or_default();
        let change = filter.detect_change(&prev, &filtered);
        let len = filtered.chars().count();
        self.note_filtered(instance, filtered);
        if matches!(change, Change::Substantive(_)) {
            self.enqueue(
                Role::System,
                crate::i18n::trf(
                    crate::i18n::Lang::of(&self.config.harness_language),
                    "timer.scan.updated",
                    &[("name", instance.to_string()), ("len", len.to_string())],
                ),
                crate::queue::QueueSource::TimerScan,
                ts,
            )?;
        }
        Ok(())
    }

    /// Timer 兜底扫描发现 tab 不复存在 → closed 终态（永久日志的消亡语义）
    /// Timer 判死（读通道返回 None）：该名字全部未 closed 生命周期各 append 一条
    /// closed 快照——读通道按 name 读，同名实例在读取侧不可区分，判死须同判
    ///（同名不同命：每 hash 独立快照，append-only 语义不变）。
    /// 判死 diff 事件化（#16 case 跑红实锤）：EventBuffer 簿记，下次放行附带入
    /// Context——否则 LLM 的全景认知停在旧快照（判死后仍答错实例数）。
    pub fn mark_instance_closed(&mut self, instance: &str, ts: i64) -> std::io::Result<()> {
        let targets: Vec<AgentEntry> = self
            .harness
            .agents
            .iter()
            .filter(|a| a.name == instance && a.status != AgentStatus::Closed)
            .cloned()
            .collect();
        if targets.is_empty() {
            return Ok(());
        }
        for a in targets {
            let name = a.name.clone();
            self.harness.upsert_agent(AgentEntry {
                status: AgentStatus::Closed,
                tab: None,
                last_seen: ts,
                ..a
            })?;
            // 判死 diff 事件化 + post-count（#16 ①：每条 hash 一条，post-count 逐条现算，
            // 同名连坐自然形成递减序列；LLM 直接读数免对账）
            let alive = self.alive_count().to_string();
            self.harness
                .event_buffer
                .push(crate::i18n::trf(
                    crate::i18n::Lang::of(&self.config.harness_language),
                    "hook.closed-timer",
                    &[("name", name.clone()), ("alive", alive)],
                ));
        }
        Ok(())
    }

    /// 执行 tool call（run_trigger tool 循环与 case-runner tool_call step 共用）
    pub async fn execute_tool(&mut self, call: &ToolCall) -> (Value, Vec<Effect>) {
        let out = self.execute_tool_inner(call).await;
        // 动作流记录：tool 副作用在此单点记录；
        // ConfigChanged 已在 edit_config_update 内记录（LLM 路径=config_changed/backend），跳过防双写
        for e in &out.1 {
            if !matches!(e, Effect::ConfigChanged { .. }) {
                let (kind, payload) = e.effect_kind_payload();
                let _ = self.harness.log_effect(
                    crate::EffectOrigin::Backend,
                    kind,
                    payload,
                    crate::server::now_ms(),
                );
            }
        }
        out
    }

    async fn execute_tool_inner(&mut self, call: &ToolCall) -> (Value, Vec<Effect>) {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
        // 错误反馈按 Harness 语言现查
        let lang = crate::i18n::Lang::of(&self.config.harness_language);
        match call.name.as_str() {
            "call_component" => {
                let spec = args.get("spec").cloned().unwrap_or(Value::Null);
                let id = spec
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // Tauri label 规则：只允许 [A-Za-z0-9_\-/.]+；id 即 Card 文件相对路径
                //（memory/cards/<id>.card.json）——禁空段与 `..` 段（路径逃逸防护）
                if id.is_empty()
                    || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/')
                    || id.split('/').any(|seg| seg.is_empty() || seg == "..")
                {
                    return (
                        json!({ "ok": false, "error": crate::i18n::trf(lang, "err.component-id", &[("id", id.clone())]) }),
                        vec![],
                    );
                }
                // 显式关闭（持续管理协议）：action="close" 只需合法 id，
                // 不要求 type 必填字段（关闭卡片不需要内容）
                // #23 两级兼容：LLM 有时把 action 放在 args 顶层（与 spec 并列），
                // spec 内查不到时回退 args 顶层，否则 close 会被当成空 update 渲染空卡
                let action = spec
                    .get("action")
                    .or_else(|| args.get("action"))
                    .and_then(Value::as_str);
                if action == Some("close") {
                    let ts = crate::server::now_ms();
                    // dismiss：删 .card.json 文件、出注册表、忘记布局
                    if let Some(entry) = self.harness.cards_remove(&id) {
                        // closed_by_agent 生命周期事件（一行，进 EventBuffer 静默簿记）
                        let alive = self.harness.cards.len();
                        let line = crate::lifecycle::DefaultLifecycle::for_lang(lang).closed_line(&entry.meta, alive, ts);
                        self.harness.event_buffer.push(line);
                    }
                    return (
                        json!({ "ok": true, "closed": id }),
                        vec![Effect::CloseComponent(id)],
                    );
                }
                // 校验 type 合法性 + 按 type 校验必填字段
                let VALID_TYPES: &[&str] = &["text_card", "quick_jump", "git_display", "data_chart", "todobox"];
                if let Some(typ) = spec.get("type").and_then(Value::as_str) {
                    if !VALID_TYPES.contains(&typ) {
                        return (
                            json!({ "ok": false, "error": crate::i18n::trf(lang, "err.component-type", &[("typ", typ.to_string()), ("valid", VALID_TYPES.join("/"))]) }),
                            vec![],
                        );
                    }
                    let required: &[&str] = match typ {
                        "text_card" => &["title", "text"],
                        "quick_jump" => &["label", "target"],
                        "git_display" => &["title", "entries"],
                        "data_chart" => &["title", "chart"],
                        "todobox" => &["title", "items"],
                        _ => &[],
                    };
                    let missing: Vec<&str> = required.iter()
                        .filter(|f| spec.get(f).map_or(true, |v| match v {
                            Value::String(s) => s.is_empty(),
                            Value::Array(a) => a.is_empty(),
                            Value::Object(o) => o.is_empty(),
                            _ => true,
                        }))
                        .copied()
                        .collect();
                    if !missing.is_empty() {
                        return (
                            json!({ "ok": false, "error": crate::i18n::trf(lang, "err.component-missing", &[("typ", typ.to_string()), ("missing", missing.join(", "))]) }),
                            vec![],
                        );
                    }
                    // todobox items 结构校验：[{text, done}]
                    if typ == "todobox" {
                        let bad = spec["items"].as_array().map(|arr| arr.iter().any(|it| {
                            it["text"].as_str().map_or(true, str::is_empty) || it["done"].as_bool().is_none()
                        })).unwrap_or(true);
                        if bad {
                            return (
                                json!({ "ok": false, "error": crate::i18n::tr(lang, "err.todobox-items") }),
                                vec![],
                            );
                        }
                    }
                    // git_display entries / data_chart chart 结构校验
                    if typ == "git_display" {
                        let bad = spec["entries"].as_array().map(|arr| arr.iter().any(|e| {
                            e["hash"].as_str().is_none() || e["msg"].as_str().is_none() || e["time"].as_str().is_none()
                        })).unwrap_or(true);
                        if bad {
                            return (
                                json!({ "ok": false, "error": crate::i18n::tr(lang, "err.git-entries") }),
                                vec![],
                            );
                        }
                    }
                    if typ == "data_chart" {
                        let c = &spec["chart"];
                        let kind_ok = c["kind"].as_str().map_or(false, |k| ["line", "bar", "pie"].contains(&k));
                        let series_ok = c["series"].as_array().map_or(false, |arr| {
                            !arr.is_empty() && arr.iter().all(|s| {
                                s["name"].as_str().is_some()
                                    && s["data"].as_array().map_or(false, |d| d.iter().all(|v| v.is_number()))
                            })
                        });
                        let labels_ok = c["labels"].as_array().map_or(false, |l| l.iter().all(|v| v.is_string()));
                        if !(kind_ok && series_ok && labels_ok) {
                            return (
                                json!({ "ok": false, "error": crate::i18n::tr(lang, "err.chart") }),
                                vec![],
                            );
                        }
                    }
                }
                // direction 合法性（auto/八方位）
                if let Some(dir) = spec.get("direction").and_then(Value::as_str) {
                    if !["auto", "n", "ne", "e", "se", "s", "sw", "w", "nw"].contains(&dir) {
                        return (
                            json!({ "ok": false, "error": crate::i18n::trf(lang, "err.direction", &[("dir", dir.to_string())]) }),
                            vec![],
                        );
                    }
                }
                // 创建 / 原地更新（同 id 不再 toggle 关闭）：先落 .card.json 文件再改注册表；
                // 更新只换 component，_meta（显示选择/布局）保留——Agent 不能借更新覆盖用户选择
                let ts = crate::server::now_ms();
                match self.harness.cards_upsert(&spec, ts) {
                    Err(e) => (json!({ "ok": false, "error": e }), vec![]),
                    Ok((meta, created)) => {
                        if created {
                            // created 生命周期事件（进 EventBuffer 静默簿记；agent 更新不产事件）
                            let line = crate::lifecycle::DefaultLifecycle::for_lang(lang).created_line(&meta, self.harness.cards.len());
                            self.harness.event_buffer.push(line);
                            return (json!({ "ok": true, "rendered": id }), vec![Effect::RenderComponent(spec)]);
                        }
                        (json!({ "ok": true, "updated": id }), vec![Effect::RenderComponent(spec)])
                    }
                }
            }
            "fetch_terminal" => {
                let inst = args.get("instance").and_then(Value::as_str).unwrap_or("");
                if inst.is_empty() {
                    return (json!({ "ok": false, "error": crate::i18n::tr(lang, "err.instance-required") }), vec![]);
                }
                // vd_switch 必填：打断性决策每次显式面对
                let Some(vd_switch) = args.get("vd_switch").and_then(Value::as_bool) else {
                    return (
                        json!({ "ok": false, "error": crate::i18n::tr(lang, "err.vd-switch-required") }),
                        vec![],
                    );
                };
                // 读通道优先（Terminal Adapter）：读到原文先存档再过滤（读取链）
                if let Some(content) = self
                    .read_terminal_filtered(inst, RecordSource::FetchTerminal, crate::server::now_ms())
                    .await
                {
                    // 成功返回携带 ok:true（成败形态自洽）
                    return (json!({ "ok": true, "instance": inst, "content": content }), vec![]);
                }
                // 新鲜读失败 → 最新归一全文回退（有历史给历史；从原文现算）
                if let Some(rec) = self.filtered_content_latest(inst) {
                    return (json!({ "ok": true, "instance": inst, "content": rec.filtered_content }), vec![]);
                }
                // 什么都没有：vd_switch=false → 失败教学；true → 切桌面重试
                if !vd_switch {
                    return (
                        json!({ "ok": false, "error": crate::i18n::trf(lang, "err.fetch-unreadable", &[("inst", inst.to_string())]) }),
                        vec![],
                    );
                }
                // 切桌面（Example F）：join 拿 hwnd → primitives 切换
                let switched = self
                    .terminal
                    .as_ref()
                    .and_then(|t| crate::terminal::join_instance(t.as_ref(), inst))
                    .map(|tab| tab.hwnd)
                    .and_then(|hwnd| self.primitives.as_ref().map(|p| p.switch_vd(hwnd)))
                    .unwrap_or(false);
                if !switched {
                    return (
                        json!({ "ok": false, "error": crate::i18n::trf(lang, "err.vd-switch-failed", &[("inst", inst.to_string())]) }),
                        vec![],
                    );
                }
                let content = self
                    .read_terminal_filtered(inst, RecordSource::FetchTerminal, crate::server::now_ms())
                    .await
                    .unwrap_or_else(|| crate::i18n::tr(lang, "fetch.switched-empty").to_string());
                (json!({ "ok": true, "instance": inst, "content": content }), vec![])
            }
            "set_autonomy" => {
                let mut face = args.get("key").and_then(Value::as_str).map(String::from);
                let motion = args.get("motion").and_then(Value::as_str).map(String::from);
                // key 传状态 key 名：解析为映射表本体；motion 不连带——
                // 「仅传参的字段被覆盖」，缺省即不碰
                if let Some(f) = &face {
                    if let Some(entry) = self.config.kaomoji_resolve(f.as_str()) {
                        face = Some(entry.face.clone());
                    } else {
                        return (
                            json!({ "ok": false, "error": crate::i18n::trf(lang, "err.autonomy-key", &[("key", f.to_string())]) }),
                            vec![],
                        );
                    }
                }
                if let Some(m) = &motion {
                    let valid = ["still", "float", "bounce", "shake"];
                    if !valid.contains(&m.as_str()) {
                        return (
                            json!({ "ok": false, "error": crate::i18n::trf(lang, "err.autonomy-motion", &[("motion", m.to_string()), ("valid", valid.join("/"))]) }),
                            vec![],
                        );
                    }
                }
                // once 契约：两套持续时间语义互斥，同传直接拒绝
                let once = args.get("once").and_then(Value::as_bool).unwrap_or(false);
                let ttl_ms = args.get("ttlMs").and_then(Value::as_u64);
                if once && ttl_ms.is_some() {
                    return (
                        json!({ "ok": false, "error": crate::i18n::tr(lang, "err.autonomy-once-ttl") }),
                        vec![],
                    );
                }
                (
                    json!({ "ok": true }),
                    vec![Effect::SetAutonomy {
                        face,
                        motion,
                        ttl_ms,
                        once,
                    }],
                )
            }
            "edit_config" => {
                // 单一 Config 工具，显式 action（不以缺参、空值或失败写入切换模式；渐进披露，按需查）
                let action = args.get("action").and_then(Value::as_str).unwrap_or("");
                match action {
                    "grep" => self.edit_config_grep(&args),
                    "query" => self.edit_config_query(&args, &call.id),
                    "update" => {
                        let (r, e) = self.edit_config_update(&args);
                        return (r, e);
                    }
                    _ => (
                        json!({ "ok": false, "error": crate::i18n::trf(lang, "err.bad-action", &[("action", action.to_string())]) }),
                        vec![],
                    ),
                }
            }
            "read_memory" => {
                // name 省略 = 读 index.md 导航首页
                let name = args.get("name").and_then(Value::as_str);
                match self.harness.memory.read(lang, name) {
                    Ok((name, content)) => (json!({ "ok": true, "name": name, "content": content }), vec![]),
                    Err(e) => (json!({ "ok": false, "error": e }), vec![]),
                }
            }
            "write_memory" => {
                // 新建或完整替换；必须附 description；index.md 自动重生成
                let name = args.get("name").and_then(Value::as_str).unwrap_or("");
                let Some(content) = args.get("content").and_then(Value::as_str) else {
                    return (json!({ "ok": false, "error": crate::i18n::tr(lang, "mem.content-required") }), vec![]);
                };
                let Some(desc) = args.get("description").and_then(Value::as_str) else {
                    return (json!({ "ok": false, "error": crate::i18n::tr(lang, "mem.desc-required") }), vec![]);
                };
                match self.harness.memory.write(lang, name, content, desc) {
                    Ok(()) => (json!({ "ok": true, "name": name }), vec![]),
                    Err(e) => (json!({ "ok": false, "error": e }), vec![]),
                }
            }
            "cron_create" => {
                // schedule 二选一 + message
                let schedule = args.get("schedule").cloned().unwrap_or(Value::Null);
                let message = args.get("message").and_then(Value::as_str).unwrap_or("");
                let at = schedule.get("at").and_then(Value::as_i64);
                let every = schedule.get("every_ms").and_then(Value::as_u64);
                if at.is_some() && every.is_some() {
                    return (
                        json!({ "ok": false, "error": crate::i18n::tr(lang, "cron.schedule-conflict") }),
                        vec![],
                    );
                }
                let parsed = at
                    .map(crate::cron::Schedule::At)
                    .or(every.map(crate::cron::Schedule::EveryMs));
                let Some(schedule) = parsed else {
                    return (
                        json!({ "ok": false, "error": crate::i18n::tr(lang, "cron.schedule-missing") }),
                        vec![],
                    );
                };
                match self
                    .harness
                    .cron
                    .create(schedule, message, crate::server::now_ms())
                {
                    Ok(id) => (json!({ "ok": true, "id": id }), vec![]),
                    Err(e) => (json!({ "ok": false, "error": e }), vec![]),
                }
            }
            "cron_delete" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                match self.harness.cron.delete(id, crate::server::now_ms()) {
                    Ok(()) => (json!({ "ok": true, "deleted": id }), vec![]),
                    Err(e) => (json!({ "ok": false, "error": e }), vec![]),
                }
            }
            "sleep" => {
                // tool result 延迟返回，等待后继续既定工具序列；
                // waiters 经共享句柄注册（调度任务在锁外到点唤醒，无死锁）
                let Some(ms) = args.get("ms").and_then(Value::as_u64) else {
                    return (json!({ "ok": false, "error": crate::i18n::tr(lang, "sleep.ms-required") }), vec![]);
                };
                if ms > crate::cron::MAX_SLEEP_MS {
                    return (
                        json!({ "ok": false, "error": crate::i18n::trf(lang, "sleep.ms-over", &[("ms", ms.to_string()), ("max", crate::cron::MAX_SLEEP_MS.to_string())]) }),
                        vec![],
                    );
                }
                let fire_ts = crate::server::now_ms() + ms as i64;
                let rx = self.harness.cron.waiter_handle().register(fire_ts);
                let _ = rx.await; // 占用 Queue 串行点等待（既定成本）
                (json!({ "ok": true, "slept_ms": ms }), vec![])
            }
            other => (
                json!({ "ok": false, "error": crate::i18n::trf(lang, "err.unknown-tool", &[("name", other.to_string())]) }),
                vec![],
            ),
        }
    }
}

/// 配置修改结果（apply_config_by_path 返回）
pub struct ConfigOutcome {
    pub effects: Vec<Effect>,
    pub llm_changed: bool,
    pub restart_required: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{DebugAgent, LlmOutput};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ambery-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// 沉默 mock：不注入任何反应的测试用
    fn make_ambery(tag: &str) -> AmberyBackend<DebugAgent> {
        make_ambery_with(tag, DebugAgent::silent())
    }

    fn make_ambery_with(tag: &str, agent: DebugAgent) -> AmberyBackend<DebugAgent> {
        let dir = tmp_dir(tag);
        let harness = Harness::load(&dir, &dir, 100_000, 0).unwrap();
        AmberyBackend::new(harness, Config::default(), agent)
    }

    #[tokio::test]
    async fn effort_resolved_from_source_with_config_override() {
        // user_chat→low、hook_stop_content→high、其余→medium
        let mut ov = make_ambery("eff1");
        use crate::llm::Effort;
        use crate::queue::QueueSource as S;
        assert_eq!(ov.resolve_effort(S::UserChat), Some(Effort::Low));
        assert_eq!(ov.resolve_effort(S::HookStopContent), Some(Effort::High));
        assert_eq!(ov.resolve_effort(S::TimerScan), Some(Effort::Medium));
        assert_eq!(ov.resolve_effort(S::CronTick), Some(Effort::Medium));
        assert_eq!(ov.resolve_effort(S::MockHook), Some(Effort::Medium));
        // Config 覆盖（effort.user_chat / effort.default）
        ov.config.effort.user_chat = Some(Effort::High);
        ov.config.effort.default = Some(Effort::Low);
        assert_eq!(ov.resolve_effort(S::UserChat), Some(Effort::High));
        assert_eq!(ov.resolve_effort(S::HookStopContent), Some(Effort::High)); // 未覆盖项不动
        assert_eq!(ov.resolve_effort(S::TimerScan), Some(Effort::Low));
        let _ = std::fs::remove_dir_all(tmp_dir("eff1"));
    }

    #[tokio::test]
    async fn effort_keyword_rewrite_only_for_user_chat() {
        // user_chat 命中关键词 → 本次临时改写；多命中取最长
        let mut ov = make_ambery("eff2");
        use crate::llm::Effort;
        use crate::queue::QueueSource as S;
        let ask = |ov: &mut AmberyBackend<DebugAgent>, text: &str| {
            ov.enqueue(Role::User, text.into(), S::UserChat, 1).unwrap();
            // 静默 mock 不产生 assistant 回复——末条即该 user 消息
            ov.harness.queue.release().unwrap();
            ov.harness
                .append_context(crate::context::ContextMessage::new(Role::User, text.to_string(), 1))
                .unwrap();
            ov.resolve_effort(S::UserChat)
        };
        assert_eq!(ask(&mut ov, "仔细想想这个架构"), Some(Effort::High));
        assert_eq!(ask(&mut ov, "快点告诉我结果"), Some(Effort::Low));
        // 多命中取最长关键词：仔细想想(4) > 快点(2)
        assert_eq!(ask(&mut ov, "仔细想想，快点"), Some(Effort::High));
        // 未命中关键词的 user_chat 保持来源默认 low
        assert_eq!(ask(&mut ov, "在吗"), Some(Effort::Low));
        // 关键词改写只作用 user_chat：hook 内容含关键词不改写（medium）
        ov.harness
            .append_context(crate::context::ContextMessage::new(Role::System, "仔细想想".to_string(), 2))
            .unwrap();
        assert_eq!(ov.resolve_effort(S::HookStopHint), Some(Effort::Medium));
        let _ = std::fs::remove_dir_all(tmp_dir("eff2"));
    }

    #[tokio::test]
    async fn queue_source_annotated_per_entry_point() {
        // 入队点逐一标注
        let mut ov = make_ambery("qsrc");
        // user_prompt hook → HookUserPrompt
        ov.handle_real_hook("user_prompt", "sess-1111-2222", "/tmp/p", Some("claude"), Some("干这个"), None, None, 1)
            .await
            .unwrap();
        // notification hook → HookNotification
        ov.handle_real_hook("notification", "sess-1111-2222", "/tmp/p", Some("claude"), None, Some("权限询问"), None, 2)
            .await
            .unwrap();
        let sources: Vec<_> = ov.harness.queue.iter().map(|i| i.source).collect();
        assert_eq!(
            sources,
            vec![
                crate::queue::QueueSource::HookUserPrompt,
                crate::queue::QueueSource::HookNotification,
            ],
            "{sources:?}"
        );
        // queue.jsonl 落盘携带 source；直接拼路径——tmp_dir() 会清空目录
        let dir = std::env::temp_dir().join(format!("ambery-test-qsrc-{}", std::process::id()));
        let raw = std::fs::read_to_string(dir.join(crate::QUEUE_FILE)).unwrap();
        assert!(raw.contains("\"source\":\"hook_user_prompt\""), "{raw}");
        assert!(raw.contains("\"source\":\"hook_notification\""), "{raw}");
        let _ = std::fs::remove_dir_all(tmp_dir("qsrc"));
    }

    #[tokio::test]
    async fn queue_source_stop_three_modes() {
        // stop 三模式来源分标
        // queue_only（默认）→ hint；message → report；auto_read 读成功 → content
        let mut ov = make_ambery("qsrc-hint");
        ov.handle_real_hook("session_start", "s0a00000-1", "/tmp/p", Some("claude"), None, None, None, 1).await.unwrap();
        ov.handle_real_hook("stop", "s0a00000-1", "/tmp/p", Some("claude"), None, None, Some("修完了"), 2).await.unwrap();
        assert_eq!(ov.harness.queue.iter().next().unwrap().source, crate::queue::QueueSource::HookStopHint);
        let _ = std::fs::remove_dir_all(tmp_dir("qsrc-hint"));

        let mut ov = make_ambery("qsrc-report");
        ov.config.stop_hook_mode = "message".into();
        ov.handle_real_hook("session_start", "s0a00000-1", "/tmp/p", Some("claude"), None, None, None, 1).await.unwrap();
        ov.handle_real_hook("stop", "s0a00000-1", "/tmp/p", Some("claude"), None, None, Some("修了 3 个文件"), 2).await.unwrap();
        assert_eq!(ov.harness.queue.iter().next().unwrap().source, crate::queue::QueueSource::HookStopReport);
        let _ = std::fs::remove_dir_all(tmp_dir("qsrc-report"));

        let mut ov = make_ambery("qsrc-content");
        ov.config.stop_hook_mode = "auto_read".into();
        let map = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([(
            "p·s0a00000".to_string(),
            "全量内容".to_string(),
        )])));
        ov.terminal = Some(std::sync::Arc::new(crate::terminal::MapAdapter::new(map)));
        ov.handle_real_hook("session_start", "s0a00000-1", "/tmp/p", Some("claude"), None, None, None, 1).await.unwrap();
        ov.handle_real_hook("stop", "s0a00000-1", "/tmp/p", Some("claude"), None, None, None, 2).await.unwrap();
        assert_eq!(ov.harness.queue.iter().next().unwrap().source, crate::queue::QueueSource::HookStopContent);
        let _ = std::fs::remove_dir_all(tmp_dir("qsrc-content"));
    }

    /// 脚本决策源：每次 LLM 调用按序弹出一条；耗尽后沉默
    fn scripted(outputs: Vec<LlmOutput>) -> DebugAgent {
        let rest = std::sync::Mutex::new(std::collections::VecDeque::from(outputs));
        DebugAgent::new(move |_| rest.lock().unwrap().pop_front().unwrap_or_else(silence))
    }

    /// 测试用 TerminalAdapter：固定 tab/内容；title 承载 join 匹配用实例名
    struct StubAdapter {
        tab: Option<crate::TabRef>,
        title: String,
        content: Option<String>,
    }
    impl crate::terminal::TerminalAdapter for StubAdapter {
        fn enumerate(&self) -> Option<Vec<crate::terminal::TabInfo>> {
            Some(match self.tab {
                Some(tab) => vec![crate::terminal::TabInfo {
                    tab,
                    title: Some(self.title.clone()),
                    cwd: None,
                    command: None,
                    focused: None,
                    extras: Default::default(),
                }],
                None => vec![],
            })
        }
        fn read(&self, _tab: &crate::TabRef) -> crate::terminal::ReadOutcome {
            match self.content.clone() {
                Some(c) => crate::terminal::ReadOutcome::Content(c),
                None => crate::terminal::ReadOutcome::Error("stub no content".into()),
            }
        }
    }
    fn stub_adapter(tab: Option<crate::TabRef>, title: &str, content: Option<&str>) -> std::sync::Arc<StubAdapter> {
        std::sync::Arc::new(StubAdapter {
            tab,
            title: title.to_string(),
            content: content.map(String::from),
        })
    }

    /// 测试用 PlatformPrimitives：固定切换结果 + 记录 hwnd
    struct StubPrimitives {
        result: bool,
        pub switched: std::sync::Mutex<Vec<i64>>,
    }
    impl crate::terminal::PlatformPrimitives for StubPrimitives {
        fn switch_vd(&self, hwnd: i64) -> bool {
            self.switched.lock().unwrap().push(hwnd);
            self.result
        }
    }

    #[tokio::test]
    async fn tab_lifecycle_join_writeback() {
        // session_start join 探测回写；session_end closed 快照 tab=null
        let mut ov = make_ambery("tab-lifecycle");
        let adapter = stub_adapter(Some(crate::TabRef { hwnd: 100, index: 2 }), "demo·sess-123", None);
        ov.terminal = Some(adapter.clone());
        // session_start：注册 + join 探测回写 tab 快照
        ov.handle_real_hook("session_start", "sess-1234-abc", "/tmp/demo", Some("claude"), None, None, None, 1000)
            .await
            .unwrap();
        let a = ov
            .harness
            .agents
            .iter()
            .rev()
            .find(|a| a.status != crate::AgentStatus::Closed)
            .unwrap()
            .clone();
        assert_eq!(a.tab, Some(crate::TabRef { hwnd: 100, index: 2 }), "join 探测回写");
        // session_end：tab=null
        ov.handle_real_hook("session_end", "sess-1234-abc", "/tmp/demo", Some("claude"), None, None, None, 1001)
            .await
            .unwrap();
        let last = ov.harness.agents.last().unwrap();
        assert_eq!(last.status, crate::AgentStatus::Closed);
        assert_eq!(last.tab, None, "closed 快照 tab 为 null");
    }

    #[tokio::test]
    async fn call_component_validates_direction_entries_chart() {
        // direction 方位集 + entries/chart 结构
        let mut ov = make_ambery("cmp-validate");
        let call = |id: &str, args: serde_json::Value| ToolCall { id: id.into(), name: "call_component".into(), arguments: args.to_string() };
        let (r, _) = ov.execute_tool(&call("t1", json!({"spec":{"id":"a","type":"text_card","title":"t","text":"x","direction":"up"}}))).await;
        assert_eq!(r["ok"], json!(false), "{r}");
        let (r, _) = ov.execute_tool(&call("t2", json!({"spec":{"id":"a","type":"text_card","title":"t","text":"x","direction":"ne"}}))).await;
        assert_eq!(r["ok"], json!(true), "{r}");
        let (r, _) = ov.execute_tool(&call("t3", json!({"spec":{"id":"g","type":"git_display","title":"g","entries":[{"hash":"h"}]}}))).await;
        assert_eq!(r["ok"], json!(false), "{r}");
        let (r, _) = ov.execute_tool(&call("t4", json!({"spec":{"id":"g","type":"git_display","title":"g","entries":[{"hash":"h","msg":"m","time":"t"}]}}))).await;
        assert_eq!(r["ok"], json!(true), "{r}");
        let (r, _) = ov.execute_tool(&call("t5", json!({"spec":{"id":"c","type":"data_chart","title":"c","chart":{"kind":"donut","labels":[],"series":[]}}}))).await;
        assert_eq!(r["ok"], json!(false), "{r}");
        let (r, _) = ov.execute_tool(&call("t6", json!({"spec":{"id":"c","type":"data_chart","title":"c","chart":{"kind":"line","labels":["a"],"series":[{"name":"s","data":[1.0]}]}}}))).await;
        assert_eq!(r["ok"], json!(true), "{r}");
    }

    #[tokio::test]
    async fn fetch_terminal_ok_flag_consistent() {
        // 成功与失败形态自洽（ok 字段恒在）
        let mut ov = make_ambery("fetch-ok");
        // Filter 按实例 kind：未注册实例 kind 缺失，读取被拒绝
        let call = ToolCall { id: "f".into(), name: "fetch_terminal".into(), arguments: json!({"instance":"ghost","vd_switch":false}).to_string() };
        let (r, _) = ov.execute_tool(&call).await;
        assert_eq!(r["ok"], json!(false), "{r}");
        // 注册（带 kind）后读取正常（name = <project>·<sid8>）
        ov.handle_real_hook("session_start", "sess0000-1111", "/tmp/ghost", Some("claude"), None, None, None, 1)
            .await
            .unwrap();
        ov.terminal = Some(stub_adapter(Some(crate::TabRef { hwnd: 1, index: 0 }), "ghost·sess0000", Some("内容")));
        let (r, _) = ov.execute_tool(&ToolCall { id: "f2".into(), name: "fetch_terminal".into(), arguments: json!({"instance":"ghost·sess0000","vd_switch":false}).to_string() }).await;
        assert_eq!(r["ok"], json!(true), "{r}");
    }

    #[test]
    fn pet_name_flows_into_system_prompt_and_validates() {
        // 默认名：Ambery
        let ov = make_ambery("petname");
        assert_eq!(ov.config.name, "Ambery");
        // 拼装请求头读取当前名称（{name} 占位替换）
        let head = ov.assemble_system_prompt();
        assert!(head.contains("你是 Ambery"), "{head}");
        assert!(!head.contains("{name}"), "{head}");
        // 改名 → 下一次拼装即当前名称（身份文案热读取）；空名/超长名原子拒绝
        // 用非 ASCII 多字节名验证 {name} 占位替换（Unicode 边界）
        let mut ov = ov;
        let r = ov.apply_config_by_path("name", serde_json::json!("测试名字"));
        assert!(r.is_ok());
        let head = ov.assemble_system_prompt();
        assert!(head.contains("你是 测试名字"), "{head}");
        assert!(ov.apply_config_by_path("name", serde_json::json!("  ")).is_err());
        assert!(ov.apply_config_by_path("name", serde_json::json!("x".repeat(65))).is_err());
    }

    #[tokio::test]
    async fn harness_language_switches_tool_and_event_texts() {
        // harness_language=en：工具说明英文（机器契约不译）+ hook 事件文字英文
        let dir = tmp_dir("i18n-en");
        let harness = Harness::load(&dir, &dir, 100_000, 0).unwrap();
        let mut config = Config::default();
        config.harness_language = "en".into();
        let mut ov = AmberyBackend::new(harness, config, DebugAgent::silent());
        // 工具说明：下一次交互的请求构建现查表
        let tools = crate::llm::tool_set(crate::i18n::Lang::of(&ov.config.harness_language));
        let sleep = tools.iter().find(|t| t.name == "sleep").unwrap();
        assert!(sleep.description.contains("Wait"), "{}", sleep.description);
        assert_eq!(sleep.name, "sleep"); // tool name 机器契约不译
        // hook 事件文字：事件发生时刻的语言
        ov.handle_real_hook("session_start", "sid-0000-1111", "/tmp/demo", Some("claude"), None, None, None, 1000)
            .await
            .unwrap();
        ov.handle_real_hook("notification", "sid-0000-1111", "/tmp/demo", Some("claude"), None, Some("need eyes"), None, 1001)
            .await
            .unwrap();
        let msgs = ov.harness.context.messages();
        assert!(msgs.is_empty(), "静默簿记不进 Context（notification 进 Queue 未放行）");
        // Event Buffer 簿记（注册行英文）
        let buf = ov.harness.event_buffer.events().join("\n");
        assert!(buf.contains("registered"), "{buf}");
        // 放行前 Queue 中的 notification 文本（英文）
        let q = ov
            .harness
            .queue
            .iter()
            .map(|i| i.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(q.contains("requests attention: need eyes"), "{q}");
        // zh 对照：默认语言事件文字中文
        let mut ov2 = make_ambery("i18n-zh");
        ov2.handle_real_hook("session_start", "sid-0000-2222", "/tmp/demo", Some("claude"), None, None, None, 1000)
            .await
            .unwrap();
        let buf2 = ov2.harness.event_buffer.events().join("\n");
        assert!(buf2.contains("注册"), "{buf2}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn say(text: &str) -> LlmOutput {
        LlmOutput {
            content: Some(text.into()),
            tool_calls: vec![],
            reasoning_content: None,
        usage: None,
        }
    }

    fn calls(specs: Vec<(&str, Value)>) -> LlmOutput {
        LlmOutput {
            content: None,
            tool_calls: specs
                .into_iter()
                .enumerate()
                .map(|(i, (name, args))| ToolCall {
                    id: format!("script-{i}"),
                    name: name.into(),
                    arguments: args.to_string(),
                })
                .collect(),
            reasoning_content: None,
        usage: None,
        }
    }

    fn silence() -> LlmOutput {
        LlmOutput {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
        usage: None,
        }
    }

    #[tokio::test]
    async fn stop_hook_scripted_notify_flow() {
        // mock 脚本：hook 触发后决定通知（set_autonomy + call_component），然后沉默
        let agent = scripted(vec![
            calls(vec![
                (
                    "set_autonomy",
                    json!({"key": "notify", "motion": "bounce", "ttlMs": 5000}),
                ),
                (
                    "call_component",
                    json!({"spec": {"id": "notify-ft", "type": "text_card", "title": "ft 完成", "text": "干完了", "direction": "auto"}}),
                ),
            ]),
            silence(),
        ]);
        let mut ov = make_ambery_with("notify", agent);
        let long = "x".repeat(120);
        ov.handle_hook("stop", "ft", "proj", &long, 1).await.unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        assert!(effects.iter().any(|e| matches!(e, Effect::RenderComponent(_))));
        assert!(effects.iter().any(|e| matches!(e, Effect::SetAutonomy { .. })));
        let roles: Vec<Role> = ov.harness.context.messages().iter().map(|m| m.role).collect();
        // system(hook) + assistant(tool_calls) + tool + tool
        assert_eq!(
            roles,
            vec![Role::System, Role::Assistant, Role::Tool, Role::Tool]
        );
        // agent 注册为 idle
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Idle);
        let _ = std::fs::remove_dir_all(tmp_dir("notify"));
    }

    #[tokio::test]
    async fn stop_short_content_silence() {
        let mut ov = make_ambery("silence");
        ov.handle_hook("stop", "oss", "proj", "清理了 2 行注释", 1).await.unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        assert!(effects.is_empty());
        let roles: Vec<Role> = ov.harness.context.messages().iter().map(|m| m.role).collect();
        // system(hook)，沉默不追加 assistant
        assert_eq!(roles, vec![Role::System]);
        let _ = std::fs::remove_dir_all(tmp_dir("silence"));
    }

    #[tokio::test]
    async fn session_start_silent_bookkeeping() {
        // 定案（mock 契约）：session_start = 静默簿记，
        // pet 不醒——注册 Idle + EventBuffer，不进 Context 不触发 LLM
        let mut ov = make_ambery("register");
        ov.handle_hook("session_start", "new-feature", "proj", "启动画面", 1)
            .await
            .unwrap();
        assert_eq!(ov.harness.agents.len(), 1);
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Idle);
        assert!(ov.harness.context.messages().is_empty()); // 无 Queue 注入
        assert_eq!(ov.harness.event_buffer.len(), 1); // 簿记待附带
        // mock 读链存档仍发生（原文 → terminal-content；归一全文现算）
        assert_eq!(
            ov.filtered_content_latest("new-feature").unwrap().filtered_content,
            "启动画面"
        );
        let _ = std::fs::remove_dir_all(tmp_dir("register"));
    }

    #[tokio::test]
    async fn user_followup_triggers_fetch_loop() {
        // mock 脚本：hook 沉默 → 追问时 fetch_terminal → 汇总回复
        let agent = scripted(vec![
            silence(),
            calls(vec![("fetch_terminal", json!({"instance": "ft", "vd_switch": false}))]),
            say("[debug] 查到：全文"),
        ]);
        let mut ov = make_ambery_with("fetch", agent);
        let long = "y".repeat(100);
        ov.handle_hook("stop", "ft", "proj", &long, 1).await.unwrap();
        ov.drain_queue(0).await.unwrap(); // stop 放行（脚本帧 1 沉默）
        ov.harness
            .append_context(ContextMessage::new(Role::User, "那个 bug 具体怎么回事？", 2))
            .unwrap();
        ov.run_trigger(3, crate::queue::QueueSource::MockHook, 0).await.unwrap();
        let msgs = ov.harness.context.messages();
        // fetch_terminal 被执行，tool result 含 Context 全文
        assert!(msgs.iter().any(|m| m.role == Role::Tool
            && m.content.as_deref().unwrap_or("").contains(&"y".repeat(100))));
        // 最终 assistant 汇总（脚本原文）
        assert_eq!(msgs.last().unwrap().role, Role::Assistant);
        assert_eq!(msgs.last().unwrap().content.as_deref(), Some("[debug] 查到：全文"));
        let _ = std::fs::remove_dir_all(tmp_dir("fetch"));
    }

    #[tokio::test]
    async fn event_buffer_attached_on_release() {
        // 定案：放行 system 输入时 Event Buffer 附带合并为一条消息
        let mut ov = make_ambery("merge");
        ov.harness.event_buffer.push("用户关闭了 text_card「摘要」");
        ov.harness.event_buffer.push("用户勾选了 todobox 条目「跑测试」");
        ov.enqueue(Role::System, "ft 完成。评估是否通知。".into(), crate::queue::QueueSource::MockHook, 1)
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let sys: Vec<_> = ov
            .harness
            .context
            .messages()
            .iter()
            .filter(|m| m.role == Role::System)
            .collect();
        // 附带合并的一条（输入 + buffer，不独立成条）
        assert_eq!(sys.len(), 1);
        let merged = sys[0].content.as_deref().unwrap();
        assert!(merged.contains("ft 完成"));
        assert!(merged.contains("用户关闭了 text_card「摘要」"));
        assert!(merged.contains("用户勾选了 todobox 条目「跑测试」"));
        assert!(ov.harness.event_buffer.is_empty());
        let _ = std::fs::remove_dir_all(tmp_dir("merge"));
    }

    #[tokio::test]
    async fn event_buffer_keeps_user_role_clean() {
        // 定案（末句）：与 user role 严格分离——user 输入放行时
        // buffer 以独立 system 消息先行附带，不污染 user 消息
        let mut ov = make_ambery("merge-user");
        ov.harness.event_buffer.push("用户关闭了 text_card「摘要」");
        ov.enqueue(Role::User, "那个 bug 怎么回事？".into(), crate::queue::QueueSource::UserChat, 1)
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let msgs = ov.harness.context.messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::System);
        assert!(msgs[0]
            .content
            .as_deref()
            .unwrap()
            .contains("用户关闭了 text_card「摘要」"));
        assert_eq!(msgs[1].role, Role::User);
        assert_eq!(msgs[1].content.as_deref(), Some("那个 bug 怎么回事？"));
        let _ = std::fs::remove_dir_all(tmp_dir("merge-user"));
    }

    #[tokio::test]
    async fn plain_user_message_replies() {
        let agent = scripted(vec![say("[debug] 收到：你好")]);
        let mut ov = make_ambery_with("reply", agent);
        ov.harness
            .append_context(ContextMessage::new(Role::User, "你好", 1))
            .unwrap();
        ov.run_trigger(2, crate::queue::QueueSource::MockHook, 0).await.unwrap();
        let last = ov.harness.context.messages().last().unwrap();
        assert_eq!(last.role, Role::Assistant);
        assert_eq!(last.content.as_deref(), Some("[debug] 收到：你好"));
        let _ = std::fs::remove_dir_all(tmp_dir("reply"));
    }

    #[tokio::test]
    async fn hook_content_is_filtered_before_decision() {
        let mut ov = make_ambery("filter");
        // 原文很长但全是噪音 + 4 字内容 → 归一后 4 字 → 沉默
        let raw = format!(
            "● 完成\n✻ Crunched for 12s\n⏵⏵ bypass permissions on (shift+tab to cycle)\n{}",
            "─".repeat(100)
        );
        ov.handle_hook("stop", "ft", "proj", &raw, 1).await.unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        assert!(effects.is_empty());
        assert_eq!(ov.filtered_content_latest("ft").unwrap().filtered_content, "● 完成");
        let _ = std::fs::remove_dir_all(tmp_dir("filter"));
    }

    #[tokio::test]
    async fn timer_scan_substantive_notifies_and_records() {
        // mock 脚本：兜底触发后通知（call_component）→ 沉默
        //（session_start 定案后为静默簿记，不消耗脚本帧）
        let agent = scripted(vec![
            calls(vec![(
                "call_component",
                json!({"spec": {"id": "notify-cship", "type": "text_card", "title": "cship 有变化", "text": "去看看", "direction": "auto"}}),
            )]),
            silence(),
        ]);
        let mut ov = make_ambery_with("timer-sub", agent);
        ov.handle_hook("session_start", "cship", "proj", "旧内容", 1)
            .await
            .unwrap();
        // 兜底扫描读到全新长内容 → Substantive → 存 Context(timer) + 入队 → 放行触发通知
        let new_content = "z".repeat(150);
        ov.handle_timer_scan("cship", &new_content, 2)
            .await
            .unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        let rec = ov.filtered_content_latest("cship").unwrap();
        assert_eq!(rec.source, RecordSource::Timer);
        assert_eq!(rec.filtered_content, new_content);
        assert!(ov
            .harness
            .context
            .messages()
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("兜底扫描发现变化")));
        assert!(effects.iter().any(|e| matches!(e, Effect::RenderComponent(_))));
        let _ = std::fs::remove_dir_all(tmp_dir("timer-sub"));
    }

    #[tokio::test]
    async fn timer_scan_minor_stays_silent() {
        let mut ov = make_ambery("timer-min");
        ov.handle_hook("session_start", "cship", "proj", "内容不变", 1)
            .await
            .unwrap();
        let msgs_before = ov.harness.context.messages().len();
        // 内容相同 → Unchanged → 存档但不入队不打扰
        ov.handle_timer_scan("cship", "内容不变", 2)
            .await
            .unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        assert!(effects.is_empty());
        assert_eq!(ov.harness.context.messages().len(), msgs_before);
        assert_eq!(ov.filtered_content_latest("cship").unwrap().source, RecordSource::Timer);
        let _ = std::fs::remove_dir_all(tmp_dir("timer-min"));
    }

    #[tokio::test]
    async fn head_includes_agents_md() {
        let ov = make_ambery("head-md");
        let head = ov.assemble_system_prompt();
        // bootstrap 写入的默认身份提示词拼进了请求头（§12：Config 引用数据运行时拼装）
        assert!(head.contains("# AGENTS.md — Ambery"));
        assert!(head.contains("## 颜文字映射"));
        // 请求头只装稳定提示词：实例状态走 diff 事件，不进请求头
        assert!(!head.contains("## 当前实例状态"));
        let _ = std::fs::remove_dir_all(tmp_dir("head-md"));
    }

    #[tokio::test]
    async fn restart_loses_prev_first_scan_reports_change() {
        // filtered_content 不持久化定案：变化检测 prev 存内存，
        // 重启丢——同目录重开后首轮 scan 对相同内容也报 Substantive（接受的代价）
        let dir = tmp_dir("prev-loss");
        {
            let mut ov = make_ambery("prev-loss");
            // 先注册（kind=claude）：Filter 按实例 kind，未注册实例扫描内容被拒绝
            ov.handle_hook("session_start", "ft", "p", "", 0).await.unwrap();
            ov.handle_timer_scan("ft", "相同内容", 1).await.unwrap();
        }
        // 同目录重开（Harness replay + 新 AmberyBackend：prev 为空）
        let harness = Harness::load(&dir, &dir, 100_000, 9).unwrap();
        let mut ov = AmberyBackend::new(harness, Config::default(), DebugAgent::silent());
        ov.handle_timer_scan("ft", "相同内容", 2).await.unwrap();
        // 相同内容仍判 Substantive → 入队一条扫描注入（prev 丢失的直接证据）
        let injected = ov
            .harness
            .queue
            .iter()
            .any(|q| q.content.contains("兜底扫描发现变化"));
        assert!(injected, "重启后首轮 scan 应报变化（prev 内存态重启丢）");
        // 原文存档仍在盘（现算源不丢）
        assert!(ov.filtered_content_latest("ft").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn hook_archives_raw_before_filter() {
        let mut ov = make_ambery("raw-archive");
        // 原文含噪音 → terminal-content.jsonl 存 filter 前全文，context.jsonl 存归一后
        let raw = format!("● 完成\n✻ Crunched for 12s\n{}", "─".repeat(100));
        ov.handle_hook("stop", "ft", "proj", &raw, 1).await.unwrap();
        let archive = std::fs::read_to_string(
            ov.harness.storage_dir().join(crate::TERMINAL_CONTENT_FILE),
        )
        .unwrap();
        assert!(archive.contains("✻ Crunched for 12s")); // 原文噪音还在
        assert!(archive.contains("\"source\":\"hook\""));
        // 归一全文不持久化：从原文 digest 现算
        assert_eq!(ov.filtered_content_latest("ft").unwrap().filtered_content, "● 完成");
        let _ = std::fs::remove_dir_all(tmp_dir("raw-archive"));
    }

    /// 捕获帧 mock：记录每次 LLM 调用看到的完整请求内容
    fn capturing(
        frames: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    ) -> DebugAgent {
        DebugAgent::new(move |msgs| {
            frames.lock().unwrap().push(
                msgs.iter()
                    .map(|m| m.content.clone().unwrap_or_default())
                    .collect(),
            );
            silence()
        })
    }

    #[tokio::test]
    async fn autonomy_logged_and_appended_to_request_end() {
        let frames = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let mut ov = make_ambery_with("autonomy", capturing(frames.clone()));
        ov.run_trigger(1, crate::queue::QueueSource::MockHook, 0).await.unwrap();
        // 请求帧：首条 = 现拼请求头，末条 = Autonomy 状态
        let f = &frames.lock().unwrap()[0];
        assert!(f[0].contains("## 颜文字映射"));
        assert_eq!(f.last().unwrap(), "[face: idle, motion: still]");
        // 末端状态不落 Queue（内存视图无它）
        assert!(ov
            .harness
            .context
            .messages()
            .iter()
            .all(|m| m.content.as_deref() != Some("[face: idle, motion: still]")));
        // context.jsonl：autonomy 行每轮一条 + head 行
        let log = std::fs::read_to_string(ov.harness.storage_dir().join(crate::CONTEXT_FILE))
            .unwrap();
        assert!(log.contains("\"type\":\"autonomy\""));
        assert!(log.contains("[face: idle, motion: still]"));
        assert!(log.contains("\"type\":\"head\""));
        let _ = std::fs::remove_dir_all(tmp_dir("autonomy"));
    }

    #[tokio::test]
    async fn head_written_only_on_change() {
        let mut ov = make_ambery("head-diff");
        ov.run_trigger(1, crate::queue::QueueSource::MockHook, 0).await.unwrap();
        ov.run_trigger(2, crate::queue::QueueSource::MockHook, 0).await.unwrap();
        let storage = ov.harness.storage_dir().to_path_buf();
        let count = || {
            std::fs::read_to_string(storage.join(crate::CONTEXT_FILE))
                .unwrap()
                .matches("\"type\":\"head\"")
                .count()
        };
        assert_eq!(count(), 1); // 不变不写
        // AGENTS.md 热编辑 → 请求头变化 → 第二条 head 快照
        std::fs::write(ov.harness.config_dir().join(AGENTS_MD_FILE), "# 改过的 pet").unwrap();
        ov.run_trigger(3, crate::queue::QueueSource::MockHook, 0).await.unwrap();
        assert_eq!(count(), 2);
        let _ = std::fs::remove_dir_all(tmp_dir("head-diff"));
    }

    #[tokio::test]
    async fn pending_notifications_drives_notify_key() {
        let frames = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let mut ov = make_ambery_with("notify-key", capturing(frames.clone()));
        ov.run_trigger(1, crate::queue::QueueSource::MockHook, 2).await.unwrap();
        let f = &frames.lock().unwrap()[0];
        assert_eq!(f.last().unwrap(), "[face: notify, motion: bounce]");
        let _ = std::fs::remove_dir_all(tmp_dir("notify-key"));
    }

    #[tokio::test]
    async fn compression_logs_boundary_and_resyncs_panorama() {
        // 阈值 10 token：几条消息就触发 auto-compact（DebugAgent → summarize 回退确定性 stub）
        // #16 起触发上限来自 config（effective_compression_limit），不再是 Harness 构造参数
        let dir = tmp_dir("compact");
        let harness = Harness::load(&dir, &dir, 10, 0).unwrap();
        let mut config = Config::default();
        config.llm.active = "debug".into();
        config.llm.providers.insert("debug".into(), crate::config::LlmProvider {
            base_url: String::new(), model: String::new(), api_key_env: None, temperature: None,
            context_window: Some(10), compression_reserve: Some(0), effort_wire: None,
        });
        let mut ov = AmberyBackend::new(harness, config, DebugAgent::silent());
        ov.harness
            .upsert_agent(AgentEntry {
                hash: "h1".into(),
                name: "ft".into(),
                project: "p".into(),
                    kind: None,
                status: AgentStatus::Idle,
                    tab: None,
                first_seen: 0,
                last_seen: 0,
            })
            .unwrap();
        for i in 0..5 {
            ov.harness
                .append_context(ContextMessage::new(Role::User, format!("第 {i} 条消息内容内容"), i as i64))
                .unwrap();
        }
        ov.run_trigger(10, crate::queue::QueueSource::MockHook, 0).await.unwrap();
        let msgs = ov.harness.context.messages();
        // 内存视图：摘要为首条（shaking）
        assert!(msgs[0].content.as_deref().unwrap().starts_with("[历史摘要]"));
        // 归零重 diff：全景消息在摘要之后（压缩不丢实例认知）
        assert!(msgs
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("实例全景同步")));
        // compact_boundary 标记落盘（文件不删历史，可审计）
        let log =
            std::fs::read_to_string(ov.harness.storage_dir().join(crate::CONTEXT_FILE)).unwrap();
        assert!(log.contains("\"type\":\"compact_boundary\""));
        assert!(log.contains("\"pre_tokens\":"));
        assert!(log.contains("\"duration_ms\":"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tab_gone_marks_closed_and_exits_panorama() {
        let mut ov = make_ambery("closed");
        ov.handle_hook("session_start", "ft", "proj", "启动", 1)
            .await
            .unwrap();
        assert!(crate::panorama(&ov.harness.agents).is_some());
        // Timer 发现 tab 不复存在 → closed 终态，全景不再包含
        ov.mark_instance_closed("ft", 2).unwrap();
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Closed);
        assert!(crate::panorama(&ov.harness.agents).is_none());
        // 同名再注册 = 新生命周期（同名不同命，hash 不同）
        ov.handle_hook("session_start", "ft", "proj", "又开了", 3)
            .await
            .unwrap();
        assert_eq!(ov.harness.agents.len(), 2);
        assert_ne!(ov.harness.agents[0].hash, ov.harness.agents[1].hash);
        // stop 沿用最近一条未 closed 的生命周期
        ov.handle_hook("stop", "ft", "proj", "完成", 4).await.unwrap();
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Closed);
        assert_eq!(ov.harness.agents[1].status, AgentStatus::Idle);
        let _ = std::fs::remove_dir_all(tmp_dir("closed"));
    }

    #[tokio::test]
    async fn hook_resets_timer_wheel() {
        let mut ov = make_ambery("timer-reset");
        ov.handle_hook("session_start", "a", "proj", "x", 1000)
            .await
            .unwrap();
        // reset 后 due ≥ 1000 + interval（Config 默认 300s）
        assert!(ov.due_timer_scans(1000 + 100_000, 10).is_empty());
        assert_eq!(ov.due_timer_scans(1000 + 400_000, 10), vec!["a".to_string()]);
        let _ = std::fs::remove_dir_all(tmp_dir("timer-reset"));
    }

    #[tokio::test]
    async fn set_autonomy_face_key_resolves_to_body() {
        let mut ov = make_ambery("face-key");
        let call = ToolCall {
            id: "c1".into(),
            name: "set_autonomy".into(),
            // face 传 key 名：仅解析 face 本体；motion 缺省不连带（保持未覆盖）
            arguments: json!({ "key": "notify", "ttlMs": 3000 }).to_string(),
        };
        let (result, effects) = ov.execute_tool(&call).await;
        assert_eq!(result["ok"], json!(true));
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::SetAutonomy {
                face: Some(f),
                motion: None,
                ..
            } if f == "✧*｡٩(ˊᗜˋ*)و✧*｡"
        )));
        // 非 key 的 face 拒绝（必须用 key 名）
        let call2 = ToolCall {
            id: "c2".into(),
            name: "set_autonomy".into(),
            arguments: json!({ "key": "(・ω・)ノ" }).to_string(),
        };
        let (result2, _) = ov.execute_tool(&call2).await;
        assert_eq!(result2["ok"], json!(false));
        assert!(result2["error"].as_str().unwrap().contains("无效 key"));
        let _ = std::fs::remove_dir_all(tmp_dir("face-key"));
    }

    #[tokio::test]
    async fn set_autonomy_once_contract() {
        // once 契约：once 与 ttlMs 同传直接拒绝；单传 once 透传 effect
        let mut ov = make_ambery("once");
        let conflict = ToolCall {
            id: "c1".into(),
            name: "set_autonomy".into(),
            arguments: json!({ "motion": "bounce", "once": true, "ttlMs": 3000 }).to_string(),
        };
        let (r1, e1) = ov.execute_tool(&conflict).await;
        assert_eq!(r1["ok"], json!(false));
        assert!(r1["error"].as_str().unwrap().contains("不能同时传"));
        assert!(e1.is_empty());
        let ok = ToolCall {
            id: "c2".into(),
            name: "set_autonomy".into(),
            arguments: json!({ "motion": "shake", "once": true }).to_string(),
        };
        let (r2, e2) = ov.execute_tool(&ok).await;
        assert_eq!(r2["ok"], json!(true));
        assert!(e2.iter().any(|e| matches!(
            e,
            Effect::SetAutonomy { once: true, ttl_ms: None, motion: Some(m), .. } if m == "shake"
        )));
        let _ = std::fs::remove_dir_all(tmp_dir("once"));
    }

    #[tokio::test]
    async fn streaming_delta_flows_to_sink() {
        // 默认回落路径：complete 一次性 → 全文单 delta → AssistantDone
        let agent = scripted(vec![say("流式回复全文")]);
        let mut ov = make_ambery_with("stream", agent);
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Effect>::new()));
        let got2 = got.clone();
        ov.effect_sink = Some(std::sync::Arc::new(move |e: &Effect| {
            got2.lock().unwrap().push(e.clone());
        }));
        ov.enqueue(Role::User, "打个招呼".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        let got = got.lock().unwrap();
        // 完整回复作为单个 delta 到达 + Done 收尾；回复本体已写 Context
        assert!(got.iter().any(|e| matches!(
            e,
            Effect::AssistantDelta { content: Some(c), .. } if c == "流式回复全文"
        )));
        assert!(got.iter().any(|e| matches!(e, Effect::AssistantDone)));
        assert_eq!(
            ov.harness.context.messages().last().unwrap().content.as_deref(),
            Some("流式回复全文")
        );
        let _ = std::fs::remove_dir_all(tmp_dir("stream"));
    }

    /// 恒失败的 LLM（错误通道测试）：complete 即 Err，流式默认回落同路
    struct FailingLlm;
    impl crate::llm::Llm for FailingLlm {
        fn complete(
            &self,
            _messages: &[ContextMessage],
            _tools: &[crate::llm::ToolDef],
            _effort: Option<crate::llm::Effort>,
        ) -> impl std::future::Future<Output = Result<LlmOutput, String>> + Send {
            async { Err("boom".to_string()) }
        }
    }

    #[tokio::test]
    async fn llm_call_failure_surfaces_transient_error_and_done() {
        // LLM 调用失败：transient error 经 sink 即时下发 + 动作流落盘；
        // AssistantDone 照发（loading 收尾不变式），run_trigger 不炸
        let dir = tmp_dir("llm-fail");
        let harness = Harness::load(&dir, &dir, 100_000, 0).unwrap();
        let mut ov = AmberyBackend::new(harness, Config::default(), FailingLlm);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Effect>::new()));
        let seen2 = seen.clone();
        ov.effect_sink = Some(std::sync::Arc::new(move |e: &Effect| {
            seen2.lock().unwrap().push(e.clone());
        }));
        ov.enqueue(Role::User, "hi".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter().any(|e| matches!(e, Effect::Error { retention: ErrorRetention::Transient, message, .. } if message.contains("boom"))),
            "{seen:?}"
        );
        assert!(seen.iter().any(|e| matches!(e, Effect::AssistantDone)), "{seen:?}");
        // 错误走 sink 旁路即时下发，不进 effects Vec 等轮末
        assert!(!effects.iter().any(|e| matches!(e, Effect::Error { .. })), "{effects:?}");
        let recs = ov.harness.read_effects().unwrap();
        assert!(
            recs.iter().any(|r| r.kind == "error" && r.payload["retention"] == json!("transient")),
            "{recs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 动作流记录──

    #[test]
    fn effect_kind_payload_exhaustive() {
        // 穷尽 match 投影：全部变体都有 kind/payload（新增变体 = effect_kind_payload 编译错）
        let cases: Vec<(Effect, &str)> = vec![
            (Effect::RenderComponent(json!({"id":"c"})), "render_component"),
            (Effect::CloseComponent("c".into()), "close_component"),
            (
                Effect::SetAutonomy { face: None, motion: Some("bounce".into()), ttl_ms: None, once: false },
                "set_autonomy",
            ),
            (Effect::ConfigChanged { llm_changed: false }, "config_changed"),
            (
                Effect::Error { message: "boom".into(), retention: ErrorRetention::Persistent, action: Some("setup".into()) },
                "error",
            ),
            (
                Effect::AssistantDelta { content: Some("x".into()), reasoning_content: None },
                "assistant_delta",
            ),
            (Effect::AssistantDone, "assistant_done"),
        ];
        for (e, kind) in cases {
            let (k, payload) = e.effect_kind_payload();
            assert_eq!(k, kind);
            assert!(payload.is_object());
        }
        // error 载荷契约：retention 字符串形态 + action 可选携带
        let (_, p) = Effect::Error {
            message: "m".into(),
            retention: ErrorRetention::Persistent,
            action: Some("setup".into()),
        }
        .effect_kind_payload();
        assert_eq!(p["retention"], json!("persistent"));
        assert_eq!(p["action"], json!("setup"));
        let (_, p) = Effect::Error {
            message: "m".into(),
            retention: ErrorRetention::Transient,
            action: None,
        }
        .effect_kind_payload();
        assert_eq!(p["retention"], json!("transient"));
        assert!(p["action"].is_null());
    }

    #[tokio::test]
    async fn execute_tool_records_component_effects() {
        // execute_tool 记录点：render/close 进 effect.jsonl（backend origin）
        let mut ov = make_ambery("eff-rec");
        let create = ToolCall {
            id: "c1".into(),
            name: "call_component".into(),
            arguments: json!({ "spec": { "id": "card1", "type": "text_card", "title": "T", "text": "x" } }).to_string(),
        };
        ov.execute_tool(&create).await;
        let close = ToolCall {
            id: "c2".into(),
            name: "call_component".into(),
            arguments: json!({ "spec": { "id": "card1", "action": "close" } }).to_string(),
        };
        ov.execute_tool(&close).await;
        let recs = ov.harness.read_effects().unwrap();
        assert_eq!(recs.len(), 2, "{recs:?}");
        assert_eq!(recs[0].origin, crate::EffectOrigin::Backend);
        assert_eq!(recs[0].kind, "render_component");
        assert_eq!(recs[0].payload["spec"]["id"], json!("card1"));
        assert_eq!(recs[1].kind, "close_component");
        assert_eq!(recs[1].payload["id"], json!("card1"));
        // 行形态：{"type":"effect","origin":"backend",...}
        // （tmp_dir 辅助会清空目录，这里只拼路径不再调它）
        let dir = std::env::temp_dir().join(format!("ambery-test-eff-rec-{}", std::process::id()));
        let raw = std::fs::read_to_string(dir.join(crate::EFFECT_FILE)).unwrap();
        assert!(raw.contains("\"type\":\"effect\""));
        assert!(raw.contains("\"origin\":\"backend\""));
        let _ = std::fs::remove_dir_all(tmp_dir("eff-rec"));
    }

    #[tokio::test]
    async fn config_changed_recorded_once_via_tool() {
        // 单变体单记录点：经 execute_tool 的 edit_config 不双写（快照门禁走完整 query→update 协议）
        let mut ov = make_ambery("eff-cfg");
        let query = ToolCall {
            id: "q1".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "query", "path": "kaomoji.user", "view": "object" }).to_string(),
        };
        let (qr, _) = ov.execute_tool(&query).await;
        assert_eq!(qr["ok"], json!(true), "{qr}");
        ov.harness
            .append_context(ContextMessage::tool_result("q1", qr.to_string(), 1))
            .unwrap();
        ov.response_seq += 1;
        let update = ToolCall {
            id: "u1".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "update", "path": "kaomoji.user", "value": { "celebrate": { "face": "(≧▽≦)", "motion": "bounce" } } }).to_string(),
        };
        let (r, _) = ov.execute_tool(&update).await;
        assert_eq!(r["ok"], json!(true), "{r}");
        let recs = ov.harness.read_effects().unwrap();
        let n = recs.iter().filter(|r| r.kind == "config_changed").count();
        assert_eq!(n, 1, "{recs:?}");
        let rec = recs.iter().find(|r| r.kind == "config_changed").unwrap();
        assert_eq!(rec.payload["llm_changed"], json!(false));
        assert_eq!(rec.origin, crate::EffectOrigin::Backend);
        let _ = std::fs::remove_dir_all(tmp_dir("eff-cfg"));
    }

    #[test]
    fn frontend_effect_reporting_channel() {
        // record_frontend_effect（record_effect command / POST /effect 共用单点）
        let ov = make_ambery("eff-fe");
        ov.record_frontend_effect("window_opened", json!({ "window": "card-x" }));
        ov.record_frontend_effect("window_moved", json!({ "window": "card-x", "x": 1, "y": 2, "count": 7 }));
        let recs = ov.harness.read_effects().unwrap();
        assert_eq!(recs.len(), 2);
        assert!(recs.iter().all(|r| r.origin == crate::EffectOrigin::Frontend));
        assert_eq!(recs[0].kind, "window_opened");
        assert_eq!(recs[1].payload["count"], json!(7));
        let _ = std::fs::remove_dir_all(tmp_dir("eff-fe"));
    }

    #[tokio::test]
    async fn enqueue_user_records_user_message_once() {
        // user 入队 = user_message/frontend（单点覆盖端点与 case user step）；system 输入不进
        let mut ov = make_ambery("eff-user");
        ov.enqueue(Role::User, "你好".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.enqueue(Role::System, "hook 输入".into(), crate::queue::QueueSource::MockHook, 2).unwrap();
        let recs = ov.harness.read_effects().unwrap();
        let user_recs: Vec<_> = recs.iter().filter(|r| r.kind == "user_message").collect();
        assert_eq!(user_recs.len(), 1, "{recs:?}");
        assert_eq!(user_recs[0].origin, crate::EffectOrigin::Frontend);
        assert_eq!(user_recs[0].payload["text"], json!("你好"));
        let _ = std::fs::remove_dir_all(tmp_dir("eff-user"));
    }

    #[tokio::test]
    async fn streaming_records_delta_and_done() {
        // run_trigger sink 记录点：delta 全量 + done 收尾都进 effect.jsonl
        let agent = scripted(vec![say("回复全文")]);
        let mut ov = make_ambery_with("eff-stream", agent);
        ov.enqueue(Role::User, "问".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        let recs = ov.harness.read_effects().unwrap();
        assert!(recs.iter().any(|r| r.kind == "assistant_delta"
            && r.payload["content"] == json!("回复全文")), "{recs:?}");
        assert!(recs.iter().any(|r| r.kind == "assistant_done"), "{recs:?}");
        let _ = std::fs::remove_dir_all(tmp_dir("eff-stream"));
    }

    #[tokio::test]
    async fn no_sink_no_delta_no_panic() {
        // 未接 sink 时流式路径静默无副作用（debug/测试模式默认）
        let agent = scripted(vec![say("你好")]);
        let mut ov = make_ambery_with("stream-none", agent);
        ov.enqueue(Role::User, "hi".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        assert_eq!(
            ov.harness.context.messages().last().unwrap().content.as_deref(),
            Some("你好")
        );
        let _ = std::fs::remove_dir_all(tmp_dir("stream-none"));
    }

    #[tokio::test]
    async fn call_component_continuous_management() {
        // 持续管理协议：同 id = 原地更新，close action 显式关闭
        let mut ov = make_ambery("cmp-mgmt");
        let mk = |text: &str| crate::context::ToolCall {
            id: "c1".into(),
            name: "call_component".into(),
            arguments: json!({"spec": {"id": "todo-1", "type": "todobox", "title": "t", "items": [{"text": text, "done": false}]}}).to_string(),
        };
        // 创建 → rendered + .card.json 落盘
        let (r1, e1) = ov.execute_tool(&mk("a")).await;
        assert_eq!(r1["rendered"], json!("todo-1"));
        assert!(matches!(e1[0], Effect::RenderComponent(_)));
        assert!(ov.harness.cards.contains_key("todo-1"));
        let card_file = ov.harness.cards_dir().join("todo-1.card.json");
        assert!(card_file.exists(), "创建即落盘 .card.json");
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&card_file).unwrap()).unwrap();
        assert_eq!(on_disk["component"]["items"][0]["text"], "a");
        // 同 id → updated（不再 toggle 关闭）；component 换、_meta 保留
        let (r2, e2) = ov.execute_tool(&mk("b")).await;
        assert_eq!(r2["updated"], json!("todo-1"));
        assert!(matches!(e2[0], Effect::RenderComponent(_)));
        assert!(ov.harness.cards.contains_key("todo-1"));
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&card_file).unwrap()).unwrap();
        assert_eq!(on_disk["component"]["items"][0]["text"], "b", "更新只换 component");
        assert_eq!(on_disk["_meta"]["user_closed"], false);
        // close action → closed + CloseComponent effect + dismiss 删文件
        let close_call = crate::context::ToolCall {
            id: "c2".into(),
            name: "call_component".into(),
            arguments: json!({"spec": {"id": "todo-1", "type": "todobox", "action": "close"}}).to_string(),
        };
        let (r3, e3) = ov.execute_tool(&close_call).await;
        assert_eq!(r3["closed"], json!("todo-1"));
        assert!(matches!(e3[0], Effect::CloseComponent(_)));
        assert!(!ov.harness.cards.contains_key("todo-1"));
        assert!(!card_file.exists(), "dismiss 删 .card.json");
        // 生命周期事件：created 一行 + closed 一行，均进 EventBuffer 静默簿记
        let owned = ov.harness.event_buffer.events();
        let events: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
        assert!(events.iter().any(|l| l.starts_with("card created: todobox「t」(todo-1) @ ") && l.ends_with(", → 存活 1")), "created 事件: {events:?}");
        assert!(events.iter().any(|l| l.starts_with("card closed: todobox「t」(todo-1), ") && l.contains(" / ") && l.ends_with(", → 存活 0")), "closed 事件: {events:?}");
        let _ = std::fs::remove_dir_all(tmp_dir("cmp-mgmt"));
    }

    #[tokio::test]
    async fn call_component_close_action_outside_spec() {
        // #23：LLM 把 action="close" 放在 args 顶层（与 spec 并列）时，
        // 回退识别为 close，而不是当成空 update 渲染空卡
        let mut ov = make_ambery("cmp-close-outside");
        let create = crate::context::ToolCall {
            id: "c1".into(),
            name: "call_component".into(),
            arguments: json!({"spec": {"id": "demo_line", "type": "text_card", "title": "t", "text": "x"}}).to_string(),
        };
        let (r1, _) = ov.execute_tool(&create).await;
        assert_eq!(r1["rendered"], json!("demo_line"));
        let close = crate::context::ToolCall {
            id: "c2".into(),
            name: "call_component".into(),
            arguments: json!({"action": "close", "spec": {"id": "demo_line"}}).to_string(),
        };
        let (r2, e2) = ov.execute_tool(&close).await;
        assert_eq!(r2["closed"], json!("demo_line"));
        assert!(matches!(e2[0], Effect::CloseComponent(_)));
        assert!(!ov.harness.cards.contains_key("demo_line"));
        let _ = std::fs::remove_dir_all(tmp_dir("cmp-close-outside"));
    }

    #[tokio::test]
    async fn call_component_registry_restored_from_card_files() {
        // Card 跨重启：文件即真相——新 Harness 从
        // memory/cards/*.card.json 恢复注册表，不经 effect.jsonl replay
        let tag = "cmp-reload";
        let dir = tmp_dir(tag);
        {
            let harness = crate::Harness::load(&dir, &dir, 100_000, 0).unwrap();
            let mut ov = AmberyBackend::new(harness, Config::default(), DebugAgent::silent());
            let call = crate::context::ToolCall {
                id: "c1".into(),
                name: "call_component".into(),
                arguments: json!({"spec": {"id": "todo-1", "type": "todobox", "title": "清单", "items": [{"text": "a", "done": false}]}}).to_string(),
            };
            ov.execute_tool(&call).await;
            assert!(ov.harness.cards.contains_key("todo-1"));
        } // ov drop（模拟进程退出）
        // 第二次 load 不清目录 = 进程重启
        let harness2 = crate::Harness::load(&dir, &dir, 100_000, 0).unwrap();
        let ov2 = AmberyBackend::new(harness2, Config::default(), DebugAgent::silent());
        assert!(ov2.harness.cards.contains_key("todo-1"), "重启后注册表从文件恢复");
        assert_eq!(ov2.harness.cards["todo-1"].meta.title, "清单");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn plain_reply_keeps_reasoning_content_in_context() {
        // thinking 全保真：纯文本回复的 reasoning_content 也落 Context（记录≠回放）
        let agent = DebugAgent::new(|_| LlmOutput {
            content: Some("答案".into()),
            tool_calls: vec![],
            reasoning_content: Some("先想三步再答".into()),
            usage: None,
        });
        let mut ov = make_ambery_with("reason-keep", agent);
        ov.enqueue(Role::User, "问".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        let last = ov.harness.context.messages().last().unwrap();
        assert_eq!(last.role, Role::Assistant);
        assert_eq!(last.reasoning_content.as_deref(), Some("先想三步再答"));
        let _ = std::fs::remove_dir_all(tmp_dir("reason-keep"));
    }

    #[tokio::test]
    async fn compression_triggers_on_usage_truth() {

        // #16 真值触发：last_usage.prompt_tokens + est 增量 > 阈值 → 压缩
        let big = crate::llm::Usage { prompt_tokens: 900_000, completion_tokens: 0 };
        let agent = DebugAgent::new(move |_| LlmOutput {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
            usage: Some(big),
        });
        let mut ov = make_ambery_with("compress-truth", agent);
        ov.config.llm.active = "debug".into();
        ov.config.llm.providers.insert("debug".into(), crate::config::LlmProvider {
            base_url: String::new(), model: String::new(), api_key_env: None, temperature: None,
            context_window: Some(100), compression_reserve: Some(0), effort_wire: None,
        });
        // 第一轮：last_usage 还是 None → est 兜底不触发；当轮落真值
        ov.enqueue(Role::User, "第一轮".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        assert_eq!(ov.harness.last_usage, Some(big));
        // 第二轮：真值 900K + 增量 ≫ 100 → 触发压缩
        ov.enqueue(Role::User, "第二轮".into(), crate::queue::QueueSource::UserChat, 2).unwrap();
        ov.drain_queue(0).await.unwrap();
        let first = ov.harness.context.messages()[0]
            .content
            .as_deref()
            .unwrap_or("");
        assert!(first.contains("[历史摘要]"), "应触发压缩: {first}");
        let _ = std::fs::remove_dir_all(tmp_dir("compress-truth"));
    }

    #[tokio::test]
    async fn compression_triggers_on_est_fallback_without_usage() {
        // #16 兜底：DebugAgent 默认无 usage → 全量 est 触发（现状路径）
        let mut ov = make_ambery("compress-est");
        ov.config.llm.active = "debug".into();
        ov.config.llm.providers.insert("debug".into(), crate::config::LlmProvider {
            base_url: String::new(), model: String::new(), api_key_env: None, temperature: None,
            context_window: Some(50), compression_reserve: Some(0), effort_wire: None,
        });
        for i in 0..30 {
            ov.enqueue(Role::User, format!("第 {i} 条消息内容内容内容"), crate::queue::QueueSource::UserChat, i as i64)
                .unwrap();
        }
        ov.drain_queue(0).await.unwrap();
        let first = ov.harness.context.messages()[0]
            .content
            .as_deref()
            .unwrap_or("");
        assert!(first.contains("[历史摘要]"), "est 兜底应触发压缩: {first}");
        let _ = std::fs::remove_dir_all(tmp_dir("compress-est"));
    }

    #[tokio::test]
    async fn real_hook_stop_three_modes() {
        let sid = "dddd3333-4444-5555";
        // B（默认 queue_only）：hint 形态
        let mut ov = make_ambery("rh5");
        let _ = ov
            .handle_real_hook("stop", sid, r"/tmp/p", Some("claude"), None, None, Some("修了 3 个文件"), 1000)
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let m = ov.harness.context.messages().last().unwrap().content.clone().unwrap();
        assert!(m.contains("完成：修了 3 个文件。评估是否通知"), "B: {m}");
        let _ = std::fs::remove_dir_all(tmp_dir("rh5"));

        // C（message）：汇报原文直达
        let mut ov = make_ambery("rh6");
        ov.config.stop_hook_mode = "message".into();
        let _ = ov
            .handle_real_hook("stop", sid, r"/tmp/p", Some("claude"), None, None, Some("修了 3 个文件"), 1000)
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let m = ov.harness.context.messages().last().unwrap().content.clone().unwrap();
        assert_eq!(m, "[汇报] p·dddd3333 完成：修了 3 个文件");
        let _ = std::fs::remove_dir_all(tmp_dir("rh6"));

        // A（auto_read）：读通道全量,Context 更新
        let mut ov = make_ambery("rh7");
        ov.config.stop_hook_mode = "auto_read".into();
        // MapAdapter：只有正确实例名可定位可读（等价原闭包的 inst 断言）
        let map = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([(
            "p·dddd3333".to_string(),
            "● 完成。hooks 已配置".to_string(),
        )])));
        ov.terminal = Some(std::sync::Arc::new(crate::terminal::MapAdapter::new(map)));
        let _ = ov
            .handle_real_hook("stop", sid, r"/tmp/p", Some("claude"), None, None, None, 1000)
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let m = ov.harness.context.messages().last().unwrap().content.clone().unwrap();
        assert!(m.contains("Context 已更新"), "A: {m}");
        let ctx = ov.filtered_content_latest("p·dddd3333").expect("归一全文现算");
        assert!(ctx.filtered_content.contains("hooks 已配置"));
        let _ = std::fs::remove_dir_all(tmp_dir("rh7"));
    }

    #[tokio::test]
    async fn startup_sweep_full_flow() {
        let mut ov = make_ambery("sweep");
        // 预置一具占位尸体（标题已消失）
        ov.harness
            .upsert_agent(AgentEntry {
                hash: "uia:gone".into(),
                name: "gone".into(),
                project: "unknown".into(),
                kind: None,
                status: AgentStatus::Idle,
                tab: None,
                first_seen: 0,
                last_seen: 0,
            })
            .unwrap();
        let call = |req: &Value| -> Option<Value> {
            match req["cmd"].as_str()? {
                "list_windows" => Some(json!({"windows":[
                    {"hwnd":100,"title":"✳ npc-prof·3f8a2c1e","cloaked":false},
                    {"hwnd":200,"title":"✳ gumtree","cloaked":false},
                    {"hwnd":300,"title":"✳ 别的桌面·aaaa0000","cloaked":true},
                    {"hwnd":400,"title":"Neovim","cloaked":false}
                ]})),
                "list_tabs" => match req["hwnd"].as_i64()? {
                    100 => Some(json!({"tabs":[{"index":2,"name":"✳ npc-prof·3f8a2c1e","selected":true}]})),
                    200 => Some(json!({"tabs":[{"index":0,"name":"✳ gumtree","selected":false}]})),
                    _ => None,
                },
                "count_processes" => Some(json!({"count":54})),
                _ => None,
            }
        };
        ov.startup_sweep(&call, 1000).await.unwrap();
        // marker 注册（带 tab 定位 + kind）
        let a = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "3f8a2c1e")
            .expect("marker 注册");
        assert_eq!(a.name, "npc-prof·3f8a2c1e");
        assert_eq!(a.tab, Some(crate::TabRef { hwnd: 100, index: 2 }));
        assert_eq!(a.kind.as_deref(), Some("claude"));
        // 占位入册
        let ph = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "uia:gumtree")
            .expect("占位入册");
        assert_eq!(ph.kind.as_deref(), Some("claude"));
        // cloaked 窗口标题带 marker:注册但无 tab 定位
        let c = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "aaaa0000")
            .expect("cloaked 注册");
        assert_eq!(c.tab, None);
        // 占位尸体 closed
        let g = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "uia:gone")
            .unwrap();
        assert_eq!(g.status, AgentStatus::Closed);
        // 对账行进 EventBuffer
        let line = ov.harness.event_buffer.merge_and_clear().unwrap_or_default();
        assert!(line.contains("54") && line.contains("cloaked"), "{line}");
        let _ = std::fs::remove_dir_all(tmp_dir("sweep"));
    }

    #[tokio::test]
    async fn fetch_terminal_vd_switch_semantics() {
        // 必填:忘传报错教学
        let mut ov = make_ambery("vd1");
        let call = ToolCall { id: "c1".into(), name: "fetch_terminal".into(), arguments: json!({"instance":"x"}).to_string() };
        let (r, _) = ov.execute_tool(&call).await;
        assert!(r["error"].as_str().unwrap_or("").contains("vd_switch 必填"), "{r}");
        // false 且读不到:报错含重试提示
        let call2 = ToolCall { id: "c2".into(), name: "fetch_terminal".into(), arguments: json!({"instance":"x","vd_switch":false}).to_string() };
        let (r2, _) = ov.execute_tool(&call2).await;
        assert!(r2["error"].as_str().unwrap_or("").contains("vd_switch=true 重试"), "{r2}");
        let _ = std::fs::remove_dir_all(tmp_dir("vd1"));

        // true + 切换成功:重读命中（先注册实例——Filter 按实例 kind，未注册读取被拒）
        let mut ov = make_ambery("vd2");
        ov.handle_real_hook("session_start", "x0x00000-1", "/tmp/p", Some("claude"), None, None, None, 1)
            .await
            .unwrap();
        let inst_name = ov.harness.agents[0].name.clone();
        // cloaked 语义：可 join（拿 hwnd）但读不到；切换后读命中
        struct CloakedAdapter {
            readable: std::sync::Arc<std::sync::Mutex<bool>>,
            tab: crate::TabRef,
            title: String,
        }
        impl crate::terminal::TerminalAdapter for CloakedAdapter {
            fn enumerate(&self) -> Option<Vec<crate::terminal::TabInfo>> {
                Some(vec![crate::terminal::TabInfo {
                    tab: self.tab,
                    title: Some(self.title.clone()),
                    cwd: None,
                    command: None,
                    focused: None,
                    extras: Default::default(),
                }])
            }
            fn read(&self, _tab: &crate::TabRef) -> crate::terminal::ReadOutcome {
                if self.readable.lock().unwrap().clone() {
                    crate::terminal::ReadOutcome::Content("内容".to_string())
                } else {
                    crate::terminal::ReadOutcome::Error("cloaked".into())
                }
            }
        }
        struct SwitchUnlocks(std::sync::Arc<std::sync::Mutex<bool>>);
        impl crate::terminal::PlatformPrimitives for SwitchUnlocks {
            fn switch_vd(&self, _hwnd: i64) -> bool {
                *self.0.lock().unwrap() = true;
                true
            }
        }
        let readable = std::sync::Arc::new(std::sync::Mutex::new(false));
        ov.terminal = Some(std::sync::Arc::new(CloakedAdapter {
            readable: readable.clone(),
            tab: crate::TabRef { hwnd: 7, index: 0 },
            title: inst_name.clone(),
        }));
        ov.primitives = Some(std::sync::Arc::new(SwitchUnlocks(readable)));
        let call3 = ToolCall { id: "c3".into(), name: "fetch_terminal".into(), arguments: json!({"instance":inst_name,"vd_switch":true}).to_string() };
        let (r3, _) = ov.execute_tool(&call3).await;
        assert_eq!(r3["content"].as_str().unwrap_or(""), "内容");
        let _ = std::fs::remove_dir_all(tmp_dir("vd2"));
    }

    #[tokio::test]
    async fn real_hook_first_sight_registers_silently() {
        let mut ov = make_ambery("rh1");
        ov.handle_real_hook(
                "session_start",
                "3f8a2c1e-9b7d-4e5f-a6c1-02d4e6f8a9b0",
                r"/tmp/p",
                Some("claude"),
                None,
                None,
                None,
                1000,
            )
            .await
            .unwrap();
        // 静默：只入 EventBuffer 簿记，不触发 LLM
        let a = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "3f8a2c1e")
            .expect("已注册");
        assert_eq!(a.name, "p·3f8a2c1e");
        assert_eq!(a.kind.as_deref(), Some("claude"));
        assert_eq!(a.status, AgentStatus::Idle);
        assert_eq!(a.first_seen, 1000);
        assert!(ov.harness.context.messages().is_empty()); // 无 queue 注入
        let _ = std::fs::remove_dir_all(tmp_dir("rh1"));
    }

    #[tokio::test]
    async fn real_hook_late_start_self_heals() {
        let mut ov = make_ambery("rh2");
        // backend 当时不在线,start 丢失:初见恰好是 stop(register-on-first-sight)
        let _ = ov
            .handle_real_hook(
                "stop",
                "aaaa0000-1111-2222",
                r"/tmp/p",
                Some("claude"),
                None,
                None,
                Some("修完了"),
                2000,
            )
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let a = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "aaaa0000")
            .expect("自愈注册");
        assert_eq!(a.status, AgentStatus::Idle);
        assert_eq!(a.first_seen, 2000); // first_seen = 后端初见时刻
        assert!(ov
            .harness
            .context
            .messages()
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("修完了")));
        let _ = std::fs::remove_dir_all(tmp_dir("rh2"));
    }

    #[tokio::test]
    async fn real_hook_resume_upserts_no_duplicate() {
        let mut ov = make_ambery("rh3");
        for ts in [1000, 5000] {
            let _ = ov
                .handle_real_hook(
                    "session_start",
                    "bbbb1111-2222-3333",
                    r"/tmp/p",
                    Some("claude"),
                    None,
                    None,
                    None,
                    ts,
                )
                .await
                .unwrap();
        }
        assert_eq!(ov.harness.agents.len(), 1); // 同 sid8 自然 upsert
        assert_eq!(ov.harness.agents[0].first_seen, 1000); // first_seen 保留
        assert_eq!(ov.harness.agents[0].last_seen, 5000);
        let _ = std::fs::remove_dir_all(tmp_dir("rh3"));
    }

    #[tokio::test]
    async fn real_hook_prompt_processing_and_session_end_closed() {
        let mut ov = make_ambery("rh4");
        let sid = "cccc2222-3333-4444";
        let _ = ov
            .handle_real_hook("session_start", sid, r"/tmp/p", Some("claude"), None, None, None, 1000)
            .await
            .unwrap();
        let _ = ov
            .handle_real_hook("user_prompt", sid, r"/tmp/p", Some("claude"), Some("帮我修 bug"), None, None, 2000)
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let a = ov.harness.agents.iter().find(|a| a.hash == "cccc2222").unwrap();
        assert_eq!(a.status, AgentStatus::Processing); // 派活驱动
        assert!(ov
            .harness
            .context
            .messages()
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("[观察] 用户在")));
        let _ = ov
            .handle_real_hook("session_end", sid, r"/tmp/p", Some("claude"), None, None, None, 3000)
            .await
            .unwrap();
        let a = ov.harness.agents.iter().find(|a| a.hash == "cccc2222").unwrap();
        assert_eq!(a.status, AgentStatus::Closed); // 真信号终态
        assert_eq!(a.tab, None);
        let _ = std::fs::remove_dir_all(tmp_dir("rh4"));
    }

    #[tokio::test]
    async fn edit_config_updates_and_persists() {
        // 完整协议：query(view=object) 读整池 → 下一 response update 完整 map
        let mut ov = make_ambery("cfg");
        let query = ToolCall {
            id: "q1".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "query", "path": "kaomoji.user", "view": "object" }).to_string(),
        };
        let (qr, _) = ov.execute_tool(&query).await;
        assert_eq!(qr["ok"], json!(true), "{qr}");
        // query 的 tool result 入 Context（模拟 run_trigger 写回）+ 进入下一 response
        ov.harness
            .append_context(ContextMessage::tool_result("q1", qr.to_string(), 1))
            .unwrap();
        ov.response_seq += 1;
        let update = ToolCall {
            id: "u1".into(),
            name: "edit_config".into(),
            arguments: json!({
                "action": "update",
                "path": "kaomoji.user",
                "value": { "celebrate": { "face": "(≧▽≦)", "motion": "bounce" } }
            })
            .to_string(),
        };
        let (result, effects) = ov.execute_tool(&update).await;
        assert_eq!(result["ok"], json!(true), "{result}");
        assert_eq!(result["msg"], json!("已生效"));
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::ConfigChanged { llm_changed: false })));
        assert_eq!(ov.config.kaomoji.user["celebrate"].face, "(≧▽≦)");
        // config.json 已持久化
        let reloaded = Config::load_or_default(ov.harness.config_dir());
        assert_eq!(reloaded.kaomoji.user["celebrate"].motion, "bounce");
        let _ = std::fs::remove_dir_all(tmp_dir("cfg"));
    }

    #[tokio::test]
    async fn edit_config_oversize_object_query_leaves_no_snapshot() {
        // H1 回归：view=object 超 1KiB 护栏被拒时快照不入册——LLM 没拿到完整当前值，
        // update 门禁不得凭错误 result 放行
        let mut ov = make_ambery("oversize");
        // 本地管道充气 user 池使 view=object 超 1KiB
        for i in 0..8 {
            ov.apply_config_by_path(
                &format!("kaomoji.user.face-{i}"),
                json!({ "face": format!("(≧▽≦){}", "长".repeat(30)), "motion": "bounce" }),
            )
            .unwrap();
        }
        let q = ToolCall {
            id: "q1".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "query", "path": "kaomoji.user", "view": "object" }).to_string(),
        };
        let (qr, _) = ov.execute_tool(&q).await;
        assert_eq!(qr["ok"], json!(false), "{qr}");
        assert!(qr["error"].as_str().unwrap().contains("1 KiB"), "{qr}");
        // 错误 result 入 Context + 下一 response：update 仍须被拒（无有效快照）
        ov.harness
            .append_context(ContextMessage::tool_result("q1", qr.to_string(), 1))
            .unwrap();
        ov.response_seq += 1;
        let u = ToolCall {
            id: "u1".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "update", "path": "kaomoji.user", "value": {} }).to_string(),
        };
        let (ur, _) = ov.execute_tool(&u).await;
        assert_eq!(ur["ok"], json!(false), "{ur}");
        assert!(ur["error"].as_str().unwrap().contains("请先 query"));
        let _ = std::fs::remove_dir_all(tmp_dir("oversize"));
    }

    #[tokio::test]
    async fn edit_config_snapshot_gating() {
        let mut ov = make_ambery("snap");
        let upd = |id: &str| ToolCall {
            id: id.into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "update", "path": "view_scale", "value": 0.7 }).to_string(),
        };
        // ① 无快照 → 拒绝
        let (r, _) = ov.execute_tool(&upd("u1")).await;
        assert_eq!(r["ok"], json!(false));
        assert!(r["error"].as_str().unwrap().contains("请先 query"));
        // ② query 留快照，但同 response（seq 未推进）→ 仍拒绝
        let query = ToolCall {
            id: "q1".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "query", "path": "view_scale" }).to_string(),
        };
        let (qr, _) = ov.execute_tool(&query).await;
        assert_eq!(qr["ok"], json!(true));
        assert_eq!(qr["node"]["value"], json!(1.0));
        let (r2, _) = ov.execute_tool(&upd("u2")).await;
        assert_eq!(r2["ok"], json!(false), "同 response 快照不算数");
        // ③ result 入 Context + 下一 response → 放行
        ov.harness
            .append_context(ContextMessage::tool_result("q1", qr.to_string(), 1))
            .unwrap();
        ov.response_seq += 1;
        let (r3, _) = ov.execute_tool(&upd("u3")).await;
        assert_eq!(r3["ok"], json!(true), "{r3}");
        assert_eq!(ov.config.view_scale, 0.7);
        // ④ 成功写入使相交快照失效 → 再写被拒（需重新 query）
        let (r4, _) = ov.execute_tool(&upd("u4")).await;
        assert_eq!(r4["ok"], json!(false), "写入后快照已失效");
        // ⑤ 快照的 message id 不在 Context（如 compression 摇掉）→ 拒绝
        let query2 = ToolCall {
            id: "q2".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "query", "path": "view_scale" }).to_string(),
        };
        let (_qr2, _) = ov.execute_tool(&query2).await;
        ov.response_seq += 1; // q2 的 result 不入 Context
        let (r5, _) = ov.execute_tool(&upd("u5")).await;
        assert_eq!(r5["ok"], json!(false), "快照 message 不在 Context 应拒绝");
        let _ = std::fs::remove_dir_all(tmp_dir("snap"));
    }

    #[tokio::test]
    async fn tool_call_budgets_enforced_with_final_wrap_up() {
        // 工具调用预算
        let mk_agent = |scripts: Vec<LlmOutput>, counter: std::sync::Arc<std::sync::atomic::AtomicUsize>| {
            let rest = std::sync::Mutex::new(std::collections::VecDeque::from(scripts));
            DebugAgent::new(move |_| {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                rest.lock().unwrap().pop_front().unwrap_or_else(silence)
            })
        };
        let auto_calls = |n: usize| {
            let specs: Vec<(&str, Value)> = (0..n)
                .map(|i| ("set_autonomy", json!({ "motion": "still", "ttlMs": 1000 + i })))
                .collect();
            calls(specs)
        };

        // ① 单 response 预算：5 calls 预算 3 → 3 执行 + 2 失败 result（turn 未耗尽 → 继续）
        let agent = mk_agent(vec![auto_calls(5), say("收尾")], std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let dir = tmp_dir("budget-resp");
        let mut cfg = Config::default();
        cfg.max_tool_calls_in_one_response = 3;
        cfg.max_tool_calls_per_turn = 50;
        let harness = Harness::load(&dir, &dir, 100_000, 0).unwrap();
        let mut ov = AmberyBackend::new(harness, cfg, agent);
        ov.enqueue(Role::User, "x".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        let results: Vec<String> = ov.harness.context.messages().iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.content.clone()).collect();
        assert_eq!(results.iter().filter(|c| c.contains("\"ok\":true")).count(), 3, "{results:?}");
        assert_eq!(results.iter().filter(|c| c.contains("单 response")).count(), 2, "{results:?}");
        let _ = std::fs::remove_dir_all(dir);

        // ② turn 预算耗尽：空 tools 收尾请求一次最终文字回复，不开启新 turn
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = mk_agent(vec![auto_calls(3), auto_calls(3), say("最终回复")], counter.clone());
        let dir = tmp_dir("budget-turn");
        let mut cfg = Config::default();
        cfg.max_tool_calls_in_one_response = 10;
        cfg.max_tool_calls_per_turn = 4;
        let harness = Harness::load(&dir, &dir, 100_000, 0).unwrap();
        let mut ov = AmberyBackend::new(harness, cfg, agent);
        ov.enqueue(Role::User, "x".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        let msgs = ov.harness.context.messages();
        let results: Vec<String> = msgs.iter().filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.content.clone()).collect();
        assert_eq!(results.iter().filter(|c| c.contains("\"ok\":true")).count(), 4, "{results:?}");
        assert_eq!(results.iter().filter(|c| c.contains("本 turn")).count(), 2, "{results:?}");
        // 收尾回复已写 Context；LLM 恰好 3 次调用（2 带 tools + 1 收尾空 tools）
        let last = msgs.iter().rev().find(|m| m.role == Role::Assistant).unwrap();
        assert_eq!(last.content.as_deref(), Some("最终回复"));
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn cron_tools_and_sleep_via_execute_tool() {
        // cron_create/cron_delete/sleep
        let mut ov = make_ambery("crontool");
        let c = ToolCall {
            id: "c1".into(),
            name: "cron_create".into(),
            arguments: json!({"schedule": {"every_ms": 60000}, "message": "日报"}).to_string(),
        };
        let (r, _) = ov.execute_tool(&c).await;
        assert_eq!(r["ok"], json!(true), "{r}");
        let id = r["id"].as_str().unwrap().to_string();
        assert_eq!(ov.harness.cron.entries().len(), 1);
        // schedule 缺 → 拒绝；空 message → 拒绝
        let (r2, _) = ov.execute_tool(&ToolCall { id: "c2".into(), name: "cron_create".into(), arguments: json!({"message": "x"}).to_string() }).await;
        assert_eq!(r2["ok"], json!(false));
        // cron_delete：存在 → deleted；不存在 → error（无 list tool 提示）
        let (r3, _) = ov.execute_tool(&ToolCall { id: "c3".into(), name: "cron_delete".into(), arguments: json!({"id": id}).to_string() }).await;
        assert_eq!(r3["deleted"], json!(id));
        assert!(ov.harness.cron.entries().is_empty());
        let (r4, _) = ov.execute_tool(&ToolCall { id: "c4".into(), name: "cron_delete".into(), arguments: json!({"id": "nope"}).to_string() }).await;
        assert!(r4["error"].as_str().unwrap().contains("不存在"));
        // sleep：注册后经共享句柄到点唤醒（模拟 cron task 的 fire_due）
        let handle = ov.harness.cron.waiter_handle();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            handle.fire_due(crate::server::now_ms());
        });
        let t0 = std::time::Instant::now();
        let (r5, _) = ov.execute_tool(&ToolCall { id: "s1".into(), name: "sleep".into(), arguments: json!({"ms": 20}).to_string() }).await;
        assert_eq!(r5["slept_ms"], json!(20));
        assert!(t0.elapsed().as_millis() >= 20, "sleep 应延迟返回");
        // sleep 上限（300s 设计常量）
        let (r6, _) = ov.execute_tool(&ToolCall { id: "s2".into(), name: "sleep".into(), arguments: json!({"ms": 300001}).to_string() }).await;
        assert_eq!(r6["ok"], json!(false));
        let _ = std::fs::remove_dir_all(tmp_dir("crontool"));
    }

    #[tokio::test]
    async fn memory_tools_round_trip_via_execute_tool() {
        // read_memory/write_memory：write 必附 description；
        // 省略 name 读 index.md 导航；Memory 根在 storage 下持久化
        let mut ov = make_ambery("memtool");
        let w = ToolCall {
            id: "w1".into(),
            name: "write_memory".into(),
            arguments: json!({ "name": "work-preferences", "content": "# 偏好\n简洁", "description": "用户的工作偏好" }).to_string(),
        };
        let (rw, _) = ov.execute_tool(&w).await;
        assert_eq!(rw["ok"], json!(true), "{rw}");
        // 缺 description → 拒绝
        let w2 = ToolCall {
            id: "w2".into(),
            name: "write_memory".into(),
            arguments: json!({ "name": "x-note", "content": "x" }).to_string(),
        };
        let (rw2, _) = ov.execute_tool(&w2).await;
        assert_eq!(rw2["ok"], json!(false));
        // 省略 name 读 index（含刚写入条目）
        let r = ToolCall { id: "r1".into(), name: "read_memory".into(), arguments: json!({}).to_string() };
        let (rr, _) = ov.execute_tool(&r).await;
        assert_eq!(rr["ok"], json!(true));
        assert!(rr["content"].as_str().unwrap().contains("work-preferences"));
        // 读具体记忆
        let r2 = ToolCall { id: "r2".into(), name: "read_memory".into(), arguments: json!({ "name": "work-preferences" }).to_string() };
        let (rr2, _) = ov.execute_tool(&r2).await;
        assert!(rr2["content"].as_str().unwrap().contains("简洁"));
        let _ = std::fs::remove_dir_all(tmp_dir("memtool"));
    }

    #[tokio::test]
    async fn edit_config_full_protocol_via_run_trigger() {
        // 完整 agent 工具策略（先读后写必须走 run_trigger 完整链路——
        // query result 写 Context tool result → 下一 response update 凭快照放行）
        let agent = scripted(vec![
            calls(vec![("edit_config", json!({ "action": "query", "path": "view_scale" }))]),
            calls(vec![("edit_config", json!({ "action": "update", "path": "view_scale", "value": 0.5 }))]),
            say("已调整缩放"),
        ]);
        let mut ov = make_ambery_with("proto", agent);
        ov.enqueue(Role::User, "把缩放调到 0.5".into(), crate::queue::QueueSource::UserChat, 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        assert_eq!(ov.config.view_scale, 0.5);
        // Context 留痕：query 与 update 的 tool result 都在
        let msgs = ov.harness.context.messages();
        let results: Vec<&str> = msgs.iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.content.as_deref()).collect();
        assert!(results.iter().any(|c| c.contains("\"value\":1")), "{results:?}");
        assert!(results.iter().any(|c| c.contains("已生效")), "{results:?}");
        let _ = std::fs::remove_dir_all(tmp_dir("proto"));
    }

    #[tokio::test]
    async fn edit_config_grep_and_query_views() {
        let mut ov = make_ambery("views");
        // grep：命中 path 与中文 desc；按 path 排序；不返回 value
        let grep = ToolCall {
            id: "g1".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "grep", "pattern": "badge|缩放" }).to_string(),
        };
        let (g, _) = ov.execute_tool(&grep).await;
        assert_eq!(g["ok"], json!(true), "{g}");
        let paths: Vec<&str> = g["matches"].as_array().unwrap().iter()
            .map(|m| m["path"].as_str().unwrap()).collect();
        assert!(paths.contains(&"badge_style") && paths.contains(&"badge_side") && paths.contains(&"view_scale"), "{paths:?}");
        // 按完整 path 字典序稳定排列
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted, "grep 结果须按 path 字典序: {paths:?}");
        assert!(g["matches"][0].get("value").is_none(), "grep 不返回 value");
        // 不可见子树不进 grep 结果
        assert!(!paths.iter().any(|p| p.starts_with("llm")), "{paths:?}");
        // grep 无匹配 = 成功空数组；非法 regex = 错误
        let (g2, _) = ov.execute_tool(&ToolCall { id: "g2".into(), name: "edit_config".into(), arguments: json!({ "action": "grep", "pattern": "zzz-no-hit" }).to_string() }).await;
        assert_eq!(g2["matches"], json!([]));
        let (g3, _) = ov.execute_tool(&ToolCall { id: "g3".into(), name: "edit_config".into(), arguments: json!({ "action": "grep", "pattern": "([" }).to_string() }).await;
        assert_eq!(g3["ok"], json!(false));
        // query 容器 children 视图：叶子 child 带 value，容器 child 不带；不产生快照
        let (qc, _) = ov.execute_tool(&ToolCall { id: "q1".into(), name: "edit_config".into(), arguments: json!({ "action": "query", "path": "timer" }).to_string() }).await;
        assert_eq!(qc["ok"], json!(true), "{qc}");
        let kids = qc["children"].as_array().unwrap();
        assert!(kids.iter().any(|k| k["path"] == "timer.interval_ms" && k["value"].is_number()));
        // query 容器 view=object：完整 JSON 无 children + 留快照
        let (qo, _) = ov.execute_tool(&ToolCall { id: "q2".into(), name: "edit_config".into(), arguments: json!({ "action": "query", "path": "kaomoji.system", "view": "object" }).to_string() }).await;
        assert_eq!(qo["ok"], json!(true), "{qo}");
        assert!(qo["node"]["value"]["idle"]["face"].is_string());
        assert!(qo.get("children").is_none());
        // 叶子 view=object → 明确报错
        let (qe, _) = ov.execute_tool(&ToolCall { id: "q3".into(), name: "edit_config".into(), arguments: json!({ "action": "query", "path": "view_scale", "view": "object" }).to_string() }).await;
        assert_eq!(qe["ok"], json!(false));
        // 未知 path → 报错（提示先 grep）
        let (qu, _) = ov.execute_tool(&ToolCall { id: "q4".into(), name: "edit_config".into(), arguments: json!({ "action": "query", "path": "nope.x" }).to_string() }).await;
        assert!(qu["error"].as_str().unwrap().contains("未知 path"));
        let _ = std::fs::remove_dir_all(tmp_dir("views"));
    }

    #[tokio::test]
    async fn kaomoji_pools_invariants_enforced_on_update() {
        // 两池校验：写入管道原子拒绝违反不变量的 candidate
        let mut ov = make_ambery("pools");
        // ① 交集为空：user 池新增与 system 重复的 key → 拒绝
        assert!(ov
            .apply_config_by_path("kaomoji.user.idle", json!({"face": "x", "motion": "still"}))
            .is_err());
        // ② 基础 key 在并集：移除 system 池（整体替换为空）→ 拒绝
        assert!(ov.apply_config_by_path("kaomoji.system", json!({})).is_err());
        // 合法：基础 key 移到 user 池（单次整节点写入 = 原子移动，并集仍齐）→ 通过
        let mut pools = serde_json::to_value(&ov.config.kaomoji).unwrap();
        let idle = pools["system"].as_object().unwrap()["idle"].clone();
        pools["system"].as_object_mut().unwrap().remove("idle");
        pools["user"]
            .as_object_mut()
            .unwrap()
            .insert("idle".into(), idle);
        ov.apply_config_by_path("kaomoji", pools).unwrap();
        assert!(ov.config.kaomoji.user.contains_key("idle"));
        assert!(!ov.config.kaomoji.system.contains_key("idle"));
        // 并集解析不受池归属影响（移动后仍参与默认状态与按 key 解析）
        assert_eq!(ov.config.kaomoji_resolve("idle").unwrap().face, "(´ω`)");
        let _ = std::fs::remove_dir_all(tmp_dir("pools"));
    }

    #[test]
    fn apply_config_by_path_hot_apply_and_persist() {
        let mut ov = make_ambery("apply");
        let out = ov
            .apply_config_by_path("compression_reserve_default", json!(5000))
            .unwrap();
        assert!(out.restart_required.is_empty());
        assert!(!out.llm_changed);
        assert_eq!(ov.config.compression_reserve_default, 5000); // 热应用
        let reloaded = Config::load_or_default(ov.harness.config_dir());
        assert_eq!(reloaded.compression_reserve_default, 5000); // persist
        let _ = std::fs::remove_dir_all(tmp_dir("apply"));
    }

    #[test]
    fn apply_config_by_path_validates_and_reports_restart() {
        let mut ov = make_ambery("apply2");
        // serde 验证失败
        assert!(ov.apply_config_by_path("compression_reserve_default", json!("oops")).is_err());
        // 动态 enum 校验
        assert!(ov.apply_config_by_path("llm.active", json!("nonexist")).is_err());
        // 合法 active
        let out = ov.apply_config_by_path("llm.active", json!("deepseek")).unwrap();
        assert!(out.llm_changed);
        // 冷字段如实上报
        let out2 = ov.apply_config_by_path("timer.interval_ms", json!(60000)).unwrap();
        assert_eq!(out2.restart_required, vec!["timer.interval_ms".to_string()]);
        let _ = std::fs::remove_dir_all(tmp_dir("apply2"));
    }

    #[test]
    fn apply_config_by_path_readonly_rejected() {
        let mut ov = make_ambery("apply3");
        ov.config.read_only = true;
        assert!(ov.apply_config_by_path("compression_reserve_default", json!(1)).is_err());
        let _ = std::fs::remove_dir_all(tmp_dir("apply3"));
    }

    #[tokio::test]
    async fn edit_config_null_write_semantics() {
        // null 语义：叶子写 null = 回自身 default；
        // object/map/动态 entry 拒绝 null 更新
        let mut ov = make_ambery("null-write");
        ov.apply_config_by_path("view_scale", json!(0.7)).unwrap();
        ov.apply_config_by_path("view_scale", Value::Null).unwrap();
        assert_eq!(ov.config.view_scale, 1.0); // 回 default
        assert!(ov.apply_config_by_path("llm", Value::Null).is_err());
        assert!(ov.apply_config_by_path("kaomoji.user", Value::Null).is_err());
        assert!(ov.apply_config_by_path("kaomoji.user.celebrate", Value::Null).is_err());
        let _ = std::fs::remove_dir_all(tmp_dir("null-write"));
    }

    #[tokio::test]
    async fn edit_config_rejects_no_llm_visible_subtree() {
        // LLM 受限投影：llm 整棵子树对 edit_config 统一拒绝；
        // 本地管道（apply_config_by_path = CLI/面板入口）不受投影限制
        let mut ov = make_ambery("proj");
        for path in ["llm.active", "llm.providers.deepseek.model", "llm"] {
            let call = ToolCall {
                id: "c".into(),
                name: "edit_config".into(),
                arguments: json!({ "action": "update", "path": path, "value": "x" }).to_string(),
            };
            let (r, _) = ov.execute_tool(&call).await;
            assert_eq!(r["ok"], json!(false), "{path} 应被拒绝");
            assert!(r["error"].as_str().unwrap().contains("不可访问"), "{path}");
        }
        // query / grep 同样拒绝不可见子树
        let (rq, _) = ov.execute_tool(&ToolCall {
            id: "q".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "query", "path": "llm.active" }).to_string(),
        }).await;
        assert!(rq["error"].as_str().unwrap().contains("不可访问"));
        let (rg, _) = ov.execute_tool(&ToolCall {
            id: "g".into(),
            name: "edit_config".into(),
            arguments: json!({ "action": "grep", "pattern": "active|provider|base_url" }).to_string(),
        }).await;
        assert!(!rg["matches"].as_array().unwrap().iter()
            .any(|m| m["path"].as_str().unwrap().starts_with("llm")));
        // 本地入口同 path 可达（投影不改真值）
        assert!(ov.apply_config_by_path("llm.active", json!("deepseek")).is_ok());
        let _ = std::fs::remove_dir_all(tmp_dir("proj"));
    }
}
