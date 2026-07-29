//! case-runner（docs/case-runner.md）：概念结构观测与 step 执行。
//! feature "case-runner" gate。

use crate::server::AppState;
use crate::AgentStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 概念结构观测快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseObserve {
    pub agents: Vec<AgentSnapshot>,
    pub panorama: Option<String>,
    pub content_rows: usize,
    pub content_latest_ts: Option<i64>,
    pub context_rows: usize,
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

/// 观测当前概念结构
pub fn observe(state: &AppState) -> CaseObserve {
    let ov = state.overseer().blocking_lock();
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
    let cnt_records = ov.harness.content.records();
    let content_rows = cnt_records.len();
    let content_latest_ts = cnt_records.last().map(|r| r.ts);
    let context_rows = ov.harness.context.messages().len();
    let event_buffer_count = ov.harness.event_buffer.len();
    CaseObserve { agents, panorama, content_rows, content_latest_ts, context_rows, event_buffer: vec![format!("{} events", event_buffer_count)] }
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
