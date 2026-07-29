//! case-runner（docs/case-runner.md）：概念结构观测与 step 执行。
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
    CaseObserve { agents, panorama, context, content, queue, event_buffer }
}

/// Case JSON 结构
#[derive(Debug, Deserialize)]
pub struct CaseFile {
    pub meta: CaseMeta,
    pub data: CaseData,
    #[serde(default)]
    pub steps: Vec<CaseStep>,
}

#[derive(Debug, Deserialize)]
pub struct CaseMeta {
    pub case_id: String,
    pub created: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Deserialize)]
pub struct CaseData {
    #[serde(default)]
    pub work_agents: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub queue: String,
    #[serde(default)]
    pub mock_terminals: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub config: Value,
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
            CaseStep::Observe { .. } => "observe",
        }
    }
}
