//! Context（concepts §10b）：ペット的外部信息注入通道，每条记录带时间戳，保留全量历史。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordSource {
    Hook,
    Timer,
    FetchTerminal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextRecord {
    pub instance: String,
    pub content: String,
    pub source: RecordSource,
    pub ts: i64,
}

/// Terminal Content 原文存档记录（terminal-content.jsonl，Filter 前，docs/storage.md）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalContentRecord {
    pub instance: String,
    pub raw: String,
    pub source: RecordSource,
    pub ts: i64,
}

#[derive(Default)]
pub struct Context {
    records: Vec<ContextRecord>,
}

impl Context {
    pub fn push(&mut self, rec: ContextRecord) {
        self.records.push(rec);
    }

    pub fn records(&self) -> &[ContextRecord] {
        &self.records
    }

    /// 某实例最近一条（追问「具体怎么回事」时取全文）
    pub fn latest(&self, instance: &str) -> Option<&ContextRecord> {
        self.records.iter().rev().find(|r| r.instance == instance)
    }

    /// 顶层状态概览：每实例最近一条，按首次出现顺序（concepts §4 状态来源）
    pub fn overview(&self) -> Vec<&ContextRecord> {
        let mut pos: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let mut out: Vec<&ContextRecord> = vec![];
        for r in &self.records {
            match pos.get(r.instance.as_str()) {
                Some(&i) => out[i] = r,
                None => {
                    pos.insert(r.instance.as_str(), out.len());
                    out.push(r);
                }
            }
        }
        out
    }

    pub fn from_records(records: Vec<ContextRecord>) -> Self {
        Self { records }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(instance: &str, content: &str, ts: i64) -> ContextRecord {
        ContextRecord {
            instance: instance.into(),
            content: content.into(),
            source: RecordSource::Hook,
            ts,
        }
    }

    #[test]
    fn latest_returns_most_recent() {
        let mut ctx = Context::default();
        ctx.push(rec("a", "old", 1));
        ctx.push(rec("b", "other", 2));
        ctx.push(rec("a", "new", 3));
        assert_eq!(ctx.latest("a").unwrap().content, "new");
        assert_eq!(ctx.latest("b").unwrap().content, "other");
        assert!(ctx.latest("c").is_none());
    }

    #[test]
    fn overview_keeps_first_seen_order_with_latest_content() {
        let mut ctx = Context::default();
        ctx.push(rec("a", "a1", 1));
        ctx.push(rec("b", "b1", 2));
        ctx.push(rec("a", "a2", 3));
        let ov = ctx.overview();
        assert_eq!(ov.len(), 2);
        assert_eq!(ov[0].instance, "a");
        assert_eq!(ov[0].content, "a2");
        assert_eq!(ov[1].instance, "b");
    }
}
