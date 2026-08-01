//! case-runner（docs/case-runner.md）：两段式 .case 解析、step 类型与概念观测。
//! feature "case-runner" gate。

use crate::llm::Llm;
use crate::overseer::OverseerBackend;
use crate::AgentStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 概念结构观测快照（内容级，docs/case-runner.md §observe 输出）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseObserve {
    pub agents: Vec<AgentSnapshot>,
    pub panorama: Option<String>,
    /// Context 消息数组（role / content / tool_calls）
    pub context: Vec<MessageSnapshot>,
    /// Content 存档（Filter 后归一全文参考数据）
    pub content: Vec<ContentSnapshot>,
    /// Queue 待放行输入
    pub queue: Vec<crate::queue::QueueInput>,
    /// Event Buffer 积压原文
    pub event_buffer: Vec<String>,
    /// 最近一次 LLM 调用真值（#16；无 = 未调用过/重启后）
    pub usage: Option<crate::llm::Usage>,
    /// 自真值落点后的 est 增量（无真值时 = 全量 est）
    pub context_est_delta: usize,
    /// 最后一条 assistant 消息原文（回答准确度扫读位）
    pub answer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub hash: String,
    pub name: String,
    pub project: String,
    pub status: AgentStatus,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSnapshot {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: usize,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentSnapshot {
    pub instance: String,
    pub content: String,
    pub source: String,
    pub ts: i64,
}

/// 观测当前概念结构
pub fn observe<L: Llm>(ov: &OverseerBackend<L>) -> CaseObserve {
    let agents: Vec<AgentSnapshot> = ov
        .harness
        .agents
        .iter()
        .map(|a| AgentSnapshot {
            hash: a.hash.clone(),
            name: a.name.clone(),
            project: a.project.clone(),
            status: a.status.clone(),
            last_seen: a.last_seen,
        })
        .collect();
    let panorama = crate::panorama(&ov.harness.agents);
    let context = ov
        .harness
        .context
        .messages()
        .iter()
        .map(|m| MessageSnapshot {
            role: format!("{:?}", m.role).to_lowercase(),
            content: m.content.clone(),
            tool_calls: m.tool_calls.as_ref().map_or(0, |c| c.len()),
            ts: m.ts,
        })
        .collect();
    let content = ov
        .harness
        .content
        .records()
        .iter()
        .map(|r| ContentSnapshot {
            instance: r.instance.clone(),
            content: r.content.clone(),
            source: format!("{:?}", r.source).to_lowercase(),
            ts: r.ts,
        })
        .collect();
    let queue = ov.harness.queue.iter().cloned().collect();
    let event_buffer = ov.harness.event_buffer.events().to_vec();
    let usage = ov.harness.last_usage;
    let context_est_delta = ov
        .harness
        .context
        .est_tokens_since(ov.harness.last_usage_msg_len);
    let answer = ov
        .harness
        .context
        .messages()
        .iter()
        .rev()
        .find(|m| m.role == crate::context::Role::Assistant)
        .and_then(|m| m.content.clone());
    CaseObserve { agents, panorama, context, content, queue, event_buffer, usage, context_est_delta, answer }
}

// ── 两段式 .case 格式（docs/case-runner.md §Case 文件格式）──

/// 解析规则（唯一一条）：首个无缩进 `{"__section":` 行之前 = JSON 头；
/// 之后按 marker 行分节，数据行归当前节。
pub const SECTION_MARKER: &str = "{\"__section\":";

#[derive(Debug)]
pub struct CaseFile {
    pub meta: CaseMeta,
    pub config: Value,
    pub steps: Vec<CaseStep>,
    /// 数据节原文（行序原样）
    pub work_agents: String,
    pub context: String,
    pub queue: String,
}

#[derive(Debug, Deserialize)]
pub struct CaseMeta {
    pub case_id: String,
    pub created: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Deserialize)]
struct CaseHead {
    meta: CaseMeta,
    #[serde(default)]
    config: Value,
    #[serde(default)]
    steps: Vec<CaseStep>,
}

/// marker 行 → 节名（`{"__section":"=== work_agents ==="}` → `work_agents`）
fn section_name(line: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("marker 行非法 JSON: {e}"))?;
    let raw = v["__section"]
        .as_str()
        .ok_or_else(|| "marker 行缺 __section 字符串".to_string())?;
    Ok(raw.trim_matches(['=', ' ']).to_string())
}

pub fn parse(text: &str) -> Result<CaseFile, String> {
    let mut head_lines: Vec<&str> = vec![];
    let mut sections: Vec<(String, Vec<String>)> = vec![];
    for line in text.lines() {
        if line.starts_with(SECTION_MARKER) {
            sections.push((section_name(line)?, vec![]));
        } else if sections.is_empty() {
            head_lines.push(line);
        } else if !line.trim().is_empty() {
            sections.last_mut().unwrap().1.push(line.to_string());
        }
    }
    let head: CaseHead = serde_json::from_str(&head_lines.join("\n"))
        .map_err(|e| format!("JSON 头解析失败: {e}"))?;
    let take = |name: &str| -> String {
        sections
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, rows)| rows.join("\n"))
            .unwrap_or_default()
    };
    Ok(CaseFile {
        meta: head.meta,
        config: head.config,
        steps: head.steps,
        work_agents: take("work_agents"),
        context: take("context"),
        queue: take("queue"),
    })
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CaseStep {
    Cmd { load: Value },
    Cmd2 { timer_scan: Value },
    Cmd3 { hook: Value },
    Cmd4 { trigger: Value },
    Cmd5 { user: Value },
    Cmd6 { tool_call: Vec<String> },
    Terminal { terminal: Value },
    TerminalGone { terminal_gone: Value },
    Observe { observe: Vec<String> },
}

impl CaseStep {
    /// 返回步骤名
    pub fn name(&self) -> &'static str {
        match self {
            CaseStep::Cmd { .. } => "load",
            CaseStep::Cmd2 { .. } => "timer_scan",
            CaseStep::Cmd3 { .. } => "hook",
            CaseStep::Cmd4 { .. } => "trigger",
            CaseStep::Cmd5 { .. } => "user",
            CaseStep::Cmd6 { .. } => "tool_call",
            CaseStep::Terminal { .. } => "terminal",
            CaseStep::TerminalGone { .. } => "terminal_gone",
            CaseStep::Observe { .. } => "observe",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_two_stage_format() {
        let text = r#"{
  "meta": { "case_id": "t", "created": "2026-07-30T00:00:00Z" },
  "config": { "timer.interval_ms": 5000 },
  "steps": [ { "load": {} }, { "terminal_gone": { "instance": "ft" } } ]
}
{"__section":"=== work_agents ==="}
{"hash":"a1","name":"ft","status":"processing"}
{"__section":"=== context ==="}
{"type":"content","instance":"ft","content":"旧内容","ts":1}
{"__section":"=== queue ==="}
"#;
        let case = parse(text).unwrap();
        assert_eq!(case.meta.case_id, "t");
        assert_eq!(case.steps.len(), 2);
        assert_eq!(case.steps[1].name(), "terminal_gone");
        assert_eq!(case.work_agents.lines().count(), 1);
        assert!(case.work_agents.contains("a1"));
        assert_eq!(case.context.lines().count(), 1);
        assert!(case.queue.is_empty());
    }

    #[test]
    fn parse_empty_sections_kept_distinct() {
        let text = "{\n \"meta\": {\"case_id\":\"t\",\"created\":\"x\"}\n}\n{\"__section\":\"=== work_agents ===\"}\n{\"__section\":\"=== context ===\"}\n{\"type\":\"message\",\"role\":\"system\",\"content\":\"hi\",\"ts\":1}\n";
        let case = parse(text).unwrap();
        assert!(case.work_agents.is_empty()); // 空节 ≠ 缺节
        assert_eq!(case.context.lines().count(), 1);
    }
}
