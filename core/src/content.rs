//! Terminal Content（§11）：原文存档（terminal-content.jsonl，
//! Filter 前）+ Filtered 内容（归一全文，**不持久化**——从原文 digest 现算，
//!。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordSource {
    Hook,
    Timer,
    FetchTerminal,
}

/// Filtered 内容：Filter 后归一全文，agent 实际读到的终端内容。
/// 不持久化——由 terminal-content.jsonl 原文 digest 现算（变化检测 prev 存内存，重启丢）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilteredContent {
    pub instance: String,
    pub filtered_content: String,
    pub source: RecordSource,
    pub ts: i64,
}

/// Terminal Content 原文存档记录（terminal-content.jsonl，Filter 前）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalContentRecord {
    pub instance: String,
    pub raw: String,
    pub source: RecordSource,
    pub ts: i64,
}
