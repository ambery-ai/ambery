//! 版本与迁移加载管线（docs/config.md）：
//! 数字版本 + enum Migration 稀疏区间映射 + reconcile 字段级 default 兜底
//! + 对称备份（config.bak/config-v0NN.json）+ 降级只读报错。
//!
//! version 是文件级控制字段，不进 Config 结构体；
//! 未带 version 的文件 = v0（前版本号时代，absence 即标记）。

use serde_json::Value;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use super::{Config, CONFIG_FILE};

/// 当前 schema 代际（bump 规则：仅语义断裂 +1）
pub const CURRENT_VERSION: u32 = 1;

/// 迁移动作：Default = 该区间已审计、无需值变换（reconcile 兜底）；
/// Transform = 值变换函数（先变换，再 reconcile）
pub enum Migration {
    Default,
    #[allow(dead_code)] // 暂无值变换条目，首个语义断裂版本启用
    Transform(fn(Value) -> Value),
}

/// 稀疏区间映射：每个历史版本区间一条显式条目，一步到 current。
/// v0 = 未带 version 字段的前版本号时代文件。
static MIGRATIONS: &[(RangeInclusive<u32>, Migration)] = &[(0..=0, Migration::Default)];

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

    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(0); // absence 即 v0

    let mut report = Vec::new();
    match version.cmp(&CURRENT_VERSION) {
        std::cmp::Ordering::Equal => {
            let cfg = reconcile(value, &mut report);
            flush_report(&cfg.load_report);
            cfg
        }
        std::cmp::Ordering::Less => {
            report.push(format!("config v{version} → v{CURRENT_VERSION} 迁移"));
            match MIGRATIONS.iter().find(|(r, _)| r.contains(&version)) {
                Some((_, Migration::Default)) => {
                    report.push("Migration::Default（已审计，无需值变换）".into())
                }
                Some((_, Migration::Transform(f))) => {
                    value = f(value);
                    report.push("Migration::Transform 已应用".into());
                }
                None => report.push(format!("v{version} 无映射条目，直接 reconcile")),
            }
            let mut cfg = reconcile(value, &mut report);
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
            let value: Value = serde_json::from_str(&s).unwrap_or_default();
            let mut cfg = reconcile(value, &mut report);
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

/// reconcile（反射结构对齐）：未知 path 剔除；缺失/整字段非法 → 该字段回退 default。
/// 返回验证通过的 Config。default 是字段级兜底标签，不是整份重生成。
fn reconcile(mut value: Value, report: &mut Vec<String>) -> Config {
    let default_v = serde_json::to_value(Config::default()).unwrap();
    if let (Value::Object(map), Value::Object(dmap)) = (&mut value, &default_v) {
        // 剔除未知字段（version 是控制字段，豁免）
        let unknown: Vec<String> = map
            .keys()
            .filter(|k| *k != "version" && !dmap.contains_key(*k))
            .cloned()
            .collect();
        for k in unknown {
            map.remove(&k);
            report.push(format!("reconcile: 剔除未知字段 {k}"));
        }
        // 缺失字段补 default（字段级）
        for (k, dv) in dmap {
            if !map.contains_key(k) {
                map.insert(k.clone(), dv.clone());
                report.push(format!("reconcile: 缺失字段 {k} 补 default"));
            }
        }
    }
    // 整字段非法（类型/值过不去 serde）→ 仅该字段回退 default：
    // 用「default 全体 + 单字段替换试算」定位病灶，无需逐字段类型信息
    if serde_json::from_value::<Config>(value.clone()).is_err() {
        if let (Value::Object(map), Value::Object(dmap)) = (&mut value, &default_v) {
            for (k, dv) in dmap {
                let user_val = map.get(k).cloned().unwrap_or(Value::Null);
                let mut trial = default_v.clone();
                trial[k.as_str()] = user_val.clone();
                if serde_json::from_value::<Config>(trial).is_err() && user_val != *dv {
                    report.push(format!("reconcile: 字段 {k} 非法，回退 default"));
                    map.insert(k.clone(), dv.clone());
                }
            }
        }
    }
    let mut cfg: Config = serde_json::from_value(value)
        .unwrap_or_else(|e| {
            report.push(format!("reconcile: 仍无法反序列化（{e}），整份回退 default"));
            Config::default()
        });
    cfg.load_report = std::mem::take(report);
    cfg
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
            r#"{"token_threshold": 1234, "dead_field": true}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.token_threshold, 1234); // 用户数据保留
        assert!(!cfg.read_only);
        // 备份 v0000 + 写回 version=1
        assert!(dir.join("config.bak/config-v0000.json").exists());
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap()).unwrap();
        assert_eq!(written["version"], 1);
        assert!(written.get("dead_field").is_none());
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
            r#"{"version": 1, "token_threshold": "oops", "base_prompt": "自定义"}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.token_threshold, 8000); // 病灶字段回退
        assert_eq!(cfg.base_prompt, "自定义"); // 其他保留
        assert!(cfg.load_report.iter().any(|l| l.contains("token_threshold")));
    }

    #[test]
    fn corrupt_file_regenerates_with_backup() {
        let dir = tmp();
        std::fs::write(dir.join(CONFIG_FILE), "not json{{{").unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.token_threshold, Config::default().token_threshold);
        assert!(cfg.load_report.iter().any(|l| l.contains("无法解析")));
        assert!(dir.join("config.bak/config-corrupt.json").exists());
    }

    #[test]
    fn downgrade_loads_best_backup_readonly_and_never_touches_new_file() {
        let dir = tmp();
        // 新版文件 v99
        std::fs::write(dir.join(CONFIG_FILE), r#"{"version": 99, "token_threshold": 1}"#).unwrap();
        // 历史备份 v0001
        std::fs::create_dir_all(dir.join("config.bak")).unwrap();
        std::fs::write(
            dir.join("config.bak/config-v0001.json"),
            r#"{"version": 1, "token_threshold": 4321}"#,
        )
        .unwrap();
        let cfg = load(&dir);
        assert_eq!(cfg.token_threshold, 4321); // 来自备份
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
}
