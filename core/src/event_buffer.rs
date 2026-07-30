//! Event Buffer（concepts §10e，docs/harness.md §Event Buffer 双载荷）：
//! Component 交互事件暂存区，与 Queue 平行。每条事件 = 自然语言（必填）+ 结构化快照
//! （可选，todobox 类交互附带）。Queue 放行时合并为一条 system message 入 Context，
//! 然后清空。永不写 user role；原始条目不持久化。

use serde_json::Value;

/// 一条缓冲事件：自然语言描述 + 可选结构化状态快照（docs/harness.md §双载荷）
#[derive(Debug, Clone, PartialEq)]
pub struct BufferedEvent {
    pub desc: String,
    pub state: Option<Value>,
}

#[derive(Default)]
pub struct EventBuffer {
    entries: Vec<BufferedEvent>,
}

impl EventBuffer {
    pub fn push(&mut self, desc: impl Into<String>) {
        self.entries.push(BufferedEvent {
            desc: desc.into(),
            state: None,
        });
    }

    /// 双载荷：自然语言 + 结构化快照（todobox 交互）
    pub fn push_with_state(&mut self, desc: impl Into<String>, state: Value) {
        self.entries.push(BufferedEvent {
            desc: desc.into(),
            state: Some(state),
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 自然语言描述（case-runner observe 用，只读）
    pub fn events(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.desc.clone()).collect()
    }

    /// 合并 + 清空；空时返回 None（不注入空消息）。
    /// 自然语言逐条保留；结构化快照**同 card 去重合并**（按 state.id，最后 wins，
    /// 单次 flush 内一 card 一份最终状态，docs/harness.md §去重合并）。
    pub fn merge_and_clear(&mut self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let mut out = String::from("Component 交互事件：");
        // 结构化快照去重合并：id → 最后一份（HashMap 语义直白版）
        let mut order: Vec<String> = vec![];
        let mut map: std::collections::HashMap<String, &Value> = std::collections::HashMap::new();
        for e in &self.entries {
            out.push_str(&format!("\n- {}", e.desc));
            if let Some(state) = &e.state {
                let id = state["id"].as_str().unwrap_or("").to_string();
                if !map.contains_key(&id) {
                    order.push(id.clone());
                }
                map.insert(id, state); // 后到覆盖 = 最终状态
            }
        }
        if !order.is_empty() {
            out.push_str("\n结构化状态：");
            for id in &order {
                out.push_str(&format!("\n{}", serde_json::to_string(map[id]).unwrap_or_default()));
            }
        }
        self.entries.clear();
        Some(out)
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

    #[test]
    fn snapshots_dedupe_per_card_last_wins() {
        let mut buf = EventBuffer::default();
        buf.push_with_state(
            "用户勾选了 todobox 条目「a」",
            serde_json::json!({"id": "todo-1", "type": "todobox", "items": [{"text": "a", "done": true}, {"text": "b", "done": false}]}),
        );
        buf.push_with_state(
            "用户勾选了 todobox 条目「b」",
            serde_json::json!({"id": "todo-1", "type": "todobox", "items": [{"text": "a", "done": true}, {"text": "b", "done": true}]}),
        );
        buf.push_with_state(
            "用户勾选了 todobox 条目「x」",
            serde_json::json!({"id": "todo-2", "type": "todobox", "items": [{"text": "x", "done": true}]}),
        );
        let merged = buf.merge_and_clear().unwrap();
        assert!(merged.contains("结构化状态："));
        // todo-1 只有最后一份（b done=true）；todo-2 一份（Value 序列化 key 按字典序）
        assert!(merged.contains(r#""id":"todo-1""#));
        assert!(merged.contains(r#""done":true,"text":"b""#));
        assert!(!merged.contains(r#""done":false,"text":"b""#));
        assert!(merged.contains(r#""id":"todo-2""#));
        // 自然语言逐条都在
        assert!(merged.contains("用户勾选了 todobox 条目「a」"));
        assert!(merged.contains("用户勾选了 todobox 条目「b」"));
    }
}
