//! Card 文件持久化（docs/components.md §Card 文件 / docs/storage.md §Memory Workspace）：
//! 一张 Card = memory/cards/<id>.card.json 一个完整 JSON 文件——component（Agent 的
//! ComponentSpec）与 _meta（本地 Surface 管理状态）同位。文件即 Card 的跨重启真相：
//! 恢复从文件读，不经 effect.jsonl replay（动作审计不反推 Card）。
//!
//! _meta 是本地 Surface 状态：schema 版本 / created / user_closed（显示选择）/
//! layout（direction 与 auto/manual offset）。Agent 普通同 id 更新只换 component、
//! 不覆盖 _meta——不能借一次更新偷偷覆盖用户的显示选择与布局。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::lifecycle::CardMeta;

/// _meta.schema 当前版本
pub const SCHEMA: u32 = 1;

/// Card 空间布局（Surface 真相：direction 与 auto/manual offset）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CardLayout {
    /// Agent spec.direction（每次更新跟随 spec；auto/省略 = None）
    pub direction: Option<String>,
    /// 相对 pet 中心的偏移（用户拖过才有；auto 布局为 None）
    pub offset: Option<(i64, i64)>,
    /// 用户亲手拖过：place 保持偏移不重算
    #[serde(default)]
    pub manual: bool,
}

/// Card 注册表条目（运行期投影 = CardMeta + _meta 状态；component 全文在文件）
#[derive(Debug, Clone, PartialEq)]
pub struct CardEntry {
    pub meta: CardMeta,
    /// 显示选择（Surface 意图）：用户隐藏 = true（Agent 更新不覆盖）
    pub user_closed: bool,
    pub layout: CardLayout,
}

/// .card.json 文件形态（完整 JSON：内容与 Surface 状态同位）
#[derive(Serialize, Deserialize)]
struct CardFile {
    component: Value,
    #[serde(rename = "_meta")]
    meta: CardFileMeta,
}

#[derive(Serialize, Deserialize)]
struct CardFileMeta {
    schema: u32,
    created: i64,
    #[serde(default)]
    user_closed: bool,
    #[serde(default)]
    layout: CardLayout,
}

/// `<id>.card.json` 路径（id 可含 `/`——Tauri label 规则；`..` 段在 tool 校验层已拒绝）
fn file_path(cards_dir: &Path, id: &str) -> PathBuf {
    cards_dir.join(format!("{id}.card.json"))
}

/// 从文件恢复全部 Card 注册表条目（启动 replay：文件即真相，无文件 = 无 Card）。
/// 坏文件跳过（单文件病灶不带倒整体）；id 取相对路径去 `.card.json` 后缀。
pub fn load_all(cards_dir: &Path) -> std::collections::HashMap<String, CardEntry> {
    let mut out = std::collections::HashMap::new();
    let mut stack = vec![cards_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".card.json") else {
                continue;
            };
            let rel = path
                .strip_prefix(cards_dir)
                .ok()
                .and_then(|p| p.parent().map(|pp| pp.join(stem)))
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| stem.to_string());
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(file) = serde_json::from_str::<CardFile>(&text) else {
                eprintln!("[cards] 坏文件跳过 {}: 解析失败", path.display());
                continue;
            };
            let typ = file
                .component
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let title = file
                .component
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            out.insert(
                rel.clone(),
                CardEntry {
                    meta: CardMeta {
                        id: rel,
                        typ,
                        title,
                        created: file.meta.created,
                    },
                    user_closed: file.meta.user_closed,
                    layout: file.meta.layout,
                },
            );
        }
    }
    out
}

/// upsert 落盘：新建 → 默认 _meta（user_closed=false / 布局空，direction 取 spec）；
/// 同 id 更新 → 只换 component（保留 _meta；direction 跟随 spec 刷新）。
/// 返回 (CardMeta, created)。文件先写成功后改内存——先落盘后改内存（与 cron 同序）。
pub fn upsert(
    cards_dir: &Path,
    cards: &mut std::collections::HashMap<String, CardEntry>,
    spec: &Value,
    ts: i64,
) -> Result<(CardMeta, bool), String> {
    let id = spec
        .get("id")
        .and_then(Value::as_str)
        .ok_or("spec.id 缺失")?
        .to_string();
    let direction = spec
        .get("direction")
        .and_then(Value::as_str)
        .filter(|d| *d != "auto")
        .map(str::to_string);
    let existing = cards.get(&id);
    let (created, meta) = match existing {
        Some(e) => (false, e.meta.created),
        None => (true, ts),
    };
    let entry = CardEntry {
        meta: CardMeta {
            id: id.clone(),
            typ: spec
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            title: spec
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            created: meta,
        },
        // Agent 普通更新不覆盖显示选择；offset/manual 同理保留
        user_closed: existing.map(|e| e.user_closed).unwrap_or(false),
        layout: CardLayout {
            direction,
            offset: existing.and_then(|e| e.layout.offset),
            manual: existing.map(|e| e.layout.manual).unwrap_or(false),
        },
    };
    let file = CardFile {
        component: spec.clone(),
        meta: CardFileMeta {
            schema: SCHEMA,
            created: entry.meta.created,
            user_closed: entry.user_closed,
            layout: entry.layout.clone(),
        },
    };
    let path = file_path(cards_dir, &id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cards 目录创建失败：{e}"))?;
    }
    let text = serde_json::to_string_pretty(&file).map_err(|e| format!("Card 序列化失败：{e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("Card 文件写入失败：{e}"))?;
    let meta = entry.meta.clone();
    cards.insert(id, entry);
    Ok((meta, created))
}

/// dismiss：结束 Surface——删文件、出注册表、忘记布局（agent close / 用户 × 共用）
pub fn remove(
    cards_dir: &Path,
    cards: &mut std::collections::HashMap<String, CardEntry>,
    id: &str,
) -> Option<CardEntry> {
    let entry = cards.remove(id)?;
    let path = file_path(cards_dir, id);
    if path.exists() {
        // 尽力而为：文件删除失败不阻断主流程（注册表已出，重启经文件复活则再 dismiss）
        let _ = std::fs::remove_file(&path);
    }
    Some(entry)
}

/// 布局回写（用户拖拽结束）：只改 _meta.layout.offset/manual，不动 component 与显示选择
pub fn write_layout(
    cards_dir: &Path,
    cards: &mut std::collections::HashMap<String, CardEntry>,
    id: &str,
    offset: (i64, i64),
) -> Result<(), String> {
    let Some(entry) = cards.get_mut(id) else {
        return Err(format!("Card '{id}' 不存在"));
    };
    entry.layout.offset = Some(offset);
    entry.layout.manual = true;
    let path = file_path(cards_dir, id);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Err(format!("Card 文件读取失败：{id}"));
    };
    let Ok(mut file) = serde_json::from_str::<CardFile>(&text) else {
        return Err(format!("Card 文件解析失败：{id}"));
    };
    file.meta.layout = entry.layout.clone();
    let text = serde_json::to_string_pretty(&file).map_err(|e| format!("Card 序列化失败：{e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("Card 文件写入失败：{e}"))
}

/// 显示选择回写（用户隐藏/恢复）：只改 _meta.user_closed
pub fn write_user_closed(
    cards_dir: &Path,
    cards: &mut std::collections::HashMap<String, CardEntry>,
    id: &str,
    user_closed: bool,
) -> Result<(), String> {
    let Some(entry) = cards.get_mut(id) else {
        return Err(format!("Card '{id}' 不存在"));
    };
    entry.user_closed = user_closed;
    let path = file_path(cards_dir, id);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Err(format!("Card 文件读取失败：{id}"));
    };
    let Ok(mut file) = serde_json::from_str::<CardFile>(&text) else {
        return Err(format!("Card 文件解析失败：{id}"));
    };
    file.meta.user_closed = user_closed;
    let text = serde_json::to_string_pretty(&file).map_err(|e| format!("Card 序列化失败：{e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("Card 文件写入失败：{e}"))
}

/// 读 component 全文（list_cards IPC / 恢复用）
pub fn read_component(cards_dir: &Path, id: &str) -> Option<Value> {
    let text = std::fs::read_to_string(file_path(cards_dir, id)).ok()?;
    serde_json::from_str::<CardFile>(&text)
        .ok()
        .map(|f| f.component)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ambery-cards-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dir = d.join("cards");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn spec(id: &str) -> Value {
        serde_json::json!({"id": id, "type": "todobox", "title": "清单", "items": [{"text": "a", "done": false}]})
    }

    #[test]
    fn upsert_create_then_reload_roundtrip() {
        let dir = tmp("crt");
        let mut cards = std::collections::HashMap::new();
        let (meta, created) = upsert(&dir, &mut cards, &spec("todo-1"), 1000).unwrap();
        assert!(created);
        assert_eq!(meta.created, 1000);
        assert!(dir.join("todo-1.card.json").exists());
        // 文件形态：component + _meta 同位
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("todo-1.card.json")).unwrap()).unwrap();
        assert_eq!(raw["component"]["type"], "todobox");
        assert_eq!(raw["_meta"]["schema"], 1);
        assert_eq!(raw["_meta"]["user_closed"], false);
        // 重启恢复：注册表从文件读
        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["todo-1"].meta.typ, "todobox");
        assert_eq!(loaded["todo-1"].meta.created, 1000);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn update_preserves_meta_not_user_choices() {
        let dir = tmp("upd");
        let mut cards = std::collections::HashMap::new();
        upsert(&dir, &mut cards, &spec("todo-1"), 1000).unwrap();
        // 用户拖过 + 隐藏
        write_layout(&dir, &mut cards, "todo-1", (30, 40)).unwrap();
        write_user_closed(&dir, &mut cards, "todo-1", true).unwrap();
        // Agent 同 id 更新（换内容+换 direction）
        let mut s2 = spec("todo-1");
        s2["title"] = Value::from("清单 v2");
        s2["direction"] = Value::from("ne");
        let (meta, created) = upsert(&dir, &mut cards, &s2, 2000).unwrap();
        assert!(!created);
        assert_eq!(meta.created, 1000, "created 不被更新覆盖");
        let e = &cards["todo-1"];
        assert!(e.user_closed, "显示选择不被 Agent 更新覆盖");
        assert_eq!(e.layout.offset, Some((30, 40)), "manual offset 保留");
        assert!(e.layout.manual);
        assert_eq!(e.layout.direction.as_deref(), Some("ne"), "direction 跟随 spec");
        // 文件里 component 已换、_meta 保留
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("todo-1.card.json")).unwrap()).unwrap();
        assert_eq!(raw["component"]["title"], "清单 v2");
        assert_eq!(raw["_meta"]["user_closed"], true);
        assert_eq!(raw["_meta"]["layout"]["offset"], serde_json::json!([30, 40]));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn remove_deletes_file_and_forgets() {
        let dir = tmp("rm");
        let mut cards = std::collections::HashMap::new();
        upsert(&dir, &mut cards, &spec("todo-1"), 1000).unwrap();
        let entry = remove(&dir, &mut cards, "todo-1").unwrap();
        assert_eq!(entry.meta.id, "todo-1");
        assert!(!dir.join("todo-1.card.json").exists(), "dismiss 删文件");
        assert!(load_all(&dir).is_empty(), "重启后不再复活");
        assert!(remove(&dir, &mut cards, "todo-1").is_none());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn nested_id_and_corrupt_file() {
        let dir = tmp("nest");
        let mut cards = std::collections::HashMap::new();
        upsert(&dir, &mut cards, &spec("proj/todo-1"), 1000).unwrap();
        assert!(dir.join("proj/todo-1.card.json").exists());
        let loaded = load_all(&dir);
        assert!(loaded.contains_key("proj/todo-1"), "嵌套 id 恢复");
        // 坏文件跳过不带倒整体
        std::fs::write(dir.join("bad.card.json"), "{not json").unwrap();
        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn auto_direction_normalizes_to_none() {
        let dir = tmp("auto");
        let mut cards = std::collections::HashMap::new();
        let mut s = spec("a");
        s["direction"] = Value::from("auto");
        upsert(&dir, &mut cards, &s, 1000).unwrap();
        assert_eq!(cards["a"].layout.direction, None, "auto = 不锁方位");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
