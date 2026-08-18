//! Config / Storage 目录解析（§13 分离：同根不同子路径）。
//!
//! 默认布局：
//! ```text
//! %USERPROFILE%\.config\ambery\
//!   config.json    ← Config（启动配置）
//!   storage\       ← Storage（session data）
//! ```
//! 覆盖：AMBERY_CONFIG_DIR / AMBERY_STORAGE_DIR（开发时指向临时目录）

use std::path::{Path, PathBuf};

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

/// UIA sidecar exe 路径发现（顺序即优先级）：
/// `AMBERY_SIDECAR` env > 当前 exe 旁（Tauri externalBin 布局）> exe 旁 sidecar/ >
/// Release publish（self-contained win-x64，打包定案）> Debug（仓库开发）。
/// 平台边界：UIA sidecar 是 Windows 可选增强——
/// 非 Windows 一律 None（不发现、不启动、不使用；Hook 驱动核心体验不依赖它）
#[cfg(windows)]
pub fn sidecar_exe() -> Option<PathBuf> {
    let env_path = std::env::var("AMBERY_SIDECAR").ok().map(PathBuf::from);
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for p in sidecar_candidates(env_path.as_deref(), &exe_dir, env!("CARGO_MANIFEST_DIR")) {
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// 候选路径（纯函数，跨平台可测）。env_path=None 时跳过第一级。
pub fn sidecar_candidates(
    env_path: Option<&Path>,
    exe_dir: &Path,
    manifest_dir: &str,
) -> Vec<PathBuf> {
    if !exe_dir.exists() {
        return vec![];
    }
    let exe_name = "ambery-uia-sidecar.exe";
    let mut out = Vec::new();
    if let Some(p) = env_path {
        out.push(p.to_path_buf());
    }
    out.push(exe_dir.join(exe_name));
    out.push(exe_dir.join("sidecar").join(exe_name));
    out.push(
        PathBuf::from(manifest_dir)
            .join("../sidecar/bin/Release/net9.0-windows/win-x64/publish")
            .join(exe_name),
    );
    out.push(
        PathBuf::from(manifest_dir)
            .join("../sidecar/bin/Debug/net9.0-windows")
            .join(exe_name),
    );
    out
}

/// 非 Windows：无 UIA sidecar
#[cfg(not(windows))]
pub fn sidecar_exe() -> Option<PathBuf> {
    None
}

fn home_config_root() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".config")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn sidecar_candidates_cover_env_exe_and_release_layout() {
        let exe_dir = std::env::temp_dir().join("ambery-exe-dir");
        let _ = std::fs::remove_dir_all(&exe_dir);
        std::fs::create_dir_all(&exe_dir).unwrap();
        let manifest = std::env::temp_dir().join("ambery-manifest");
        let env_p = exe_dir.join("env-sidecar.exe");
        std::fs::write(&env_p, "x").unwrap();
        let sibling = exe_dir.join("ambery-uia-sidecar.exe");
        std::fs::write(&sibling, "x").unwrap();
        let rel_release = exe_dir.join("sidecar/ambery-uia-sidecar.exe");
        std::fs::create_dir_all(rel_release.parent().unwrap()).unwrap();
        std::fs::write(&rel_release, "x").unwrap();

        let candidates = sidecar_candidates(Some(&env_p), &exe_dir, manifest.to_str().unwrap());
        let names: Vec<_> = candidates
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["env-sidecar.exe", "ambery-uia-sidecar.exe", "ambery-uia-sidecar.exe", "ambery-uia-sidecar.exe", "ambery-uia-sidecar.exe"], "{candidates:?}");
        // 顺序：env > exe 旁 sibling > exe 旁 sidecar/ > Release publish > Debug
        // join 字符串字面量在 Windows 保留 `/`（display 混用分隔符）——先统一为平台分隔符再匹配
        let sep = std::path::MAIN_SEPARATOR;
        let paths: Vec<_> = candidates
            .iter()
            .map(|p| p.display().to_string().replace('/', &sep.to_string()))
            .collect();
        let norm = |n: &str| n.replace('/', &sep.to_string());
        let pos = |needle: &str| paths.iter().position(|p| p.contains(&norm(needle))).unwrap();
        assert!(pos("env-sidecar") < pos("sidecar/ambery-uia-sidecar"));
        assert!(pos("sidecar/ambery-uia-sidecar") < pos("Release/net9.0-windows/win-x64/publish"));
        assert!(pos("Release/net9.0-windows/win-x64/publish") < pos("Debug/net9.0-windows"));
        let _ = std::fs::remove_dir_all(&exe_dir);
    }

    #[test]
    fn sidecar_candidates_require_existing_exe_dir() {
        let dir = std::env::temp_dir().join("ambery-no-such-dir");
        let candidates = sidecar_candidates(None, &dir, "/nonexistent/manifest");
        assert!(candidates.is_empty());
    }
}
