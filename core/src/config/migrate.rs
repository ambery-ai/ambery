//! 版本与迁移加载管线（docs/config.md）：
//! 数字版本 + enum Migration 稀疏区间映射 + reconcile 字段级 default 兜底
//! + 对称备份（config.bak/config-v0NN.json）+ 降级只读报错。
//!
//! version 是文件级控制字段，不进 Config 结构体；
//! 未带 version 的文件 = v0（前版本号时代，absence 即标记）。

use serde_json::Value;
use std::path::{Path, PathBuf};

use super::{meta, Config, CONFIG_FILE};
use meta::NodeKind;

/// 当前 schema 代际（bump 规则：仅语义断裂 +1）
/// v2：kaomoji 扁平 map → 两池 {system, user}（docs/config.md §表情池）
/// v3：timer_* 扁平字段 → timer 子树（docs/timer.md 字段表）
pub const CURRENT_VERSION: u32 = 3;

/// 迁移步进表（累计应用）：target version → 变换；source version < target 的步
/// 全部按序应用。每步是纯、确定性、可重放的 JSON→JSON 映射（docs/config.md
/// §版本与 migration）；不读环境、网络、文件或其他运行时状态。
static STEPS: &[(u32, fn(Value) -> Value)] = &[
    (2, migrate_kaomoji_pools),
    (3, migrate_timer_subtree),
];

/// 从 from 版本累计步进到 current
fn migrate_steps(mut value: Value, from: u32) -> Value {
    for (target, f) in STEPS {
        if from < *target {
            value = f(value);
        }
    }
    value
}

/// v2→v3：timer_* 四个扁平字段收编为 timer 子树（docs/timer.md 字段表）。
/// 缺失字段交 reconcile 补 default；已是子树形态时不动（防御）。
fn migrate_timer_subtree(mut value: Value) -> Value {
    let Some(obj) = value.as_object_mut() else { return value };
    if obj.contains_key("timer") {
        return value;
    }
    let mut timer = serde_json::Map::new();
    for (old, new) in [
        ("timer_interval_ms", "interval_ms"),
        ("timer_stagger_ms", "stagger_ms"),
        ("timer_tick_ms", "tick_ms"),
        ("timer_batch", "batch"),
    ] {
        if let Some(v) = obj.remove(old) {
            timer.insert(new.into(), v);
        }
    }
    obj.insert("timer".into(), Value::Object(timer));
    value
}

/// v1→v2：旧扁平 kaomoji map 整体迁入 system 池（行为保持：旧表既是请求头表
/// 也是尺寸扫描来源，与 system 池职责一致），user 池空；用户随后可在面板移动。
/// 以系统池 default 为底、旧条目覆盖同名 key——用户自定义的基础表情保留，
/// 且迁移产物天然满足「基础 key ⊆ 并集」不变量（不被加载校验 default 化误伤）。
/// 无 kaomoji 字段或已是两池形态时不动（交 reconcile 补 default）。
fn migrate_kaomoji_pools(mut value: Value) -> Value {
    let Some(obj) = value.as_object_mut() else { return value };
    let Some(kaomoji) = obj.get("kaomoji").cloned() else { return value };
    if kaomoji.get("system").is_some() || kaomoji.get("user").is_some() {
        return value; // 已是两池形态（防御；正常 v0/v1 不会命中）
    }
    if let Value::Object(flat) = kaomoji {
        let mut system =
            serde_json::to_value(super::KaomojiConfig::default().system).unwrap();
        let smap = system.as_object_mut().unwrap();
        for (k, v) in flat {
            smap.insert(k, v);
        }
        obj.insert(
            "kaomoji".into(),
            serde_json::json!({ "system": system, "user": {} }),
        );
    }
    value
}

/// 加载入口（Config::load_or_default 的实现体）
pub fn load(dir: &Path) -> Config {
    let file = dir.join(CONFIG_FILE);
    let raw = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(_) => {
            let cfg = Config::default();
            let _ = cfg.save(dir);
            return cfg;
        }
    };
    let mut value: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            // 整份重生成仅此时发生；原文件照样备份
            let bak = backup_bytes(dir, "corrupt", raw.as_bytes());
            let mut cfg = Config::default();
            cfg.load_report = vec![format!(
                "config.json 无法解析（{e}），整份重生成；原文件备份于 {}",
                bak.display()
            )];
            flush_report(&cfg.load_report);
            let _ = cfg.save(dir);
            return cfg;
        }
    };
    // null 归一（docs/config.md §null = 缺失）：进入 migration / reconcile 前，
    // 递归移除所有 object 中值为 null 的 key（数组中的 null 不适用）
    normalize_nulls(&mut value);

    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(0); // absence 即 v0

    let mut report = Vec::new();
    match version.cmp(&CURRENT_VERSION) {
        std::cmp::Ordering::Equal => {
            let cfg = validate_and_repair(reconcile(value, &mut report));
            flush_report(&cfg.load_report);
            cfg
        }
        std::cmp::Ordering::Less => {
            report.push(format!("config v{version} → v{CURRENT_VERSION} 迁移（累计步进）"));
            value = migrate_steps(value, version);
            let mut cfg = validate_and_repair(reconcile(value, &mut report));
            let bak = backup_bytes(dir, &format!("v{version:04}"), raw.as_bytes());
            cfg.load_report
                .push(format!("原文件备份于 {}", bak.display()));
            flush_report(&cfg.load_report);
            let _ = cfg.save(dir); // 写回 = version 归一到 current
            cfg
        }
        std::cmp::Ordering::Greater => downgrade(dir, version, &raw),
    }
}

/// 加载期校验（docs/config.md：validator 失败不阻断启动——同一节点的全部
/// message 写入加载报告，该节点只 default 化一次，然后继续）。
/// validators 单源于 config/meta.rs 注册表（当前手写挂点，目标形态为字段 metadata）。
fn validate_and_repair(mut cfg: Config) -> Config {
    let value = serde_json::to_value(&cfg).unwrap_or(Value::Null);
    let errors = meta::validate_all(&value);
    if errors.is_empty() {
        return cfg;
    }
    for (p, msg) in &errors {
        cfg.load_report.push(format!("validate: {p} — {msg}"));
    }
    // 失败节点去重（已按 path 排序），逐节点 default 化
    let default_v = serde_json::to_value(Config::default()).unwrap();
    let mut repaired = value;
    let mut done: Vec<&str> = Vec::new();
    for (p, _) in &errors {
        if done.contains(&p.as_str()) {
            continue;
        }
        done.push(p);
        if let Some(dv) = meta::value_at(&default_v, p).cloned() {
            let _ = crate::config::reflect::set_by_path(&mut repaired, p, dv);
            cfg.load_report.push(format!("validate: {p} 节点 default 化"));
        }
    }
    match serde_json::from_value::<Config>(repaired) {
        Ok(mut fixed) => {
            fixed.load_report = std::mem::take(&mut cfg.load_report);
            fixed
        }
        Err(e) => {
            cfg.load_report
                .push(format!("validate: 修复后仍无法反序列化（{e}），整份回退 default"));
            let mut d = Config::default();
            d.load_report = std::mem::take(&mut cfg.load_report);
            d
        }
    }
}

/// 降级（file version > binary version）：对称备份新版现场后，
/// 只读加载 ≤ 自身版本的最大 bak；config.json 完全不碰；找不到 → 拒绝启动。
fn downgrade(dir: &Path, file_version: u32, raw: &str) -> Config {
    let bak_new = backup_bytes(dir, &format!("v{file_version:04}"), raw.as_bytes());
    let mut report = vec![format!(
        "检测到更新版本 config v{file_version}（本 binary v{CURRENT_VERSION}），新版现场已备份于 {}",
        bak_new.display()
    )];
    match best_backup(dir, CURRENT_VERSION) {
        Some((_, bak_path)) => {
            report.push(format!("降级只读模式：加载备份 {}", bak_path.display()));
            let s = std::fs::read_to_string(&bak_path).unwrap_or_default();
            let mut value: Value = serde_json::from_str(&s).unwrap_or_default();
            normalize_nulls(&mut value);
            // 备份是历史版本：先累计步进再 reconcile（否则扁平旧字段被当未知剔除，L1）
            let bak_version = value
                .get("version")
                .and_then(Value::as_u64)
                .map(|v| v as u32)
                .unwrap_or(0);
            if bak_version < CURRENT_VERSION {
                value = migrate_steps(value, bak_version);
                report.push(format!("备份 v{bak_version} → v{CURRENT_VERSION} 迁移（累计步进）"));
            }
            let mut cfg = validate_and_repair(reconcile(value, &mut report));
            cfg.read_only = true; // 任何 save 报错（Config::save 检查）
            flush_report(&cfg.load_report);
            cfg
        }
        None => panic!(
            "config.json 是 v{file_version}，本 binary 仅懂 v{CURRENT_VERSION}，\
             且 config.bak/ 无可用旧版本备份——拒绝启动。\
             请升级 binary，或手动从 {} 恢复/删除 config.json",
            dir.join("config.bak").display()
        ),
    }
}

/// null 归一（docs/config.md §null = 缺失）：递归移除 object 中值为 null 的 key；
/// 数组中的 null 不适用。加载、工具写入与保存遵守同一归一化
pub fn normalize_nulls(v: &mut Value) {
    match v {
        Value::Object(map) => {
            let nulls: Vec<String> = map
                .iter()
                .filter(|(_, val)| val.is_null())
                .map(|(k, _)| k.clone())
                .collect();
            for k in nulls {
                map.remove(&k);
            }
            for val in map.values_mut() {
                normalize_nulls(val);
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                normalize_nulls(item);
            }
        }
        _ => {}
    }
}

/// reconcile（docs/config.md §递归 reconcile 逻辑链，registry 驱动）：
/// 未知 path 递归剔除并逐项上报；静态 object 向下递归组装 child；
/// 叶子缺失/类型错误回退自身 default；map 缺失/类型错误回退自身 default、
/// 已存在不 merge default key，entry 经 probe 修复（无法修复 = 该 entry 不存在）。
fn reconcile(mut value: Value, report: &mut Vec<String>) -> Config {
    let default_v = serde_json::to_value(Config::default()).unwrap();
    reconcile_node("", &mut value, &default_v, report);
    let mut cfg: Config = serde_json::from_value(value).unwrap_or_else(|e| {
        report.push(format!("reconcile: 仍无法反序列化（{e}），整份回退 default"));
        Config::default()
    });
    cfg.load_report = std::mem::take(report);
    cfg
}

/// 节点的静态 children（root "" = Config 顶层字段）
fn child_paths(path: &str) -> Vec<&'static str> {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}.")
    };
    let mut out: Vec<&'static str> = meta::NODES
        .iter()
        .map(|n| n.path)
        .filter(|p| p.len() > prefix.len() && p.starts_with(&prefix) && !p[prefix.len()..].contains('.'))
        .map(|p| &p[prefix.len()..])
        .collect();
    out.sort_unstable();
    out
}

/// 动态 map key 的运行时 grammar 检查（docs/config.md §Config path grammar，单源在 config.rs）
fn valid_map_key(k: &str) -> bool {
    super::valid_dynamic_key(k)
}

fn reconcile_node(path: &str, v: &mut Value, default: &Value, report: &mut Vec<String>) {
    // root "" 视为静态 Object；其余查注册表
    let kind = if path.is_empty() {
        NodeKind::Object
    } else {
        match meta::node_meta(path) {
            Some(m) => m.kind,
            None => return, // 未注册节点由父级的未知剔除处理
        }
    };
    match kind {
        NodeKind::Leaf => {
            let type_ok = match (&*v, default) {
                (Value::Bool(_), Value::Bool(_)) => true,
                (Value::String(_), Value::String(_)) => true,
                (Value::Number(a), Value::Number(b)) => {
                    // default 是整数 → 值也须整数可表示（float 2.5 进 usize 字段
                    // 会过不去 serde 爆整份回退；字段级 default 兜底才是设计语义，L2）
                    if b.is_i64() || b.is_u64() {
                        a.is_i64() || a.is_u64()
                    } else {
                        true
                    }
                }
                _ => false,
            };
            if !type_ok {
                *v = default.clone();
                report.push(format!("reconcile: 字段 {path} 缺失或非法，回退 default"));
            }
        }
        NodeKind::Object => {
            if !v.is_object() {
                *v = default.clone();
                report.push(format!("reconcile: 字段 {path} 缺失或非法，回退 default"));
                return;
            }
            let children = child_paths(path);
            let map = v.as_object_mut().unwrap();
            // 未知 child 剔除（version 是文件级控制字段，豁免）
            let unknown: Vec<String> = map
                .keys()
                .filter(|k| *k != "version" && !children.contains(&k.as_str()))
                .cloned()
                .collect();
            for k in unknown {
                map.remove(&k);
                let full = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                report.push(format!("reconcile: 剔除未知字段 {full}"));
            }
            for child in children {
                let cpath = if path.is_empty() {
                    child.to_string()
                } else {
                    format!("{path}.{child}")
                };
                // child default 从父 default 直取（递归携带各自子树的 default）
                let cdefault = default.get(child).cloned().unwrap_or(Value::Null);
                match map.get_mut(child) {
                    None => {
                        map.insert(child.to_string(), cdefault);
                        report.push(format!("reconcile: 缺失字段 {cpath} 补 default"));
                    }
                    Some(cv) => reconcile_node(&cpath, cv, &cdefault, report),
                }
            }
        }
        NodeKind::Map { entry_probe } => {
            if !v.is_object() {
                *v = default.clone();
                report.push(format!("reconcile: 字段 {path} 缺失或非法，回退 default"));
                return;
            }
            let map = v.as_object_mut().unwrap();
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                // 动态 key grammar 运行时检查（docs/config.md：无法 default 化的 key，
                // 修复结果为该 entry 不存在；其余 map entry 保留）
                if !valid_map_key(&k) {
                    map.remove(&k);
                    report.push(format!("reconcile: 剔除非法 key {path}.{k}（path grammar）"));
                    continue;
                }
                let ev = map[&k].clone();
                match entry_probe(&ev) {
                    // entry 按固定 value schema 归一（serde default 填充、未知 key 剔除）
                    Some(fixed) => {
                        map.insert(k.clone(), fixed);
                    }
                    None => {
                        map.remove(&k);
                        report.push(format!(
                            "reconcile: 剔除无法修复的 entry {path}.{k}（其余 entry 保留）"
                        ));
                    }
                }
            }
        }
    }
}

/// 对称备份：写 config.bak/config-v0NN.json；同版本已存在则不覆盖（第一份最接近原始现场）
fn backup_bytes(dir: &Path, tag: &str, bytes: &[u8]) -> PathBuf {
    let bak_dir = dir.join("config.bak");
    let _ = std::fs::create_dir_all(&bak_dir);
    let path = bak_dir.join(format!("config-{tag}.json"));
    if !path.exists() {
        let _ = std::fs::write(&path, bytes);
    }
    path
}

/// 找 ≤ max_version 的最大备份
fn best_backup(dir: &Path, max_version: u32) -> Option<(u32, PathBuf)> {
    let bak_dir = dir.join("config.bak");
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in std::fs::read_dir(bak_dir).ok()? {
        let path = entry.ok()?.path();
        let name = path.file_name()?.to_str()?;
        let v: u32 = name
            .strip_prefix("config-v")?
            .strip_suffix(".json")?
            .parse()
            .ok()?;
        if v <= max_version && best.as_ref().is_none_or(|(bv, _)| v > *bv) {
            best = Some((v, path));
        }
    }
    best
}

/// 只读加载预览（docs/config.md §外部文件自动载入）：
/// 不备份、不写回、缺失不 bootstrap——只读解析 + 完整加载管线
/// （migration → null 归一 → reconcile → serde 验证 → validate）。
/// 错误原样返回：调用方保持 live Config 不变并在 UI 显示具体错误。
pub fn preview(dir: &Path) -> Result<Config, String> {
    let file = dir.join(CONFIG_FILE);
    let raw = std::fs::read_to_string(&file)
        .map_err(|_| "配置文件被移动或者删除".to_string())?;
    let mut value: Value =
        serde_json::from_str(&raw).map_err(|e| format!("config.json 无法解析（{e}）"))?;
    normalize_nulls(&mut value);
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(0);
    match version.cmp(&CURRENT_VERSION) {
        std::cmp::Ordering::Greater => {
            return Err(format!(
                "config v{version} 高于本 binary v{CURRENT_VERSION}，外部载入不降级"
            ));
        }
        std::cmp::Ordering::Less => {
            value = migrate_steps(value, version);
        }
        std::cmp::Ordering::Equal => {}
    }
    let mut report = Vec::new();
    Ok(validate_and_repair(reconcile(value, &mut report)))
}

fn flush_report(lines: &[String]) {
    for l in lines {
        eprintln!("[config] {l}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tmp() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "mig-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn legacy_v0_migrates_with_backup_and_writeback() {
        let dir = tmp();
        // 旧文件：无 version、带一个未知字段
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"compression_reserve_default": 1234, "dead_field": true}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.compression_reserve_default, 1234); // 用户数据保留
        assert!(!cfg.read_only);
        // 备份 v0000 + 写回 version=CURRENT
        assert!(dir.join("config.bak/config-v0000.json").exists());
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap()).unwrap();
        assert_eq!(written["version"], CURRENT_VERSION);
        assert!(written.get("dead_field").is_none());
    }

    #[test]
    fn v1_flat_kaomoji_migrates_to_system_pool() {
        let dir = tmp();
        // v1 文件：扁平 kaomoji map（含用户自定义 key）
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"version": 1, "kaomoji": {"idle": {"face": "(´ω`)", "motion": "still"},
                "celebrate": {"face": "(≧▽≦)", "motion": "bounce"}}}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        // 扁平 map 整体迁入 system 池（行为保持），user 池空
        assert_eq!(cfg.kaomoji.system["celebrate"].face, "(≧▽≦)");
        assert_eq!(cfg.kaomoji.system["idle"].face, "(´ω`)");
        assert!(cfg.kaomoji.user.is_empty());
        // 基础 key 缺失项由 reconcile 补 default（processing/notify 旧文件没有）
        assert!(cfg.kaomoji.system.contains_key("processing"));
        assert!(cfg.kaomoji.system.contains_key("notify"));
        // 写回 current 两池形态
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap()).unwrap();
        assert_eq!(written["version"], CURRENT_VERSION);
        assert!(written["kaomoji"]["system"]["celebrate"].is_object());
        assert!(written["kaomoji"]["user"].is_object());
    }

    #[test]
    fn v2_flat_timer_migrates_to_subtree() {
        let dir = tmp();
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"version": 2, "timer_interval_ms": 5000, "timer_tick_ms": 3000}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        // 扁平字段收编 timer 子树；缺失成员 reconcile 补 default
        assert_eq!(cfg.timer.interval_ms, 5000);
        assert_eq!(cfg.timer.tick_ms, 3000);
        assert_eq!(cfg.timer.stagger_ms, 30_000);
        assert_eq!(cfg.timer.batch, 2);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap()).unwrap();
        assert_eq!(written["version"], 3);
        assert!(written.get("timer_interval_ms").is_none());
        assert_eq!(written["timer"]["interval_ms"], 5000);
    }

    #[test]
    fn v0_migrates_through_all_steps() {
        let dir = tmp();
        // v0（无 version）：同时携带扁平 kaomoji 与扁平 timer —— 累计步进两步都跑
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"kaomoji": {"idle": {"face": "(´ω`)", "motion": "still"}}, "timer_interval_ms": 7000}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.kaomoji.system["idle"].face, "(´ω`)");
        assert_eq!(cfg.timer.interval_ms, 7000);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap()).unwrap();
        assert_eq!(written["version"], CURRENT_VERSION);
    }

    #[test]
    fn load_repairs_pool_invariant_violation_without_blocking() {
        let dir = tmp();
        // v2 文件：两池 key 冲突 + 缺基础 key（只剩 idle）
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"version": 3, "kaomoji": {
                "system": {"idle": {"face": "a", "motion": "still"}},
                "user": {"idle": {"face": "b", "motion": "float"}}}}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        // 不阻断启动：kaomoji 节点 default 化 + 报告
        assert_eq!(cfg.kaomoji, crate::config::KaomojiConfig::default());
        assert!(cfg.load_report.iter().any(|l| l.contains("重复")));
        assert!(cfg.load_report.iter().any(|l| l.contains("default 化")));
    }

    #[test]
    fn current_version_loads_clean() {
        let dir = tmp();
        Config::default().save(&dir).unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg, Config::default());
        assert!(!dir.join("config.bak").exists());
    }

    #[test]
    fn bad_field_falls_back_to_field_default_keeping_others() {
        let dir = tmp();
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"version": 1, "compression_reserve_default": "oops", "base_prompt": "自定义"}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.compression_reserve_default, 10000); // 病灶字段回退
        assert_eq!(cfg.base_prompt, "自定义"); // 其他保留
        assert!(cfg.load_report.iter().any(|l| l.contains("compression_reserve_default")));
    }

    #[test]
    fn corrupt_file_regenerates_with_backup() {
        let dir = tmp();
        std::fs::write(dir.join(CONFIG_FILE), "not json{{{").unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.compression_reserve_default, Config::default().compression_reserve_default);
        assert!(cfg.load_report.iter().any(|l| l.contains("无法解析")));
        assert!(dir.join("config.bak/config-corrupt.json").exists());
    }

    #[test]
    fn downgrade_loads_best_backup_readonly_and_never_touches_new_file() {
        let dir = tmp();
        // 新版文件 v99
        std::fs::write(dir.join(CONFIG_FILE), r#"{"version": 99, "compression_reserve_default": 1}"#).unwrap();
        // 历史备份 v0001
        std::fs::create_dir_all(dir.join("config.bak")).unwrap();
        std::fs::write(
            dir.join("config.bak/config-v0001.json"),
            r#"{"version": 1, "compression_reserve_default": 4321}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.compression_reserve_default, 4321); // 来自备份
        assert!(cfg.read_only);
        assert!(cfg.save(&dir).is_err()); // 只读降级：写报错
        // config.json（新版）未被碰
        let disk = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(disk.contains("\"version\": 99"));
        // 新版现场已对称备份
        assert!(dir.join("config.bak/config-v0099.json").exists());
    }

    #[test]
    #[should_panic(expected = "拒绝启动")]
    fn downgrade_without_backup_refuses_to_start() {
        let dir = tmp();
        std::fs::write(dir.join(CONFIG_FILE), r#"{"version": 99}"#).unwrap();
        load(&dir);
    }

    #[test]
    fn downgrade_migrates_legacy_backup_before_reconcile() {
        // L1：降级加载旧版备份时先累计步进——v1 备份的扁平 kaomoji 不被当未知剔除
        let dir = tmp();
        std::fs::write(dir.join(CONFIG_FILE), r#"{"version": 99}"#).unwrap();
        std::fs::create_dir_all(dir.join("config.bak")).unwrap();
        std::fs::write(
            dir.join("config.bak/config-v0001.json"),
            r#"{"version": 1, "kaomoji": {"celebrate": {"face": "(≧▽≦)", "motion": "bounce"}},
                "timer_interval_ms": 7000}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert!(cfg.read_only);
        // 扁平 kaomoji 迁入 system 池 + 扁平 timer 迁入子树（两步都跑）
        assert_eq!(cfg.kaomoji.system["celebrate"].face, "(≧▽≦)");
        assert_eq!(cfg.timer.interval_ms, 7000);
    }

    #[test]
    fn float_into_int_leaf_falls_back_to_field_default_not_whole_config() {
        // L2：timer.batch = 2.5（float 进 usize 字段）→ 仅该字段回退 default，其他保留
        let dir = tmp();
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"version": 3, "timer": {"batch": 2.5, "interval_ms": 9000}, "view_scale": 0.7}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.timer.batch, 2); // 病灶字段回退 default
        assert_eq!(cfg.timer.interval_ms, 9000); // 其他保留
        assert_eq!(cfg.view_scale, 0.7); // 整份不回退
        assert!(cfg.load_report.iter().any(|l| l.contains("timer.batch")));
    }

    #[test]
    fn recursive_reconcile_removes_nested_unknown_and_repairs_entries() {
        let dir = tmp();
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"version": 3,
                "llm": {"active": "deepseek", "unknown_nested": 1,
                        "providers": {
                            "deepseek": {"base_url": "https://api.deepseek.com", "model": "deepseek-chat", "junk": true},
                            "BadKey": {"base_url": "x", "model": "y"},
                            "broken": {"model": 123}
                        }},
                "kaomoji": {"system": {
                    "idle": {"face": "(´ω`)", "motion": "still", "junk": 1},
                    "processing": {"face": "(ˇωˇ」∠)_", "motion": "float"},
                    "notify": {"face": "✧*｡٩(ˊᗜˋ*)و✧*｡", "motion": "bounce"}
                }, "user": {}}
            }"#,
        )
        .unwrap();
        let cfg = load(&dir);
        // 嵌套未知字段剔除（llm.unknown_nested / provider.junk / kaomoji entry.junk）
        assert!(!cfg.load_report.iter().any(|l| l.contains("panic")));
        let v = serde_json::to_value(&cfg).unwrap();
        assert!(v["llm"].get("unknown_nested").is_none());
        assert!(v["llm"]["providers"]["deepseek"].get("junk").is_none());
        assert!(v["kaomoji"]["system"]["idle"].get("junk").is_none());
        // 保留的合法字段不丢
        assert_eq!(cfg.llm.active, "deepseek");
        assert_eq!(cfg.llm.providers["deepseek"].model, "deepseek-chat");
        assert_eq!(cfg.kaomoji.system["idle"].face, "(´ω`)");
        // 非法 key 与无法修复的 entry 剔除，其余 entry 保留
        assert!(!cfg.llm.providers.contains_key("BadKey"));
        assert!(!cfg.llm.providers.contains_key("broken"));
        assert!(cfg.load_report.iter().any(|l| l.contains("path grammar")));
        assert!(cfg.load_report.iter().any(|l| l.contains("无法修复")));
    }

    #[test]
    fn null_normalized_to_missing_on_load() {
        let dir = tmp();
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"version": 3, "view_scale": null, "timer": {"interval_ms": 60000}, "llm": {"active": null}}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        // null = 缺失：view_scale/llm.active 回 default；显式值保留
        assert_eq!(cfg.view_scale, 1.0);
        assert_eq!(cfg.timer.interval_ms, 60000);
        assert_eq!(cfg.llm.active, "debug");
    }

    #[test]
    fn preview_readonly_pipeline() {
        let dir = tmp();
        // 缺失 → 具体错误，不 bootstrap
        assert_eq!(preview(&dir).unwrap_err(), "配置文件被移动或者删除");
        assert!(!dir.join(CONFIG_FILE).exists());
        // 合法 current → 完整管线（reconcile 补 default），不写回（文件内容不变）
        std::fs::write(dir.join(CONFIG_FILE), r#"{"version": 3, "view_scale": 0.6}"#).unwrap();
        let cfg = preview(&dir).unwrap();
        assert_eq!(cfg.view_scale, 0.6);
        assert_eq!(cfg.timer.interval_ms, 300_000); // reconcile 补 default
        let disk = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(!disk.contains("timer")); // 不写回
        // 旧版 v1 → migration 应用（kaomoji 两池化），同样不写回
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"version": 1, "kaomoji": {"idle": {"face": "(´ω`)", "motion": "still"}}}"#,
        )
        .unwrap();
        let cfg = preview(&dir).unwrap();
        assert!(cfg.kaomoji.system.contains_key("notify"));
        // 无法解析 → 具体错误
        std::fs::write(dir.join(CONFIG_FILE), "not json{{{").unwrap();
        assert!(preview(&dir).unwrap_err().contains("无法解析"));
        // 更高版本 → 拒绝（不降级）
        std::fs::write(dir.join(CONFIG_FILE), r#"{"version": 99}"#).unwrap();
        assert!(preview(&dir).unwrap_err().contains("不降级"));
    }
}
