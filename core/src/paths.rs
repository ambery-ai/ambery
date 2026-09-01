//! Config / Storage 目录解析（§13 分离：同根不同子路径）。
//!
//! 默认布局：
//! ```text
//! %USERPROFILE%\.config\ambery\
//!   config.json    ← Config（启动配置）
//!   storage\       ← Storage（session data）
//! ```
//! 覆盖：AMBERY_CONFIG_DIR / AMBERY_STORAGE_DIR（开发时指向临时目录）

use std::path::PathBuf;

pub const APP_DIR_NAME: &str = "ambery";

/// Config 目录（不含文件名）
pub fn config_root() -> PathBuf {
    if let Ok(dir) = std::env::var("AMBERY_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    home_config_root().join(APP_DIR_NAME)
}

/// Storage 目录（默认 = config_root/storage）
pub fn storage_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AMBERY_STORAGE_DIR") {
        return PathBuf::from(dir);
    }
    config_root().join("storage")
}

/// Config 文件完整路径
pub fn config_file() -> PathBuf {
    config_root().join(crate::CONFIG_FILE)
}

/// 应用级 env 文件（key 存储位；0600，`KEY=value` 行）——
/// 覆盖系统环境变量的应用级层，见 docs/llm-setup.md §Key storage model
pub fn env_file() -> PathBuf {
    config_root().join("env")
}

fn home_config_root() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".config")
}
