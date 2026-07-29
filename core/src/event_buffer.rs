//! Event Buffer（concepts §10e）：Component 交互事件暂存区，与 Queue 并列。
//! LLM 触发时合并为一条 system message 注入 Queue，然后清空。永不写 user role。

#[derive(Default)]
pub struct EventBuffer {
    events: Vec<String>,
}

impl EventBuffer {
    pub fn push(&mut self, desc: impl Into<String>) {
        self.events.push(desc.into());
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// 积压事件原文（case-runner observe 用，只读）
    pub fn events(&self) -> &[String] {
        &self.events
    }

    /// 合并 + 清空；空时返回 None（不注入空消息）
    pub fn merge_and_clear(&mut self) -> Option<String> {
        if self.events.is_empty() {
            return None;
        }
        let merged = format!(
            "Component 交互事件：\n{}",
            self.events
                .iter()
                .map(|e| format!("- {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        self.events.clear();
        Some(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_formats_and_clears() {
        let mut buf = EventBuffer::default();
        buf.push("用户关闭了 text_card「构建结果」");
        buf.push("用户勾选了 todobox 条目「跑测试」");
        let merged = buf.merge_and_clear().unwrap();
        assert!(merged.contains("用户关闭了 text_card「构建结果」"));
        assert!(merged.contains("用户勾选了 todobox 条目「跑测试」"));
        assert!(merged.starts_with("Component 交互事件："));
        assert!(buf.is_empty());
    }

    #[test]
    fn empty_merge_returns_none() {
        let mut buf = EventBuffer::default();
        assert!(buf.merge_and_clear().is_none());
    }
}
