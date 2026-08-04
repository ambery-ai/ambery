//! Memory（concepts §10f，docs/memory.md）：Harness 管理的持久工作空间。
//! memory/ 根：AGENTS.md + index.md（只读）；notes/ 普通 note（YAML frontmatter 必带
//! description）；cards/ 持久工作产物（不经 read_memory/write_memory 管理）。

use std::path::{Path, PathBuf};

pub const MEMORY_DIR: &str = "memory";
pub const NOTES_DIR: &str = "notes";
pub const CARDS_DIR: &str = "cards";
/// 设计常量（docs/memory.md）：单文件 UTF-8 字节上限（碎片化记忆）
pub const MAX_CONTENT_BYTES: usize = 4096;
/// description 上限（单行、不含 `|`，进 index.md 表）
pub const MAX_DESC_CHARS: usize = 80;
/// 文件名长度上限
pub const MAX_NAME_CHARS: usize = 64;
/// 保留名（可读不可写）
pub const RESERVED: &[&str] = &["index", "AGENTS"];

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

/// Memory 工作空间根管理器（只依赖目录路径，后端/用户/Agent 共享同一文件集）
pub struct Memory {
    root: PathBuf,
}

impl Memory {
    /// bootstrap：创建 Memory 工作空间（根 + notes/ + cards/）+ 默认 AGENTS.md（不存在时）；
    /// 旧扁平根自动迁移（根下普通 .md 移入 notes/，首行 description 注释转 frontmatter）；
    /// index.md 不存在或发生迁移时重生成。
    pub fn bootstrap(storage_dir: &Path) -> std::io::Result<Self> {
        let root = storage_dir.join(MEMORY_DIR);
        std::fs::create_dir_all(root.join(NOTES_DIR))?;
        std::fs::create_dir_all(root.join(CARDS_DIR))?;
        let agents_md = root.join("AGENTS.md");
        if !agents_md.exists() {
            std::fs::write(&agents_md, default_agents_md())?;
        } else if std::fs::read_to_string(&agents_md).ok().as_deref() == Some(OLD_DEFAULT_AGENTS_MD) {
            // 旧版 bootstrap 生成的默认导航（未改过）随工作空间模型收敛
            std::fs::write(&agents_md, default_agents_md())?;
        }
        let m = Self { root };
        let migrated = m.migrate_flat_root()?;
        if migrated || !m.root.join("index.md").exists() {
            m.regenerate_index()?;
        }
        Ok(m)
    }

    /// Memory 工作空间根（storage/memory/）
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 普通 note 目录（memory/notes/）
    pub fn notes_dir(&self) -> PathBuf {
        self.root.join(NOTES_DIR)
    }

    /// 旧扁平根迁移：根下普通 .md（非 index/AGENTS）移入 notes/；
    /// 首行 `<!-- description: ... -->` 注释转为 YAML frontmatter；目标已存在则不动（不丢数据）。
    /// 返回是否发生了迁移。
    fn migrate_flat_root(&self) -> std::io::Result<bool> {
        let mut migrated = false;
        for entry in std::fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if RESERVED.contains(&stem) {
                continue;
            }
            let target = self.notes_dir().join(format!("{stem}.md"));
            if target.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            let converted = match strip_desc_comment(&content) {
                Some((desc, body)) => format!("---\ndescription: {desc}\n---\n\n{body}"),
                None => content,
            };
            std::fs::write(&target, converted)?;
            std::fs::remove_file(&path)?;
            migrated = true;
        }
        Ok(migrated)
    }

    /// 读记忆（None = index.md 导航）。返回 (name, content)；全文含 frontmatter。
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
        let path = if name == "AGENTS" {
            self.root.join("AGENTS.md")
        } else {
            self.notes_dir().join(format!("{name}.md"))
        };
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
        // description 存于文件开头 YAML frontmatter（与正文同文件不漂移，docs/memory.md）
        let file_content = format!("---\ndescription: {description}\n---\n\n{content}");
        std::fs::write(self.notes_dir().join(format!("{name}.md")), file_content)
            .map_err(|e| format!("写入失败：{e}"))?;
        self.regenerate_index()
            .map_err(|e| format!("index.md 重生成失败：{e}"))?;
        Ok(())
    }

    /// 当前普通 note 汇总（名称 + frontmatter description，字典序）；
    /// frontmatter 不合法（缺 description / 含未定义字段 / 非单行）的 note 不进入汇总。
    pub fn list_notes(&self) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = Vec::new();
        let Ok(dir) = std::fs::read_dir(self.notes_dir()) else {
            return entries;
        };
        for entry in dir.flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(desc) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|c| parse_frontmatter_desc(&c))
            else {
                continue;
            };
            entries.push((stem.to_string(), desc));
        }
        entries.sort();
        entries
    }

    /// index.md 全量重生成（docs/memory.md §index.md 契约）：
    /// 汇总 notes/ 内普通 note 的名称 + frontmatter description。
    /// 每次 write 后调用；外部增删文件在下一次 write 时自动收敛。
    fn regenerate_index(&self) -> std::io::Result<()> {
        let mut out = String::from("# Memory Index\n\n| 名称 | 描述 |\n|---|---|\n");
        for (name, desc) in self.list_notes() {
            out.push_str(&format!("| [{name}](notes/{name}.md) | {desc} |\n"));
        }
        std::fs::write(self.root.join("index.md"), out)
    }
}

/// description 存取格式：写入时在文件头加 YAML frontmatter `description` 字段。
/// frontmatter 只定义 `description`：必须是单行标量，不得出现未定义 metadata 字段；
/// 解析失败（无 frontmatter / 字段不合法）返回 None（该 note 不进入 index.md）。
fn parse_frontmatter_desc(content: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()? != "---" {
        return None;
    }
    let desc = lines
        .next()?
        .strip_prefix("description: ")?
        .trim();
    if desc.is_empty() || lines.next()? != "---" {
        return None;
    }
    Some(desc.to_string())
}

/// 旧格式（首行 `<!-- description: ... -->` 注释）拆分，供迁移转换
fn strip_desc_comment(content: &str) -> Option<(String, String)> {
    let first = content.lines().next()?;
    let desc = first
        .strip_prefix("<!-- description: ")?
        .strip_suffix(" -->")?;
    let body = content
        .strip_prefix(first)?
        .strip_prefix('\n')
        .unwrap_or("");
    Some((desc.to_string(), body.to_string()))
}

/// 旧版 bootstrap 默认 AGENTS.md（仅用于识别未改过的自动生成文件并收敛）
const OLD_DEFAULT_AGENTS_MD: &str = "# Memory（Overseer 持久化理解 buffer）\n\n\
     本目录是ペット的长期记忆根（扁平、无子目录，concepts §10f）。普通记忆是同层短小 .md 文件；\n\
     `index.md` 自动汇总所有普通记忆的名称与描述（请勿手编，会被下一次 write 覆盖）。\n\n\
     读写规则：`read_memory` 读记忆（省略 name = 读 index.md 导航）；`write_memory` 整篇新建/覆盖，\n\
     必须附 description（进 index.md）。本文件与 index.md 默认只读；无删除 tool——记忆经同名覆盖演进，\n\
     确需删除由用户或后端直接管理本目录文件。详见 docs/memory.md。\n";

fn default_agents_md() -> String {
    "# Memory Workspace（Overseer 持久工作空间）\n\n\
     本目录是ペット的持久工作空间（concepts §10f）：`notes/` 放长期理解（短小 .md，frontmatter 必带\n\
     description）；`cards/` 放持久工作产物（Component / Card 文件，不经 read_memory / write_memory 管理）。\n\
     `index.md` 自动汇总 notes/ 的名称与描述（请勿手编，会被下一次 write 覆盖）。\n\n\
     读写规则：`read_memory` 读记忆（省略 name = 读 index.md 导航）；`write_memory` 整篇新建/覆盖 notes/\n\
     下的普通 note，必须附 description（写入 frontmatter 并进 index.md）。本文件与 index.md 默认只读；\n\
     无删除 tool——note 经同名覆盖演进，确需删除由用户或后端直接管理 notes/ 文件。详见 docs/memory.md。\n"
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
    fn bootstrap_creates_workspace() {
        let dir = tmp("boot");
        let m = Memory::bootstrap(&dir).unwrap();
        assert!(dir.join("memory/AGENTS.md").exists());
        assert!(dir.join("memory/index.md").exists());
        assert!(dir.join("memory/notes").is_dir());
        assert!(dir.join("memory/cards").is_dir());
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
        // description 写入 frontmatter，read 返回全文含 frontmatter
        assert!(content.starts_with("---\ndescription: 用户的工作偏好\n---\n\n"));
        assert!(dir.join("memory/notes/work-preferences.md").exists());
        let (_, idx) = m.read(None).unwrap();
        assert!(idx.contains("[work-preferences](notes/work-preferences.md) | 用户的工作偏好"));
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
        // 保留名 AGENTS 可读（根下导航文件）
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
        // 外部删除 b-note（用户/后端直接管理 notes/），下一次 write 时 index 收敛
        std::fs::remove_file(dir.join("memory/notes/b-note.md")).unwrap();
        m.write("a-note", "A2", "A 描述").unwrap();
        let (_, idx) = m.read(None).unwrap();
        assert!(idx.contains("a-note"));
        assert!(!idx.contains("b-note"), "{idx}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_frontmatter_excluded_from_index_but_readable() {
        let dir = tmp("fm");
        let m = Memory::bootstrap(&dir).unwrap();
        m.write("ok-note", "正文", "合法描述").unwrap();
        // 外部写入：未定义 metadata 字段 / 无 frontmatter 的 note
        std::fs::write(
            dir.join("memory/notes/extra-field.md"),
            "---\ndescription: 多余字段\nowner: someone\n---\n\n正文",
        )
        .unwrap();
        std::fs::write(dir.join("memory/notes/no-fm.md"), "没有 frontmatter").unwrap();
        m.write("ok-note", "正文 v2", "合法描述").unwrap(); // 触发 index 收敛
        let (_, idx) = m.read(None).unwrap();
        assert!(idx.contains("ok-note"));
        assert!(!idx.contains("extra-field"), "{idx}");
        assert!(!idx.contains("no-fm"), "{idx}");
        // read 不拦：全文照返
        let (_, c) = m.read(Some("extra-field")).unwrap();
        assert!(c.contains("多余字段"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flat_root_migrates_to_notes_with_frontmatter() {
        let dir = tmp("mig");
        let root = dir.join("memory");
        std::fs::create_dir_all(&root).unwrap();
        // 旧扁平布局：旧默认 AGENTS.md + 旧注释格式 note + 无描述外部文件
        std::fs::write(root.join("AGENTS.md"), OLD_DEFAULT_AGENTS_MD).unwrap();
        std::fs::write(
            root.join("work-preferences.md"),
            "<!-- description: 用户的工作偏好 -->\n# 偏好\n正文",
        )
        .unwrap();
        std::fs::write(root.join("loose.md"), "无描述外部文件").unwrap();
        let m = Memory::bootstrap(&dir).unwrap();
        // note 移入 notes/ 且注释转 frontmatter
        assert!(!root.join("work-preferences.md").exists());
        let (_, c) = m.read(Some("work-preferences")).unwrap();
        assert!(c.starts_with("---\ndescription: 用户的工作偏好\n---\n\n# 偏好\n正文"));
        // 无描述文件只移动不转换，不进 index 但仍可读
        let (_, loose) = m.read(Some("loose")).unwrap();
        assert_eq!(loose, "无描述外部文件");
        let (_, idx) = m.read(None).unwrap();
        assert!(idx.contains("[work-preferences](notes/work-preferences.md)"));
        assert!(!idx.contains("loose"), "{idx}");
        // 旧默认 AGENTS.md（未改过）收敛为工作空间版导航
        let (_, agents) = m.read(Some("AGENTS")).unwrap();
        assert!(agents.contains("Memory Workspace"), "{agents}");
        // cards/ 一并创建
        assert!(root.join("cards").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_keeps_existing_notes_target() {
        let dir = tmp("migkeep");
        let root = dir.join("memory");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("dup.md"), "<!-- description: 旧 -->旧版").unwrap();
        std::fs::write(root.join("notes/dup.md"), "---\ndescription: 新\n---\n\n新版").unwrap();
        let m = Memory::bootstrap(&dir).unwrap();
        // 目标已存在：根下旧文件原地保留（不丢数据），index 以 notes/ 为准
        assert!(root.join("dup.md").exists());
        let (_, c) = m.read(Some("dup")).unwrap();
        assert!(c.contains("新版"));
        let (_, idx) = m.read(None).unwrap();
        assert!(idx.contains("| [dup](notes/dup.md) | 新 |"));
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
