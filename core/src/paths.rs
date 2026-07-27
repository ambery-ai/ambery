//! Config / Storage 目录解析（concepts §12/§13 分离：同根不同子路径）。
//!
//! 默认布局：
//! ```text
//! %USERPROFILE%\.config\terminal-overseer\
//!   config.json    ← Config（启动配置）
//!   storage\       ← Storage（session data）
//! ```
//! 覆盖：OVERSEER_CONFIG_DIR / OVERSEER_STORAGE_DIR（开发时指向临时目录）

use std::path::PathBuf;

pub const APP_DIR_NAME: &str = "terminal-overseer";

/// Config 目录（不含文件名）
pub fn config_root() -> PathBuf {
    if let Ok(dir) = std::env::var("OVERSEER_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    home_config_root().join(APP_DIR_NAME)
}

/// Storage 目录（默认 = config_root/storage）
pub fn storage_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OVERSEER_STORAGE_DIR") {
        return PathBuf::from(dir);
    }
    config_root().join("storage")
}

/// Config 文件完整路径
pub fn config_file() -> PathBuf {
    config_root().join(crate::CONFIG_FILE)
}

/// UIA sidecar exe 路径发现（docs/sidecar.md §常驻与拉起）：
/// OVERSEER_SIDECAR env > 仓库约定位置（CARGO_MANIFEST_DIR/../sidecar）
pub fn sidecar_exe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("OVERSEER_SIDECAR") {
        let p = PathBuf::from(p);
        return p.exists().then_some(p);
    }
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sidecar/bin/Debug/net9.0-windows/overseer-uia-sidecar.exe");
    p.exists().then_some(p)
}

fn home_config_root() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".config")
}
