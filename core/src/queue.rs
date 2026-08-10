//! Queue（concepts §10c，docs/harness.md）：输入串行化关口——只装输入
//! （hook 内容 = system 输入；user 消息 = user 输入），串行放行一轮一条：
//! 放行 → Context 写输入 → LLM → 输出写 Context → 放行下一条。
//! assistant/tool 输出不走 Queue，直接入 Context。
//! queue.jsonl 留痕（排队轨迹非对话本体；崩溃丢失未放行输入可接受，docs/storage.md）。

use crate::context::Role;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Queue 输入来源（docs/harness.md §Queue 规则 2；docs/concrete-insight.md §Queue 中的
/// System 消息来源）：触发这次输入的语义原因——effort 档位、双队列优先级等
/// 按来源定行为的机制的一等公民
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueSource {
    /// 用户 chat 面板直接发送（User 输入）
    UserChat,
    /// stop queue_only 产物（hint）/ auto_read 读失败回落
    HookStopHint,
    /// stop auto_read 产物（filter 后全量内容）
    HookStopContent,
    /// stop message 产物（汇报原文）
    HookStopReport,
    /// UserPromptSubmit 观察注入
    HookUserPrompt,
    /// Notification 事件注入
    HookNotification,
    /// debug/测试注入（mock hook）
    MockHook,
    /// Timer 兜底扫描产物
    TimerScan,
    /// Cron 计划到期消息
    CronTick,
}

/// 旧数据/case 兼容：缺失 source 的 queue.jsonl 行反序列化为 mock_hook
/// （案例注入的诚实来源）
impl Default for QueueSource {
    fn default() -> Self {
        Self::MockHook
    }
}

/// Queue 输入条目：role 仅 System（hook 内容）/ User（用户消息）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueInput {
    pub role: Role,
    pub content: String,
    #[serde(default)]
    pub source: QueueSource,
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
            source: QueueSource::MockHook,
            ts,
        }
    }

    #[test]
    fn queue_input_source_serde_compat() {
        // 旧 queue.jsonl 行（无 source 字段）→ mock_hook（docs/harness.md §Queue 规则 2）
        let v: QueueInput = serde_json::from_str(r#"{"role":"system","content":"x","ts":1}"#).unwrap();
        assert_eq!(v.source, QueueSource::MockHook);
        // 落盘携带 source（snake_case）
        let s = serde_json::to_string(&input("y", 2)).unwrap();
        assert!(s.contains("\"source\":\"mock_hook\""), "{s}");
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
