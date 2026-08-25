//! 字段 metadata 注册表（「字段 metadata」实现形态）。
//!
//! 文档的目标语法是 `#[config(...)]` derive 在字段上共位标注；当前以声明式注册表
//! 承载同一份 descriptor tree——行为元数据（节点种类 / validation / no_llm_visible /
//! 冷字段）单源于此，loader（migrate）/ reflect / 统一写入管道 / LLM 投影共用。
//! 类型与 desc 仍由 serde + schemars 从结构体派生（reflect.rs），两处同源于 config.rs。

use serde_json::Value;

/// 节点种类（reconcile 逻辑链的结构知识：object 向下构造 / map 自带 default / 叶子兜底）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Leaf,
    Object,
    /// map<String, T>：entry 经 probe 反序列化修复（填 serde default、剔除未知 key）；
    /// 无法修复的 entry 修复结果为该 entry 不存在，其余 entry 保留。
    /// free_keys=false：key 走动态 grammar（小写字母开头 [a-z0-9_-]）；
    /// free_keys=true：自由文本 key（如 effort.keywords 的匹配关键词）——仅排除空串与
    /// 路径分隔符 '.'（含 '.' 的 key 无法经 path 寻址，无意义）
    Map { entry_probe: fn(&Value) -> Option<Value>, free_keys: bool },
}

/// Validation：校验最终当前 Config 是否允许生效。
/// Range 闭区间只挂数值节点；OneOf 严格 JSON 值相等；Func 收节点最终值返回 message。
pub enum Validation {
    Range { min: Option<f64>, max: Option<f64> },
    OneOf(&'static [&'static str]),
    Func(fn(&Value) -> Vec<String>),
}

/// Migration：历史 JSON → 当前 JSON 的纯、确定性、
/// 可重放映射。适用源版本由所属 range 表达；未命中显式 range = 隐式 Current。
/// Func/RenameWithFunc 可失败——任意 migration 失败统一 default 化该节点（migrate.rs）
pub enum Migration {
    /// 当前节点的 default
    Default,
    /// from 指定的完整旧点分 path 的值原样成为当前节点值
    Rename { from: &'static str },
    /// 当前节点在旧 JSON 中的同路径子树经 func 变换
    Func(fn(&Value) -> Result<Value, String>),
    /// from 完整旧点分 path 的值经 func 变换
    RenameWithFunc { from: &'static str, func: fn(&Value) -> Result<Value, String> },
}

pub struct NodeMeta {
    /// 完整点分 path（静态字段；map entry 不入表，由 Map 节点自身承载规则）
    pub path: &'static str,
    pub kind: NodeKind,
    pub validate: &'static [Validation],
    /// 该节点及整棵子树不进入 LLM 的 Config 投影
    pub no_llm_visible: bool,
    /// 冷字段：写盘但保持当前运行行为，重启生效。
    /// 待重启状态 = 保存值与启动快照不同
    pub cold: bool,
    /// 迁移表：(源版本 range, 规则) 稀疏映射；
    /// 显式 range 不得相交（同节点内、祖先与后代节点间；启动时 check_migrate_ranges 检查）
    pub migrate: &'static [(std::ops::RangeInclusive<u32>, Migration)],
}

const V: &[Validation] = &[];
/// 空迁移表（绝大多数节点：隐式 Current）
const M: &[(std::ops::RangeInclusive<u32>, Migration)] = &[];

fn probe_kaomoji_entry(v: &Value) -> Option<Value> {
    serde_json::from_value::<super::KaomojiEntry>(v.clone())
        .ok()
        .map(|e| serde_json::to_value(e).unwrap())
}

fn probe_llm_provider(v: &Value) -> Option<Value> {
    serde_json::from_value::<super::LlmProvider>(v.clone())
        .ok()
        .map(|p| serde_json::to_value(p).unwrap())
}

/// effort.keywords 条目：合法 Effort 档位字符串（low/medium/high）才通过
fn probe_effort_value(v: &Value) -> Option<Value> {
    serde_json::from_value::<crate::llm::Effort>(v.clone())
        .ok()
        .map(|e| serde_json::to_value(e).unwrap())
}

fn kaomoji_pools_func(v: &Value) -> Vec<String> {
    match serde_json::from_value::<super::KaomojiConfig>(v.clone()) {
        Ok(pools) => super::validate_kaomoji_pools(&pools),
        Err(e) => vec![format!("kaomoji 结构不合法：{e}")],
    }
}

fn probe_theme_value(v: &Value) -> Option<Value> {
    serde_json::from_value::<std::collections::HashMap<String, String>>(v.clone())
        .ok()
        .map(|t| serde_json::to_value(t).unwrap())
}

fn themes_func(v: &Value) -> Vec<String> {
    match serde_json::from_value::<std::collections::HashMap<String, std::collections::HashMap<String, String>>>(v.clone()) {
        Ok(themes) => super::validate_theme_table(&themes),
        Err(e) => vec![format!("themes 结构不合法：{e}")],
    }
}

/// v0..=1 扁平 kaomoji map → 两池（从 migrate.rs 步进表收编为
/// per-node 迁移规则）。以系统池 default 为底、旧条目覆盖同名 key（行为保持 + 天然满足
/// 基础 key 不变量）；已是两池形态原样返回（幂等防御）
fn migrate_kaomoji_v1(old: &Value) -> Result<Value, String> {
    if old.get("system").is_some() || old.get("user").is_some() {
        return Ok(old.clone());
    }
    let Value::Object(flat) = old else {
        return Err(format!("kaomoji 应为 object，实际：{}", old.to_string().chars().take(60).collect::<String>()));
    };
    let mut system = serde_json::to_value(super::KaomojiConfig::default().system).unwrap();
    let smap = system.as_object_mut().unwrap();
    for (k, v) in flat {
        smap.insert(k.clone(), v.clone());
    }
    Ok(serde_json::json!({ "system": system, "user": {} }))
}

/// 显式 range 不相交检查：
/// 同一节点表内 range 不重叠；任一祖先与后代节点的显式 range 不相交。
/// 启动时由 migrate::check 调用一次；测试覆盖
pub fn check_migrate_ranges() -> Result<(), String> {
    fn overlaps(a: &std::ops::RangeInclusive<u32>, b: &std::ops::RangeInclusive<u32>) -> bool {
        a.start() <= b.end() && b.start() <= a.end()
    }
    for n in NODES {
        let ranges: Vec<_> = n.migrate.iter().map(|(r, _)| r).collect();
        for i in 0..ranges.len() {
            for j in (i + 1)..ranges.len() {
                if overlaps(ranges[i], ranges[j]) {
                    return Err(format!("{} 迁移表 range 相交", n.path));
                }
            }
        }
    }
    // 祖先×后代：共同显式 range 相交即拒绝（path 前缀即父子）
    let explicit: Vec<(&'static NodeMeta, &std::ops::RangeInclusive<u32>)> = NODES
        .iter()
        .flat_map(|n| n.migrate.iter().map(move |(r, _)| (n, r)))
        .collect();
    for (i, (a, ra)) in explicit.iter().enumerate() {
        for (b, rb) in &explicit[i + 1..] {
            let related = a.path.starts_with(&format!("{}.", b.path))
                || b.path.starts_with(&format!("{}.", a.path));
            if related && overlaps(ra, rb) {
                return Err(format!("祖先/后代迁移 range 相交：{} × {}", a.path, b.path));
            }
        }
    }
    Ok(())
}

fn pet_name_func(v: &Value) -> Vec<String> {
    match v.as_str() {
        Some(s) => super::validate_pet_name(s),
        None => vec!["pet 名称应为字符串".into()],
    }
}

/// providers 动态 key grammar（动态 key 在每次
/// update 时由统一 validation 运行时检查——覆盖 update 路径，加载侧 reconcile 已有）
fn providers_keys_func(v: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    if let Some(obj) = v.as_object() {
        for k in obj.keys() {
            if !super::valid_dynamic_key(k) {
                errs.push(format!("provider key 不符合 path grammar（小写字母开头，仅小写字母/数字/_/-）：{k}"));
            }
        }
    }
    errs
}

/// descriptor tree 行为元数据（单源）。desc/类型见 config.rs 结构体 doc comment + schemars。
///
/// 迁移表语义：kaomoji v0..=1 扁平 map → 两池；timer.* v0..=2 四个
/// 扁平顶层字段 → timer 子树（逐字段 Rename 完整旧路径）。失败统一 default 化（migrate.rs）。
pub static NODES: &[NodeMeta] = &[
    NodeMeta { path: "kaomoji", kind: NodeKind::Object, validate: &[Validation::Func(kaomoji_pools_func)], no_llm_visible: false, cold: false, migrate: &[(0..=1, Migration::Func(migrate_kaomoji_v1))] },
    NodeMeta { path: "kaomoji.system", kind: NodeKind::Map { entry_probe: probe_kaomoji_entry, free_keys: false }, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "kaomoji.user", kind: NodeKind::Map { entry_probe: probe_kaomoji_entry, free_keys: false }, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "compression_reserve_default", kind: NodeKind::Leaf, validate: V, no_llm_visible: true, cold: false, migrate: M },
    NodeMeta { path: "set_autonomy_default_ttl_ms", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
    // filter_strategy 已退役（Filter 按实例 kind 选择；旧字段经 reconcile 剔除）
    NodeMeta { path: "timer", kind: NodeKind::Object, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "timer.interval_ms", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: true, migrate: &[(0..=2, Migration::Rename { from: "timer_interval_ms" })] },
    NodeMeta { path: "timer.stagger_ms", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: true, migrate: &[(0..=2, Migration::Rename { from: "timer_stagger_ms" })] },
    NodeMeta { path: "timer.tick_ms", kind: NodeKind::Leaf, validate: &[Validation::Range { min: Some(100.0), max: None }], no_llm_visible: false, cold: true, migrate: &[(0..=2, Migration::Rename { from: "timer_tick_ms" })] },
    NodeMeta { path: "timer.batch", kind: NodeKind::Leaf, validate: &[Validation::Range { min: Some(1.0), max: None }], no_llm_visible: false, cold: true, migrate: &[(0..=2, Migration::Rename { from: "timer_batch" })] },
    // terminal.adapter_*装配期生效 = 冷字段
    NodeMeta { path: "terminal", kind: NodeKind::Object, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "terminal.adapter_wt", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: true, migrate: M },
    NodeMeta { path: "terminal.adapter_zellij", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: true, migrate: M },
    NodeMeta { path: "stop_hook_mode", kind: NodeKind::Leaf, validate: V, no_llm_visible: true, cold: false, migrate: M },
    NodeMeta { path: "max_tool_calls_in_one_response", kind: NodeKind::Leaf, validate: &[Validation::Range { min: Some(1.0), max: None }], no_llm_visible: true, cold: true, migrate: M },
    NodeMeta { path: "max_tool_calls_per_turn", kind: NodeKind::Leaf, validate: &[Validation::Range { min: Some(1.0), max: None }], no_llm_visible: true, cold: true, migrate: M },
    NodeMeta { path: "base_prompt", kind: NodeKind::Leaf, validate: V, no_llm_visible: true, cold: false, migrate: M },
    NodeMeta { path: "view_scale", kind: NodeKind::Leaf, validate: &[Validation::Range { min: Some(0.2), max: Some(4.0) }], no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "badge_style", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "badge_side", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "theme", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "themes", kind: NodeKind::Map { entry_probe: probe_theme_value, free_keys: false }, validate: &[Validation::Func(themes_func)], no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "ui_language", kind: NodeKind::Leaf, validate: &[Validation::OneOf(&["zh", "en"])], no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "harness_language", kind: NodeKind::Leaf, validate: &[Validation::OneOf(&["zh", "en"])], no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "name", kind: NodeKind::Leaf, validate: &[Validation::Func(pet_name_func)], no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "context_compression_keep_recent_messages", kind: NodeKind::Leaf, validate: &[Validation::Range { min: Some(1.0), max: None }], no_llm_visible: false, cold: true, migrate: M },
    // effort.*档位映射与关键词表（热：每次 LLM 调用现读）
    NodeMeta { path: "effort", kind: NodeKind::Object, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "effort.user_chat", kind: NodeKind::Leaf, validate: &[Validation::OneOf(&["low", "medium", "high"])], no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "effort.hook_stop_content", kind: NodeKind::Leaf, validate: &[Validation::OneOf(&["low", "medium", "high"])], no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "effort.default", kind: NodeKind::Leaf, validate: &[Validation::OneOf(&["low", "medium", "high"])], no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "effort.keywords", kind: NodeKind::Map { entry_probe: probe_effort_value, free_keys: true }, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "llm", kind: NodeKind::Object, validate: V, no_llm_visible: true, cold: false, migrate: M },
    NodeMeta { path: "llm.active", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "llm.providers", kind: NodeKind::Map { entry_probe: probe_llm_provider, free_keys: false }, validate: &[Validation::Func(providers_keys_func)], no_llm_visible: false, cold: false, migrate: M },
    // ui.* 壳层行为（热字段：壳监听 config 事件即时应用；整棵子树不进 LLM 投影）
    NodeMeta { path: "ui", kind: NodeKind::Object, validate: V, no_llm_visible: true, cold: false, migrate: M },
    NodeMeta { path: "ui.topmost", kind: NodeKind::Object, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "ui.topmost.pet", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "ui.topmost.chat", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "ui.topmost.shelf", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
    NodeMeta { path: "ui.topmost.card", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false, migrate: M },
];

pub fn node_meta(path: &str) -> Option<&'static NodeMeta> {
    NODES.iter().find(|n| n.path == path)
}

/// path 是否进入 LLM 投影：任一 no_llm_visible 祖先（含自身）即排除
pub fn llm_visible(path: &str) -> bool {
    !NODES.iter().any(|n| {
        n.no_llm_visible && (path == n.path || path.starts_with(&format!("{}.", n.path)))
    })
}

/// 冷字段 path 列表
pub fn cold_paths() -> Vec<&'static str> {
    NODES.iter().filter(|n| n.cold).map(|n| n.path).collect()
}

/// 在 JSON 值上按点分 path 取值（不存在 → None）
pub fn value_at<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// 单个 validator 执行（Func 只返回 message，path 由本框架补）
fn run_one(v: &Validation, node_value: Option<&Value>) -> Vec<String> {
    match v {
        Validation::Range { min, max } => {
            let Some(n) = node_value.and_then(Value::as_f64) else {
                return vec!["应为数值".into()];
            };
            let mut errs = Vec::new();
            if let Some(lo) = min {
                if n < *lo {
                    errs.push(format!("{n} 小于下界 {lo}"));
                }
            }
            if let Some(hi) = max {
                if n > *hi {
                    errs.push(format!("{n} 大于上界 {hi}"));
                }
            }
            errs
        }
        Validation::OneOf(opts) => match node_value {
            Some(Value::String(s)) if opts.contains(&s.as_str()) => vec![],
            // 可选叶未设（缺省 None / 显式 null）合法——OneOf 只约束出现的值
            None | Some(Value::Null) => vec![],
            other => vec![format!("{other:?} 不在合法候选 {opts:?} 中")],
        },
        Validation::Func(f) => f(&node_value.cloned().unwrap_or(Value::Null)),
    }
}

/// 在指定节点集合上执行 validators，返回 (path, message) 列表
/// （错误聚合：按完整 path 字典序、同 path 按 message 字典序）
fn run_on(cfg: &Value, paths: &[&str]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for p in paths {
        let Some(meta) = node_meta(p) else { continue };
        for v in meta.validate {
            for msg in run_one(v, value_at(cfg, p)) {
                out.push((p.to_string(), msg));
            }
        }
    }
    out.sort();
    out
}

/// update 校验：目标节点子树的 validators → 祖先 validators，
/// 执行顺序子树→祖先；任一失败原子拒绝整次更新（调用方语义）。
pub fn validate_for_update(cfg: &Value, target: &str) -> Vec<(String, String)> {
    // 子树（含目标自身）：path == target 或 path 以 target. 开头
    let mut subtree: Vec<&str> = NODES
        .iter()
        .map(|n| n.path)
        .filter(|p| *p == target || p.starts_with(&format!("{target}.")))
        .collect();
    subtree.sort_unstable();
    // 祖先（近→远执行；聚合时统一排序）
    let mut ancestors: Vec<&str> = NODES
        .iter()
        .map(|n| n.path)
        .filter(|p| target.starts_with(&format!("{p}.")))
        .collect();
    ancestors.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let mut paths = subtree;
    paths.extend(ancestors);
    run_on(cfg, &paths)
}

/// load 校验：没有单一目标，运行全部 validators
pub fn validate_all(cfg: &Value) -> Vec<(String, String)> {
    let mut paths: Vec<&str> = NODES.iter().map(|n| n.path).collect();
    paths.sort_unstable();
    run_on(cfg, &paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_json() -> Value {
        serde_json::to_value(super::super::Config::default()).unwrap()
    }

    #[test]
    fn llm_subtree_excluded_from_projection() {
        assert!(!llm_visible("llm"));
        assert!(!llm_visible("llm.active"));
        assert!(!llm_visible("llm.providers.deepseek.model"));
        assert!(llm_visible("kaomoji.system.idle.face"));
        assert!(llm_visible("view_scale"));
    }

    #[test]
    fn update_runs_subtree_then_ancestors() {
        // 改 kaomoji.user 单条 → 祖先 kaomoji 的 pools Func 必须跑到
        let mut c = cfg_json();
        c["kaomoji"]["user"]["idle"] = serde_json::json!({"face": "x", "motion": "still"});
        let errs = validate_for_update(&c, "kaomoji.user.idle");
        assert!(errs.iter().any(|(p, m)| p == "kaomoji" && m.contains("重复")));
        // 无关子树不跑：改 view_scale 不报 kaomoji
        let errs2 = validate_for_update(&c, "view_scale");
        assert!(errs2.is_empty());
    }

    #[test]
    fn validate_all_catches_pool_violation() {
        let mut c = cfg_json();
        c["kaomoji"]["system"] = serde_json::json!({});
        c["kaomoji"]["user"] = serde_json::json!({"celebrate": {"face": "x", "motion": "bounce"}});
        let errs = validate_all(&c);
        assert!(errs.iter().any(|(p, _)| p == "kaomoji"));
    }

    #[test]
    fn range_validators_enforced_on_update_and_load() {
        // view_scale 越界 → update 校验拒绝（M3：热字段非法值直达前端链路被拦）
        let mut c = cfg_json();
        c["view_scale"] = serde_json::json!(0.0);
        assert!(validate_for_update(&c, "view_scale").iter().any(|(p, _)| p == "view_scale"));
        c["view_scale"] = serde_json::json!(0.5);
        assert!(validate_for_update(&c, "view_scale").is_empty());
        // timer.tick_ms=0 → load 全量校验捕获（防 tokio interval(0) panic）
        let mut c2 = cfg_json();
        c2["timer"]["tick_ms"] = serde_json::json!(0);
        assert!(validate_all(&c2).iter().any(|(p, _)| p == "timer.tick_ms"));
        // 预算字段下界
        let mut c3 = cfg_json();
        c3["max_tool_calls_per_turn"] = serde_json::json!(0);
        assert!(validate_all(&c3).iter().any(|(p, _)| p == "max_tool_calls_per_turn"));
    }

    #[test]
    fn pool_key_grammar_enforced() {
        // L7：两池非法 key（含空格/大写/点）被 Func 校验捕获（写入路径同拦）
        let mut c = cfg_json();
        c["kaomoji"]["user"]["Bad Key!"] = serde_json::json!({"face": "x", "motion": "still"});
        let errs = validate_for_update(&c, "kaomoji.user");
        assert!(errs.iter().any(|(p, m)| p == "kaomoji" && m.contains("grammar")), "{errs:?}");
    }
}
