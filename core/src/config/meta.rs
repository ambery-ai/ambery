//! 字段 metadata 注册表（docs/config.md「字段 metadata」实现形态）。
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
    /// 无法修复的 entry 修复结果为该 entry 不存在，其余 entry 保留（docs/config.md）
    Map { entry_probe: fn(&Value) -> Option<Value> },
}

/// Validation（docs/config.md）：校验最终当前 Config 是否允许生效。
/// Range 闭区间只挂数值节点；OneOf 严格 JSON 值相等；Func 收节点最终值返回 message。
pub enum Validation {
    Range { min: Option<f64>, max: Option<f64> },
    OneOf(&'static [&'static str]),
    Func(fn(&Value) -> Vec<String>),
}

pub struct NodeMeta {
    /// 完整点分 path（静态字段；map entry 不入表，由 Map 节点自身承载规则）
    pub path: &'static str,
    pub kind: NodeKind,
    pub validate: &'static [Validation],
    /// 该节点及整棵子树不进入 LLM 的 Config 投影（docs/config.md §反射与消费者投影）
    pub no_llm_visible: bool,
    /// 冷字段：写盘但保持当前运行行为，重启生效（docs/config.md §热/冷语义）。
    /// 待重启状态 = 保存值与启动快照不同（docs/config.md §待重启状态）
    pub cold: bool,
}

const V: &[Validation] = &[];

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

fn kaomoji_pools_func(v: &Value) -> Vec<String> {
    match serde_json::from_value::<super::KaomojiConfig>(v.clone()) {
        Ok(pools) => super::validate_kaomoji_pools(&pools),
        Err(e) => vec![format!("kaomoji 结构不合法：{e}")],
    }
}

/// descriptor tree 行为元数据（单源）。desc/类型见 config.rs 结构体 doc comment + schemars。
pub static NODES: &[NodeMeta] = &[
    NodeMeta { path: "kaomoji", kind: NodeKind::Object, validate: &[Validation::Func(kaomoji_pools_func)], no_llm_visible: false, cold: false },
    NodeMeta { path: "kaomoji.system", kind: NodeKind::Map { entry_probe: probe_kaomoji_entry }, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "kaomoji.user", kind: NodeKind::Map { entry_probe: probe_kaomoji_entry }, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "compression_reserve_default", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "set_autonomy_default_ttl_ms", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "filter_strategy", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "timer", kind: NodeKind::Object, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "timer.interval_ms", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: true },
    NodeMeta { path: "timer.stagger_ms", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: true },
    NodeMeta { path: "timer.tick_ms", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: true },
    NodeMeta { path: "timer.batch", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: true },
    NodeMeta { path: "stop_hook_mode", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "base_prompt", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "view_scale", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "badge_style", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "badge_side", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "llm", kind: NodeKind::Object, validate: V, no_llm_visible: true, cold: false },
    NodeMeta { path: "llm.active", kind: NodeKind::Leaf, validate: V, no_llm_visible: false, cold: false },
    NodeMeta { path: "llm.providers", kind: NodeKind::Map { entry_probe: probe_llm_provider }, validate: V, no_llm_visible: false, cold: false },
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
            other => vec![format!("{other:?} 不在合法候选 {opts:?} 中")],
        },
        Validation::Func(f) => f(&node_value.cloned().unwrap_or(Value::Null)),
    }
}

/// 在指定节点集合上执行 validators，返回 (path, message) 列表
/// （错误聚合：按完整 path 字典序、同 path 按 message 字典序，docs/config.md）
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

/// update 校验（docs/config.md）：目标节点子树的 validators → 祖先 validators，
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

/// load 校验（docs/config.md §启动载入）：没有单一目标，运行全部 validators
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
}
