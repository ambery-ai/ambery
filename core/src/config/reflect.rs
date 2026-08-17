//! 声明式 UI 反射：Config 类型 = UI 唯一声明源。
//! serde + schemars 即 Rust 的 type reflection：reflect() 泛型 walker 产出节点列表，
//! CLI / 托盘面板 / LLM tool 全是节点的薄渲染器——加字段零成本。

use schemars::schema::{InstanceType, Schema, SchemaObject, SingleOrVec};
use serde::Serialize;
use serde_json::Value;

use super::Config;

/// 反射节点：一个可渲染/可修改的配置项（map 会展开已有条目为子节点）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigNode {
    /// 点分路径："compression_reserve_default" / "llm.providers.deepseek.model"
    pub path: String,
    #[serde(rename = "type")]
    pub ty: NodeType,
    /// 来自 doc comment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    pub value: Value,
}

/// 节点类型（→ 控件机械映射：bool→开关，enum→单选组，int+range→滑块…）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeType {
    Bool,
    Int {
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Float {
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Str,
    /// enum 选项：schema 静态 enum 或 OPTIONS 注册表动态注入（如 llm.active）。
    /// struct variant：内部 tag 的 serde 限制，newtype 装不了 sequence
    Enum { options: Vec<String> },
    /// map<String, T>：节点本体 + 已有条目展开为子节点
    Map,
    /// 静态 object 容器（仅 LLM 投影树产出；descriptor tree 为所有容器保留可定位节点）
    Object,
    /// 暂不支持渲染的类型（array 等）：只读展示 JSON
    Other,
}

/// 泛型反射：对任意 T: Serialize + JsonSchema 走查 schema + 当前值
pub fn reflect<T: Serialize + schemars::JsonSchema>(value: &T) -> Vec<ConfigNode> {
    let root = schemars::schema_for!(T);
    let val = serde_json::to_value(value).unwrap_or(Value::Null);
    let mut out = Vec::new();
    walk(&root.schema, &root.definitions, &val, String::new(), &mut out, false);
    out
}

/// Config 专用入口：reflect() + OPTIONS 注册表动态 enum 注入 + meta 静态约束投影
/// （Range/OneOf 是 descriptor/reflect 输出的一部分，
///  供 CLI 与设置面板机械选择控件——后端仍是唯一放行者，此处只是同一真值的只读投影）
pub fn config_nodes(config: &Config) -> Vec<ConfigNode> {
    let mut nodes = reflect(config);
    for (path, f) in OPTIONS {
        if let Some(n) = nodes.iter_mut().find(|n| n.path == *path) {
            n.ty = NodeType::Enum { options: f(config) };
        }
    }
    apply_meta_constraints(&mut nodes);
    nodes
}

/// LLM 受限投影：完整 descriptor tree
/// （静态 object / map / map entry 容器全部保留可定位节点）按 no_llm_visible
/// 过滤——edit_config 的 grep / query 唯一数据源。本地 CLI/面板用 config_nodes。
pub fn config_nodes_llm(config: &Config) -> Vec<ConfigNode> {
    let root = schemars::schema_for!(Config);
    let val = serde_json::to_value(config).unwrap_or(Value::Null);
    let mut nodes = Vec::new();
    walk(&root.schema, &root.definitions, &val, String::new(), &mut nodes, true);
    nodes.retain(|n| crate::config::meta::llm_visible(&n.path));
    for (path, f) in OPTIONS {
        if let Some(n) = nodes.iter_mut().find(|n| n.path == *path) {
            n.ty = NodeType::Enum { options: f(config) };
        }
    }
    apply_meta_constraints(&mut nodes);
    nodes
}

/// meta 注册表的静态约束投影进节点（Range → min/max；OneOf → enum options）。
/// OPTIONS 动态 enum 优先（已覆盖者跳过 OneOf）；Func 不输出静态约束（提交后由统一
/// validation 返回其 message）
fn apply_meta_constraints(nodes: &mut [ConfigNode]) {
    for n in nodes.iter_mut() {
        let Some(meta) = crate::config::meta::node_meta(&n.path) else { continue };
        for v in meta.validate {
            match (v, &mut n.ty) {
                (crate::config::meta::Validation::Range { min, max }, NodeType::Int { min: m, max: x })
                | (crate::config::meta::Validation::Range { min, max }, NodeType::Float { min: m, max: x }) => {
                    *m = *min;
                    *x = *max;
                }
                (crate::config::meta::Validation::OneOf(opts), ty @ NodeType::Str) => {
                    *ty = NodeType::Enum { options: opts.iter().map(|s| s.to_string()).collect() };
                }
                _ => {}
            }
        }
    }
}

/// 动态 enum 校验：path 有 OPTIONS 提供者时返回当前合法选项（验证集中于此）
pub fn valid_options(config: &Config, path: &str) -> Option<Vec<String>> {
    OPTIONS
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, f)| f(config))
}

/// 动态 enum 注册表（唯二手工钩子之一）：path → 选项提供者
static OPTIONS: &[(&str, fn(&Config) -> Vec<String>)] = &[
    ("llm.active", |c| {
        let mut v = vec!["unconfigured".to_string(), "debug".to_string()];
        let mut keys: Vec<String> = c.llm.providers.keys().cloned().collect();
        keys.sort();
        v.extend(keys);
        v
    }),
    ("theme", |c| {
        // 合法主题名 = themes 的 key
        let mut keys: Vec<String> = c.themes.keys().cloned().collect();
        keys.sort();
        keys
    }),
];

/// 按点分路径写入 JSON Value（中间缺失自动建 object；撞到非 object 报错）。
/// 只搬数据不做验证——验证 = 调用方反序列化回 Config（serde 白送）。
pub fn set_by_path(root: &mut Value, path: &str, new_value: Value) -> Result<(), String> {
    let segs: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Err("empty path".into());
    }
    let mut cur = root;
    for seg in &segs[..segs.len() - 1] {
        if cur.get(seg).is_none() {
            cur[seg] = Value::Object(serde_json::Map::new());
        }
        let next = cur.get_mut(seg).unwrap();
        if !next.is_object() {
            return Err(format!("path {path}: {seg} 不是 object"));
        }
        cur = next;
    }
    let last = segs[segs.len() - 1];
    let obj = cur
        .as_object_mut()
        .ok_or_else(|| format!("path {path}: 终点父级不是 object"))?;
    obj.insert(last.to_string(), new_value);
    Ok(())
}

fn walk(
    schema: &SchemaObject,
    defs: &std::collections::BTreeMap<String, Schema>,
    value: &Value,
    path: String,
    out: &mut Vec<ConfigNode>,
    include_objects: bool,
) {
    // doc comment 挂在 allOf 包装层上， 目标层没有——两层都要找
    let desc = desc_of(schema).or_else(|| desc_of(resolve(schema, defs)));
    let schema = resolve(schema, defs);
    match primary_type(schema) {
        Some(InstanceType::Object) => {
            if let Some(obj) = &schema.object {
                if !obj.properties.is_empty() {
                    // 嵌套 struct：LLM 投影树保留容器节点（descriptor tree 全容器可定位），
                    // 本地 UI 不产节点（按 path 前缀分组呈现，不改变 descriptor tree）
                    if include_objects && !path.is_empty() {
                        out.push(ConfigNode {
                            path: path.clone(),
                            ty: NodeType::Object,
                            desc: desc.clone(),
                            value: value.clone(),
                        });
                    }
                    for (name, sub) in &obj.properties {
                        let child_val = value.get(name).cloned().unwrap_or(Value::Null);
                        if let Schema::Object(sub_obj) = sub {
                            walk(sub_obj, defs, &child_val, join(&path, name), out, include_objects);
                        }
                    }
                    return;
                }
                if obj.additional_properties.is_some() {
                    // map<String, T>：节点本体 + 已有条目展开
                    out.push(ConfigNode {
                        path: path.clone(),
                        ty: NodeType::Map,
                        desc: desc.clone(),
                        value: value.clone(),
                    });
                    let entry_schema = match obj.additional_properties.as_deref() {
                        Some(Schema::Object(o)) => Some(resolve(o, defs)),
                        _ => None,
                    };
                    if let (Some(es), Value::Object(map)) = (entry_schema, value) {
                        let mut keys: Vec<&String> = map.keys().collect();
                        keys.sort();
                        for k in keys {
                            walk(es, defs, &map[k], join(&path, k), out, include_objects);
                        }
                    }
                    return;
                }
            }
            out.push(leaf(path, NodeType::Other, desc, value));
        }
        Some(InstanceType::Boolean) => out.push(leaf(path, NodeType::Bool, desc, value)),
        Some(InstanceType::Integer) => {
            let (min, max) = range_of(schema);
            out.push(leaf(path, NodeType::Int { min, max }, desc, value))
        }
        Some(InstanceType::Number) => {
            let (min, max) = range_of(schema);
            out.push(leaf(path, NodeType::Float { min, max }, desc, value))
        }
        Some(InstanceType::String) => {
            let ty = match &schema.enum_values {
                Some(vals) => NodeType::Enum {
                    options: vals
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                },
                None => NodeType::Str,
            };
            out.push(leaf(path, ty, desc, value));
        }
        _ => out.push(leaf(path, NodeType::Other, desc, value)),
    }
}

/// 解析 $ref 到 definitions（schemars 0.8 嵌套 struct 默认走引用；
/// 带 doc comment 的属性会被包成 { allOf: [$ref] }，要再剥一层）
fn resolve<'a>(
    schema: &'a SchemaObject,
    defs: &'a std::collections::BTreeMap<String, Schema>,
) -> &'a SchemaObject {
    if let Some(r) = &schema.reference {
        let name = r.trim_start_matches("#/definitions/");
        if let Some(Schema::Object(target)) = defs.get(name) {
            return target;
        }
    }
    if let Some(sub) = &schema.subschemas {
        if let Some(all_of) = &sub.all_of {
            if let Some(Schema::Object(inner)) = all_of.first() {
                if inner.reference.is_some() {
                    return resolve(inner, defs);
                }
            }
        }
    }
    schema
}

/// 主类型：Option<T> 是 [T, Null] 两态，取非 Null 者
fn primary_type(schema: &SchemaObject) -> Option<InstanceType> {
    match schema.instance_type.as_ref()? {
        SingleOrVec::Single(t) => Some(**t),
        SingleOrVec::Vec(ts) => ts.iter().copied().find(|t| *t != InstanceType::Null),
    }
}

fn range_of(schema: &SchemaObject) -> (Option<f64>, Option<f64>) {
    match &schema.number {
        Some(nv) => (nv.minimum, nv.maximum),
        None => (None, None),
    }
}

fn desc_of(schema: &SchemaObject) -> Option<String> {
    schema
        .metadata
        .as_ref()
        .and_then(|m| m.description.clone())
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn leaf(path: String, ty: NodeType, desc: Option<String>, value: &Value) -> ConfigNode {
    ConfigNode {
        path,
        ty,
        desc,
        value: value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_cover_core_paths_and_types() {
        let nodes = config_nodes(&Config::default());
        let get = |p: &str| nodes.iter().find(|n| n.path == p).unwrap_or_else(|| panic!("缺节点 {p}"));
        assert!(matches!(get("compression_reserve_default").ty, NodeType::Int { .. }));
        assert!(matches!(get("view_scale").ty, NodeType::Float { .. }));
        assert!(matches!(get("base_prompt").ty, NodeType::Str));
        assert!(matches!(get("kaomoji.system").ty, NodeType::Map));
        assert!(matches!(get("kaomoji.user").ty, NodeType::Map));
        assert!(matches!(get("llm.providers").ty, NodeType::Map));
        // doc comment → desc
        assert!(get("compression_reserve_default").desc.is_some());
        // 当前值
        assert_eq!(get("compression_reserve_default").value, Value::from(10000));
    }

    #[test]
    fn map_entries_expanded_with_types() {
        let nodes = config_nodes(&Config::default());
        let get = |p: &str| nodes.iter().find(|n| n.path == p).unwrap_or_else(|| panic!("缺节点 {p}"));
        assert!(matches!(get("kaomoji.system.idle.face").ty, NodeType::Str));
        assert!(matches!(get("llm.providers.deepseek.base_url").ty, NodeType::Str));
        assert!(matches!(get("llm.providers.deepseek.temperature").ty, NodeType::Float { .. }));
    }

    #[test]
    fn meta_constraints_project_into_nodes() {
        // Range/OneOf 是 reflect 输出的只读投影
        let cfg = Config::default();
        let nodes = config_nodes(&cfg);
        let vs = nodes.iter().find(|n| n.path == "view_scale").unwrap();
        match &vs.ty {
            NodeType::Float { min, max } => {
                assert_eq!(*min, Some(0.2));
                assert_eq!(*max, Some(4.0));
            }
            other => panic!("view_scale 应为 Float: {other:?}"),
        }
        let tick = nodes.iter().find(|n| n.path == "timer.tick_ms").unwrap();
        match &tick.ty {
            NodeType::Int { min, .. } => assert_eq!(*min, Some(100.0)),
            other => panic!("timer.tick_ms 应为 Int: {other:?}"),
        }
        let lang = nodes.iter().find(|n| n.path == "ui_language").unwrap();
        match &lang.ty {
            NodeType::Enum { options } => assert_eq!(options, &["zh", "en"]),
            other => panic!("ui_language 应为 Enum: {other:?}"),
        }
        // OPTIONS 动态 enum 优先（theme 的 options = themes keys）
        let theme = nodes.iter().find(|n| n.path == "theme").unwrap();
        match &theme.ty {
            NodeType::Enum { options } => assert!(options.contains(&"dark".to_string())),
            other => panic!("theme 应为 Enum: {other:?}"),
        }
    }

    #[test]
    fn provider_keys_grammar_validated_on_update() {
        // 动态 key 在每次 update 时由统一 validation 检查
        let mut c = serde_json::to_value(Config::default()).unwrap();
        c["llm"]["providers"]["Bad Key"] = serde_json::json!({"base_url": "http://x", "model": "m"});
        let errs = crate::config::meta::validate_for_update(&c, "llm.providers.Bad Key");
        assert!(errs.iter().any(|(p, m)| p == "llm.providers" && m.contains("grammar")), "{errs:?}");
    }

    #[test]
    fn active_gets_dynamic_enum_options() {
        let nodes = config_nodes(&Config::default());
        let n = nodes.iter().find(|n| n.path == "llm.active").unwrap();
        match &n.ty {
            NodeType::Enum { options: opts } => {
                assert!(opts.contains(&"debug".to_string()));
                assert!(opts.contains(&"deepseek".to_string()));
            }
            other => panic!("llm.active 应为 Enum，实际 {other:?}"),
        }
    }

    #[test]
    fn set_by_path_writes_and_validates_via_deserialize() {
        let mut v = serde_json::to_value(Config::default()).unwrap();
        // 嵌套已有路径
        set_by_path(&mut v, "compression_reserve_default", Value::from(5000)).unwrap();
        // map 新 key 自动建 object
        set_by_path(&mut v, "llm.providers.local.base_url", Value::from("http://x")).unwrap();
        set_by_path(&mut v, "llm.providers.local.model", Value::from("m")).unwrap();
        let cfg: Config = serde_json::from_value(v).unwrap();
        assert_eq!(cfg.compression_reserve_default, 5000);
        assert_eq!(cfg.llm.providers["local"].base_url, "http://x");
    }

    #[test]
    fn set_by_path_rejects_non_object_traversal_and_bad_type_fails_validation() {
        let mut v = serde_json::to_value(Config::default()).unwrap();
        assert!(set_by_path(&mut v, "compression_reserve_default.x", Value::from(1)).is_err());
        // set_by_path 本身不验证类型，反序列化兜底
        set_by_path(&mut v, "compression_reserve_default", Value::from("oops")).unwrap();
        assert!(serde_json::from_value::<Config>(v).is_err());
    }
}
