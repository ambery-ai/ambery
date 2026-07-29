//! Queue（concepts §10c，docs/harness.md）：输入串行化关口——只装输入
//! （hook 内容 = system 输入；user 消息 = user 输入），串行放行一轮一条：
//! 放行 → Context 写输入 → LLM → 输出写 Context → 放行下一条。
//! assistant/tool 输出不走 Queue，直接入 Context。
//! queue.jsonl 留痕（排队轨迹非对话本体；崩溃丢失未放行输入可接受，docs/storage.md）。

use crate::context::Role;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Queue 输入条目：role 仅 System（hook 内容）/ User（用户消息）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueInput {
    pub role: Role,
    pub content: String,
    pub ts: i64,
}

#[derive(Default)]
pub struct Queue {
    pending: VecDeque<QueueInput>,
}

impl Queue {
    /// 入队（尾部追加）
    pub fn enqueue(&mut self, input: QueueInput) {
        self.pending.push_back(input);
    }

    /// 放行一条（取走即消费，FIFO）
    pub fn release(&mut self) -> Option<QueueInput> {
        self.pending.pop_front()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// 待放行输入（case-runner observe 用，只读）
    pub fn iter(&self) -> impl Iterator<Item = &QueueInput> {
        self.pending.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(content: &str, ts: i64) -> QueueInput {
        QueueInput {
            role: Role::System,
            content: content.into(),
            ts,
        }
    }

    #[test]
    fn fifo_release_order() {
        let mut q = Queue::default();
        q.enqueue(input("第一条", 1));
        q.enqueue(input("第二条", 2));
        assert_eq!(q.len(), 2);
        assert_eq!(q.release().unwrap().content, "第一条");
        assert_eq!(q.release().unwrap().content, "第二条");
        assert!(q.release().is_none());
        assert!(q.is_empty());
    }

    #[test]
    fn enqueue_during_processing_queues_up() {
        // 处理中到来 → 排队等待（concepts §10c）
        let mut q = Queue::default();
        let first = q.enqueue(input("处理中", 1));
        let _ = first;
        q.enqueue(input("等待", 2));
        assert_eq!(q.len(), 2); // 未放行前不减少
    }
}
