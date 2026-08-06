//! 卡片生命周期（docs/components.md §卡片生命周期事件）：
//! 语义单源在 core——五类事件的自然语言格式、start/end 时间、存活 N 计数。
//! 前端只做上报终端，不各自拼事件文本。

use serde::{Deserialize, Serialize};

/// 存活卡片元数据（持续管理协议注册表条目）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardMeta {
    pub id: String,
    pub typ: String,
    pub title: String,
    pub created: i64,
}

/// Component 交互事件文本（语义单源，docs/i18n.md §Harness 内部语言）：
/// 前端只上报结构化事实（action + 字段），自然语言由 core 按 Harness 语言现写。
/// 输入 = push_event 的结构化载荷（serde_json Value）
pub fn user_action_desc(lang: crate::i18n::Lang, ev: &serde_json::Value) -> String {
    use crate::i18n::{tr, trf};
    let g = |k: &str| ev[k].as_str().unwrap_or("").to_string();
    match ev["action"].as_str().unwrap_or("") {
        "copy" => trf(lang, "ev.copy", &[("type", g("card_type")), ("title", g("title"))]),
        "jump" => trf(lang, "ev.jump", &[("type", g("card_type")), ("target", g("target"))]),
        "expand_diff" => trf(lang, "ev.expand-diff", &[("type", g("card_type")), ("title", g("title"))]),
        "todo_toggle" => {
            let checked = ev["checked"].as_bool().unwrap_or(false);
            let verb = tr(lang, if checked { "ev.verb-checked" } else { "ev.verb-unchecked" }).to_string();
            trf(lang, "ev.todo-toggle", &[("verb", verb), ("type", g("card_type")), ("text", g("text"))])
        }
        "todo_add" => trf(lang, "ev.todo-add", &[("type", g("card_type")), ("text", g("text"))]),
        other => format!("component interaction: {other}"), // 未知 action 原样留痕（不静默丢）
    }
}

/// 生命周期事件生产（五类：created / closed_by_user / closed_by_agent / user_action / agent 更新不产事件）
pub trait Lifecycle {
    /// `card created: {type}「{title}」({id}) @ {YYMMDD-HH:MM}, → 存活 N`
    fn created_line(&self, meta: &CardMeta, alive: usize) -> String;
    /// `card closed: {type}「{title}」({id}), {start} / {end}, → 存活 N`
    fn closed_line(&self, meta: &CardMeta, alive: usize, end: i64) -> String;
    /// `用户关闭了 {type}「{title}」({id})`
    fn user_close_line(&self, meta: &CardMeta) -> String;
    /// `用户{勾选了/取消了/新增了} {type} 条目「{text}」`
    fn user_action_line(&self, typ: &str, action: &str, text: &str) -> String;
}

/// 默认生命周期生产器。事件文字按 Harness 语言现写（docs/i18n.md：事件发生时刻的
/// 语言生效，此后成为历史记录不被改写）
pub struct DefaultLifecycle {
    lang: crate::i18n::Lang,
}

impl DefaultLifecycle {
    pub fn for_lang(lang: crate::i18n::Lang) -> Self {
        Self { lang }
    }
}

impl Default for DefaultLifecycle {
    fn default() -> Self {
        Self::for_lang(crate::i18n::Lang::Zh)
    }
}

/// 时间格式 `YYMMDD-HH:MM`（UTC，确定性）
fn fmt_ts(ts: i64) -> String {
    let secs = ts / 1000;
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m) = (rem / 3600, rem % 3600 / 60);
    let (mut y, mut d) = (1970i64, days);
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let yd = if leap { 366 } else { 365 };
        if d < yd {
            break;
        }
        d -= yd;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 0usize;
    for (i, &md) in months.iter().enumerate() {
        if d < md {
            mo = i + 1;
            break;
        }
        d -= md;
    }
    format!("{:02}{:02}{:02}-{:02}:{:02}", y % 100, mo, d + 1, h, m)
}

impl Lifecycle for DefaultLifecycle {
    fn created_line(&self, meta: &CardMeta, alive: usize) -> String {
        crate::i18n::trf(
            self.lang,
            "lifecycle.created",
            &[
                ("type", meta.typ.clone()),
                ("title", meta.title.clone()),
                ("id", meta.id.clone()),
                ("ts", fmt_ts(meta.created)),
                ("n", alive.to_string()),
            ],
        )
    }

    fn closed_line(&self, meta: &CardMeta, alive: usize, end: i64) -> String {
        crate::i18n::trf(
            self.lang,
            "lifecycle.closed",
            &[
                ("type", meta.typ.clone()),
                ("title", meta.title.clone()),
                ("id", meta.id.clone()),
                ("start", fmt_ts(meta.created)),
                ("end", fmt_ts(end)),
                ("n", alive.to_string()),
            ],
        )
    }

    fn user_close_line(&self, meta: &CardMeta) -> String {
        crate::i18n::trf(
            self.lang,
            "lifecycle.user-close",
            &[("type", meta.typ.clone()), ("title", meta.title.clone()), ("id", meta.id.clone())],
        )
    }

    fn user_action_line(&self, typ: &str, action: &str, text: &str) -> String {
        crate::i18n::trf(
            self.lang,
            "lifecycle.user-action",
            &[("action", action.to_string()), ("type", typ.to_string()), ("text", text.to_string())],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> CardMeta {
        CardMeta {
            id: "todo-1".into(),
            typ: "todobox".into(),
            title: "发布清单".into(),
            created: 1_785_400_000_000, // 固定时刻
        }
    }

    #[test]
    fn lines_match_protocol_format() {
        let lc = DefaultLifecycle::default();
        let c = lc.created_line(&meta(), 3);
        assert!(c.starts_with("card created: todobox「发布清单」(todo-1) @ "));
        assert!(c.ends_with(", → 存活 3"));
        assert!(c.contains("-") && c.contains(":"));
        let x = lc.closed_line(&meta(), 2, 1_785_403_600_000);
        assert!(x.starts_with("card closed: todobox「发布清单」(todo-1), "));
        assert!(x.contains(" / "));
        assert!(x.ends_with(", → 存活 2"));
        assert_eq!(lc.user_close_line(&meta()), "用户关闭了 todobox「发布清单」(todo-1)");
        assert_eq!(
            lc.user_action_line("todobox", "勾选了", "跑测试"),
            "用户勾选了 todobox 条目「跑测试」"
        );
    }
}
