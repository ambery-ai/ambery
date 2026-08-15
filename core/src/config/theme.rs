//! 主题导出 / 导入（docs/theme.md §导出、分享与兼容）。
//!
//! 导出文件自包含：`config_version` + 主题名 + 一个完整主题 value（token 覆写表），
//! 不引用导入方的任何其他配置。导入经兼容层：先按声明的 Config version 变换为当前
//! 版本可理解的形态（当前仅自身版本，恒等；未来版本演进在此挂迁移），再校验并写入
//! 本地 themes Map。未来 version > 当前：明确拒绝并提示更新应用，不猜测/截断/静默套用。

use serde_json::{json, Value};

use super::migrate::CURRENT_VERSION;
use super::{validate_theme_table, Config};

/// 主题分享文件目录（config_root/themes/）
pub const THEME_DIR: &str = "themes";

/// 导出主题名对应的 value 到 `<config_root>/themes/<name>.theme.json`，返回文件路径
pub fn export_theme(
    config_root: &std::path::Path,
    config: &Config,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    let value = config
        .themes
        .get(name)
        .ok_or_else(|| format!("主题不存在：{name}"))?;
    let payload = json!({
        "config_version": CURRENT_VERSION,
        "name": name,
        "value": value,
    });
    let dir = config_root.join(THEME_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建主题目录失败: {e}"))?;
    let path = dir.join(format!("{name}.theme.json"));
    let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("写入主题文件失败: {e}"))?;
    Ok(path)
}

/// 版本兼容变换（docs/theme.md §兼容层）：按导出文件声明的 Config version 将主题
/// value 变换为当前版本可理解的形态。当前只承诺自身版本（恒等）；旧版本演进在此挂表
fn compat_transform(_from_version: u32, value: Value) -> Result<Value, String> {
    Ok(value)
}

/// 从 `<config_root>/themes/<file>` 导入主题：读文件 → 版本检查 → 兼容变换 → 校验。
/// 返回（主题名, value JSON），写入 Config 由调用方走统一修改管道（原子拒绝）。
/// 失败不改变当前主题与既有主题表
pub fn import_theme(config_root: &std::path::Path, file: &str) -> Result<(String, Value), String> {
    if file.contains(['/', '\\', '.']) {
        return Err("文件名只允许主题名本身（不带路径与扩展名）".into());
    }
    let path = config_root.join(THEME_DIR).join(format!("{file}.theme.json"));
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取主题文件失败（{}）: {e}", path.display()))?;
    let payload: Value = serde_json::from_str(&text).map_err(|e| format!("主题文件不是合法 JSON: {e}"))?;
    let version = payload["config_version"]
        .as_u64()
        .ok_or("主题文件缺 config_version 字段")? as u32;
    if version > CURRENT_VERSION {
        return Err(format!(
            "主题文件来自更新的 Config 版本（v{version} > 当前 v{CURRENT_VERSION}），请更新应用后再导入"
        ));
    }
    let name = payload["name"]
        .as_str()
        .ok_or("主题文件缺 name 字段")?
        .to_string();
    if !super::valid_dynamic_key(&name) {
        return Err(format!("主题名不符合 path grammar：{name}"));
    }
    let value = compat_transform(version, payload["value"].clone())?;
    if !value.is_object() {
        return Err("主题 value 须为 object（token → CSS 值）".into());
    }
    // 校验（与统一修改管道同一份主题表校验；写回时管道还会再跑一遍，原子拒绝）
    let table: std::collections::HashMap<String, std::collections::HashMap<String, String>> =
        serde_json::from_value(json!({ name.clone(): value.clone() }))
            .map_err(|e| format!("主题 value 结构不合法: {e}"))?;
    let errs = validate_theme_table(&table);
    if !errs.is_empty() {
        return Err(format!("主题校验失败：{}", errs.join("；")));
    }
    Ok((name, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_theme(name: &str) -> Config {
        let mut c = Config::default();
        c.themes.insert(
            name.to_string(),
            std::collections::HashMap::from([("panel-bg".to_string(), "rgba(1,2,3,0.9)".to_string())]),
        );
        c
    }

    #[test]
    fn export_import_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ambery-theme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let c = config_with_theme("my-theme");
        let path = export_theme(&dir, &c, "my-theme").unwrap();
        assert!(path.ends_with("my-theme.theme.json"));
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["config_version"], json!(CURRENT_VERSION));
        assert_eq!(raw["name"], json!("my-theme"));
        let (name, value) = import_theme(&dir, "my-theme").unwrap();
        assert_eq!(name, "my-theme");
        assert_eq!(value["panel-bg"], json!("rgba(1,2,3,0.9)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_future_version() {
        let dir = std::env::temp_dir().join(format!("ambery-theme-fut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(THEME_DIR)).unwrap();
        std::fs::write(
            dir.join(THEME_DIR).join("new.theme.json"),
            serde_json::to_string(&json!({
                "config_version": CURRENT_VERSION + 1,
                "name": "new",
                "value": {},
            }))
            .unwrap(),
        )
        .unwrap();
        let err = import_theme(&dir, "new").unwrap_err();
        assert!(err.contains("更新的 Config 版本"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_bad_token_and_bad_name() {
        let dir = std::env::temp_dir().join(format!("ambery-theme-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(THEME_DIR)).unwrap();
        std::fs::write(
            dir.join(THEME_DIR).join("evil.theme.json"),
            serde_json::to_string(&json!({
                "config_version": CURRENT_VERSION,
                "name": "evil",
                "value": { "panel-bg": "red; } body { display:none" },
            }))
            .unwrap(),
        )
        .unwrap();
        let err = import_theme(&dir, "evil").unwrap_err();
        assert!(err.contains("主题校验失败"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_theme_table_valid() {
        // 内置默认（dark 空覆写）必须通过自身校验
        let c = Config::default();
        assert!(c.themes.contains_key("dark"));
        assert!(validate_theme_table(&c.themes).is_empty());
    }
}
