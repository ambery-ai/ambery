//! 应用级 env 文件（key 存储位）：
//! `config_root()/env`（0600，`KEY=value` 行）是覆盖系统环境变量的应用级层——
//! 解析顺序：env 文件 → 进程环境（先命中者胜）。见 docs/llm-setup.md §Key storage model。
//!
//! GUI 应用从 Finder/Dock 启动不继承 shell profile 的 export（环境由 launchd 提供），
//! 因此 key 需要与 shell 无关的家；config.json 永不存 key 值（只存变量名 `api_key_env`）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::paths;

/// 按解析链取变量：env 文件优先，进程环境 fallback。
/// 返回 Some(值) / None（两处都未设置）。
pub fn var_override(name: &str) -> Option<String> {
    env_map(&paths::env_file())
        .get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok())
}

/// 存在性检查：env 文件或进程环境任一命中（与读取链同源，本地即时）。
pub fn is_set(name: &str) -> bool {
    env_map(&paths::env_file()).contains_key(name) || std::env::var(name).is_ok()
}

/// 来源说明（UI 提示用）："env 文件" / "环境变量" / None（未设置）。
pub fn source_of(name: &str) -> Option<&'static str> {
    if env_map(&paths::env_file()).contains_key(name) {
        Some("env 文件")
    } else if std::env::var(name).is_ok() {
        Some("环境变量")
    } else {
        None
    }
}

/// upsert：写入/覆盖某 key 到 env 文件（原子写：temp + rename，防半写）。
/// 文件不存在时创建；已有其他 key 保留。
pub fn upsert(name: &str, value: &str) -> Result<(), String> {
    let path = paths::env_file();
    let mut map = env_map(&path);
    map.insert(name.to_string(), value.to_string());
    write_map(&path, &map)
}

/// remove：从 env 文件删除某 key；其余保留。key 不存在 = 无操作成功。
pub fn remove(name: &str) -> Result<(), String> {
    let path = paths::env_file();
    if !path.exists() {
        return Ok(());
    }
    let mut map = env_map(&path);
    map.remove(name);
    write_map(&path, &map)
}

/// 解析 env 文件为 map（不存在 = 空 map；解析容忍空行与 `#` 注释；
/// 畸形行跳过并告警，不炸——手改文件难免有格式错误）。
fn env_map(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return map;
    };
    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            eprintln!("[envfile] 第 {} 行畸形（无 `=`），跳过: {line}", i + 1);
            continue;
        };
        map.insert(k.trim().to_string(), v.trim().to_string());
    }
    map
}

/// 写 map 回文件：0600 + 原子写（同目录 temp → rename）。
/// 输出固定顺序（按 key 排序）——diff 友好、可预测。
fn write_map(path: &Path, map: &HashMap<String, String>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        out.push_str(k);
        out.push('=');
        out.push_str(&map[k]);
        out.push('\n');
    }
    let tmp: PathBuf = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, out).map_err(|e| format!("写入失败: {e}"))?;
    set_private_perms(&tmp);
    std::fs::rename(&tmp, path).map_err(|e| format!("替换失败: {e}"))?;
    Ok(())
}

/// 0600 权限（Unix）；Windows 无此概念（用户目录 ACL 兜底）。
#[cfg(unix)]
fn set_private_perms(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_private_perms(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 隔离：单测用独立临时目录（不碰真实 config_root）。
    /// 目录名含递增计数——Rust 测试并行跑，共享 pid 会互相删文件。
    fn with_tmp_env(f: impl FnOnce(&Path)) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "ambery-envfile-test-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        f(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn parse_blank_comment_and_malformed_lines() {
        with_tmp_env(|tmp| {
            let f = tmp.join("env");
            write(
                &f,
                "# comment\n\nAMBERY_DEEPSEEK_API_KEY=sk-abc\n  AMBERY_MOONSHOT_API_KEY = sk-def  \nbadline\n",
            );
            let map = env_map(&f);
            assert_eq!(map.get("AMBERY_DEEPSEEK_API_KEY").map(String::as_str), Some("sk-abc"));
            assert_eq!(map.get("AMBERY_MOONSHOT_API_KEY").map(String::as_str), Some("sk-def"));
            assert_eq!(map.len(), 2, "注释/空行/畸形行都不进 map");
        });
    }

    #[test]
    fn upsert_adds_and_overwrites_keeping_others() {
        with_tmp_env(|tmp| {
            let f = tmp.join("env");
            write(&f, "AMBERY_A=1\nAMBERY_B=2\n");
            let mut map = env_map(&f);
            map.insert("AMBERY_A".into(), "10".into());
            map.insert("AMBERY_C".into(), "3".into());
            write_map(&f, &map);
            let after = env_map(&f);
            assert_eq!(after.get("AMBERY_A").map(String::as_str), Some("10"), "覆盖");
            assert_eq!(after.get("AMBERY_B").map(String::as_str), Some("2"), "保留");
            assert_eq!(after.get("AMBERY_C").map(String::as_str), Some("3"), "新增");
        });
    }

    #[test]
    fn remove_deletes_only_target_key() {
        with_tmp_env(|tmp| {
            let f = tmp.join("env");
            write(&f, "AMBERY_A=1\nAMBERY_B=2\n");
            let mut map = env_map(&f);
            map.remove("AMBERY_A");
            write_map(&f, &map);
            let after = env_map(&f);
            assert!(!after.contains_key("AMBERY_A"));
            assert_eq!(after.get("AMBERY_B").map(String::as_str), Some("2"));
        });
    }

    #[test]
    fn missing_file_is_empty_map() {
        with_tmp_env(|tmp| {
            let map = env_map(&tmp.join("does-not-exist"));
            assert!(map.is_empty());
        });
    }

    #[cfg(unix)]
    #[test]
    fn written_file_is_0600() {
        with_tmp_env(|tmp| {
            use std::os::unix::fs::PermissionsExt;
            let f = tmp.join("env");
            write(&f, "AMBERY_A=1\n");
            let mut map = env_map(&f);
            map.insert("AMBERY_B".into(), "2".into());
            write_map(&f, &map);
            let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "env 文件必须 0600");
        });
    }
}
