//! Memory（concepts §10f，docs/memory.md）：Harness 管理的持久化理解 buffer。
//! 扁平 Markdown 根：普通记忆同层 .md（短小碎片化）；index.md 自动汇总（只读）；
//! AGENTS.md 索引导航（只读，与 Config 域身份提示词不是同一文件）。

use std::path::{Path, PathBuf};

pub const MEMORY_DIR: &str = "memory";
/// 设计常量（docs/memory.md）：单文件 UTF-8 字节上限（碎片化记忆）
pub const MAX_CONTENT_BYTES: usize = 4096;
/// description 上限（单行、不含 `|`，进 index.md 表）
pub const MAX_DESC_CHARS: usize = 80;
/// 文件名长度上限
pub const MAX_NAME_CHARS: usize = 64;
/// 保留名（可读不可写）
const RESERVED: &[&str] = &["index", "AGENTS"];

/// 文件名 grammar（docs/memory.md）：`^[a-z][a-z0-9_-]*$` 且 ≤ 64 字符
pub fn valid_name(name: &str) -> bool {
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Memory 根管理器（只依赖目录路径，后端/用户/Agent 共享同一文件集）
pub struct Memory {
    root: PathBuf,
}

impl Memory {
    /// bootstrap：创建 Memory 根 + 默认 AGENTS.md（不存在时）；index.md 不存在则生成
    pub fn bootstrap(storage_dir: &Path) -> std::io::Result<Self> {
        let root = storage_dir.join(MEMORY_DIR);
        std::fs::create_dir_all(&root)?;
        let agents_md = root.join("AGENTS.md");
        if !agents_md.exists() {
            std::fs::write(&agents_md, default_agents_md())?;
        }
        let m = Self { root };
        if !m.root.join("index.md").exists() {
            m.regenerate_index()?;
        }
        Ok(m)
    }

    /// 读记忆（None = index.md 导航）。返回 (name, content)
    pub fn read(&self, name: Option<&str>) -> Result<(String, String), String> {
        let name = name.unwrap_or("index");
        if name == "index" {
            let content = std::fs::read_to_string(self.root.join("index.md"))
                .map_err(|_| "index.md 不存在（Memory 根未初始化）".to_string())?;
            return Ok(("index".into(), content));
        }
        if !valid_name(name) && name != "AGENTS" {
            return Err(format!(
                "名称 '{name}' 不合法：小写字母开头，仅小写字母/数字/_/-，≤ {MAX_NAME_CHARS} 字符"
            ));
        }
        let path = self.root.join(format!("{name}.md"));
        std::fs::read_to_string(&path)
            .map(|c| (name.to_string(), c))
            .map_err(|_| format!("记忆 '{name}' 不存在（先 read_memory() 看 index）"))
    }

    /// 写记忆：新建或完整替换 + index.md 自动重生成
    pub fn write(&self, name: &str, content: &str, description: &str) -> Result<(), String> {
        if !valid_name(name) {
            return Err(format!(
                "名称 '{name}' 不合法：小写字母开头，仅小写字母/数字/_/-，≤ {MAX_NAME_CHARS} 字符"
            ));
        }
        if RESERVED.contains(&name) {
            return Err(format!("'{name}' 是保留名（index/AGENTS 默认只读）"));
        }
        if content.len() > MAX_CONTENT_BYTES {
            return Err(format!(
                "内容 {} 字节超上限 {MAX_CONTENT_BYTES}（碎片化记忆，拆分多条）",
                content.len()
            ));
        }
        let desc_chars = description.chars().count();
        if description.trim().is_empty() {
            return Err("description 必填（非空，进 index.md）".into());
        }
        if description.contains('\n') || description.contains('|') {
            return Err("description 必须单行且不含 '|'".into());
        }
        if desc_chars > MAX_DESC_CHARS {
            return Err(format!("description {desc_chars} 字符超上限 {MAX_DESC_CHARS}"));
        }
        // description 以注释行存于文件首行（与正文同文件不漂移，docs/memory.md）
        let file_content = format!("<!-- description: {description} -->\n{content}");
        std::fs::write(self.root.join(format!("{name}.md")), file_content)
            .map_err(|e| format!("写入失败：{e}"))?;
        self.regenerate_index()
            .map_err(|e| format!("index.md 重生成失败：{e}"))?;
        Ok(())
    }

    /// index.md 全量重生成（docs/memory.md §index.md 契约）：
    /// 汇总当前普通 .md 的名称 + description（description 取文件内首行标记或空）。
    /// 每次 write 后调用；外部增删文件在下一次 write 时自动收敛。
    fn regenerate_index(&self) -> std::io::Result<()> {
        let mut entries: Vec<(String, String)> = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let path = entry?.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if RESERVED.contains(&stem) {
                continue;
            }
            // description 存于文件首行 HTML 注释（write 时写入）：<!-- description: ... -->
            let desc = std::fs::read_to_string(&path)
                .ok()
                .and_then(|c| parse_desc(&c))
                .unwrap_or_default();
            entries.push((stem.to_string(), desc));
        }
        entries.sort();
        let mut out = String::from("# Memory Index\n\n| 名称 | 描述 |\n|---|---|\n");
        for (name, desc) in entries {
            out.push_str(&format!("| [{name}]({name}.md) | {desc} |\n"));
        }
        std::fs::write(self.root.join("index.md"), out)
    }
}

/// description 存取格式：写入时在文件头加 `<!-- description: ... -->` 注释行，
/// index 生成时解析（Markdown 注释不影响正文渲染；description 与正文同文件不漂移）
fn parse_desc(content: &str) -> Option<String> {
    let first = content.lines().next()?;
    let inner = first
        .strip_prefix("<!-- description: ")?
        .strip_suffix(" -->")?;
    Some(inner.to_string())
}

fn default_agents_md() -> String {
    "# Memory（Overseer 持久化理解 buffer）\n\n\
     本目录是ペット的长期记忆根（扁平、无子目录，concepts §10f）。普通记忆是同层短小 .md 文件；\n\
     `index.md` 自动汇总所有普通记忆的名称与描述（请勿手编，会被下一次 write 覆盖）。\n\n\
     读写规则：`read_memory` 读记忆（省略 name = 读 index.md 导航）；`write_memory` 整篇新建/覆盖，\n\
     必须附 description（进 index.md）。本文件与 index.md 默认只读；无删除 tool——记忆经同名覆盖演进，\n\
     确需删除由用户或后端直接管理本目录文件。详见 docs/memory.md。\n"
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("overseer-mem-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn bootstrap_creates_root_agents_and_index() {
        let dir = tmp("boot");
        let m = Memory::bootstrap(&dir).unwrap();
        assert!(dir.join("memory/AGENTS.md").exists());
        assert!(dir.join("memory/index.md").exists());
        let (_, idx) = m.read(None).unwrap();
        assert!(idx.contains("# Memory Index"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_then_read_and_index_lists_entry() {
        let dir = tmp("rw");
        let m = Memory::bootstrap(&dir).unwrap();
        m.write("work-preferences", "# 偏好\n用户喜欢简洁回复", "用户的工作偏好")
            .unwrap();
        let (name, content) = m.read(Some("work-preferences")).unwrap();
        assert_eq!(name, "work-preferences");
        assert!(content.contains("用户喜欢简洁回复"));
        // description 写入文件头注释
        assert!(content.starts_with("<!-- description: 用户的工作偏好 -->"));
        let (_, idx) = m.read(None).unwrap();
        assert!(idx.contains("[work-preferences](work-preferences.md) | 用户的工作偏好"));
        // 覆盖更新（完整替换语义）
        m.write("work-preferences", "# 偏好 v2", "用户的工作偏好 v2").unwrap();
        let (_, idx2) = m.read(None).unwrap();
        assert!(idx2.contains("用户的工作偏好 v2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_validations() {
        let dir = tmp("valid");
        let m = Memory::bootstrap(&dir).unwrap();
        // 保留名
        assert!(m.write("index", "x", "d").is_err());
        assert!(m.write("AGENTS", "x", "d").is_err());
        // 非法名
        assert!(m.write("Bad Name", "x", "d").is_err());
        assert!(m.write("../escape", "x", "d").is_err());
        // description 缺/多行/含 |/超长
        assert!(m.write("ok-name", "x", "  ").is_err());
        assert!(m.write("ok-name", "x", "两行\n描述").is_err());
        assert!(m.write("ok-name", "x", "含|竖线").is_err());
        assert!(m.write("ok-name", "x", &"长".repeat(81)).is_err());
        // 内容超长
        assert!(m.write("ok-name", &"超".repeat(4097), "d").is_err());
        // 合法
        assert!(m.write("ok-name", "正文", "简短描述").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_missing_and_invalid() {
        let dir = tmp("miss");
        let m = Memory::bootstrap(&dir).unwrap();
        let e = m.read(Some("nope")).unwrap_err();
        assert!(e.contains("不存在"), "{e}");
        assert!(m.read(Some("Bad!")).is_err());
        // 保留名 AGENTS 可读
        let (name, _) = m.read(Some("AGENTS")).unwrap();
        assert_eq!(name, "AGENTS");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn external_delete_converges_on_next_write() {
        let dir = tmp("conv");
        let m = Memory::bootstrap(&dir).unwrap();
        m.write("a-note", "A", "A 描述").unwrap();
        m.write("b-note", "B", "B 描述").unwrap();
        // 外部删除 b-note（用户/后端直接管理），下一次 write 时 index 收敛
        std::fs::remove_file(dir.join("memory/b-note.md")).unwrap();
        m.write("a-note", "A2", "A 描述").unwrap();
        let (_, idx) = m.read(None).unwrap();
        assert!(idx.contains("a-note"));
        assert!(!idx.contains("b-note"), "{idx}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn name_grammar() {
        assert!(valid_name("work-preferences"));
        assert!(valid_name("a"));
        assert!(!valid_name("A"));
        assert!(!valid_name("_x"));
        assert!(!valid_name("a b"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(&"a".repeat(65)));
    }
}
