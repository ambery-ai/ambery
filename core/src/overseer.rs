//! Overseer（concepts §1）：触发循环 + tool 执行 + hook 处理（docs/agent-loop.md）。

use crate::context::RecordSource;
use crate::llm::{tool_set, Llm};
use crate::queue::{QueueMessage, Role, ToolCall};
use crate::{AgentEntry, AgentStatus, Config, ContextRecord, Harness, KaomojiEntry};
use serde_json::{json, Value};

/// 副作用：经 WS 广播给前端（docs/agent-loop.md §协议）
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    RenderComponent(Value),
    SetAutonomy {
        face: Option<String>,
        motion: Option<String>,
        ttl_ms: Option<u64>,
    },
    ConfigChanged,
}

pub struct Overseer<L: Llm> {
    pub harness: Harness,
    pub config: Config,
    llm: L,
    max_tool_iters: usize,
}

impl<L: Llm> Overseer<L> {
    pub fn new(harness: Harness, config: Config, llm: L) -> Self {
        Self {
            harness,
            config,
            llm,
            max_tool_iters: 8, // 防 tool 循环死转
        }
    }

    /// 拼装 system prefix（concepts §12：Config 引用的各概念数据运行时拼装）
    fn build_prefix(&self) -> String {
        let mut s = self.config.base_prompt.clone();
        s.push_str("\n\n## 颜文字映射\n");
        let mut keys: Vec<_> = self.config.kaomoji.keys().collect();
        keys.sort();
        for k in keys {
            let v = &self.config.kaomoji[k];
            s.push_str(&format!("- {k}: {} ({})\n", v.face, v.motion));
        }
        s.push_str("\n## 当前实例状态\n");
        if self.harness.agents.is_empty() {
            s.push_str("（无实例）\n");
        }
        for a in &self.harness.agents {
            s.push_str(&format!("- {} [{:?}] project={}\n", a.name, a.status, a.project));
        }
        s
    }

    /// 一轮触发（docs/agent-loop.md §一轮触发）
    pub async fn run_trigger(&mut self, ts: i64) -> std::io::Result<Vec<Effect>> {
        // 1. merge Event Buffer → 一条 system message
        if let Some(merged) = self.harness.event_buffer.merge_and_clear() {
            self.harness
                .append_queue(QueueMessage::new(Role::System, merged, ts))?;
        }
        // 2. 替换式更新 system prefix
        let prefix = self.build_prefix();
        self.harness.queue.replace_prefix(prefix, ts);
        // 3. Compression：超阈值 → stub 摘要 + shaking
        if self.harness.queue.needs_compression() {
            let summary = stub_summary(self.harness.queue.messages());
            self.harness.queue.compress(summary, ts);
        }
        // 4. tool 循环
        let tools = tool_set();
        let mut effects = vec![];
        for _ in 0..self.max_tool_iters {
            let out = self.llm.complete(self.harness.queue.messages(), &tools).await;
            if out.tool_calls.is_empty() {
                // 沉默语义：空 content 不追加（docs/agent-loop.md）
                if let Some(content) = out.content.filter(|c| !c.is_empty()) {
                    self.harness
                        .append_queue(QueueMessage::new(Role::Assistant, content, ts))?;
                }
                break;
            }
            self.harness.append_queue(QueueMessage::assistant_tool_calls(
                out.tool_calls.clone(),
                ts,
            ))?;
            for call in &out.tool_calls {
                let (result, mut eff) = self.execute_tool(call);
                effects.append(&mut eff);
                self.harness
                    .append_queue(QueueMessage::tool_result(&call.id, result.to_string(), ts))?;
            }
        }
        Ok(effects)
    }

    /// mock hook（docs/agent-loop.md §Mock Hook 契约）
    pub async fn handle_hook(
        &mut self,
        event: &str,
        instance: &str,
        project: &str,
        content: &str,
        ts: i64,
    ) -> std::io::Result<Vec<Effect>> {
        match event {
            "session_start" => {
                self.harness.upsert_agent(AgentEntry {
                    id: instance.into(),
                    name: instance.into(),
                    project: project.into(),
                    status: AgentStatus::Processing,
                    first_seen: ts,
                    last_seen: ts,
                })?;
                self.harness.append_context(ContextRecord {
                    instance: instance.into(),
                    content: content.into(),
                    source: RecordSource::Hook,
                    ts,
                })?;
                self.harness.append_queue(QueueMessage::new(
                    Role::System,
                    format!("新实例 {instance} 已注册"),
                    ts,
                ))?;
            }
            "stop" => {
                let first_seen = self
                    .harness
                    .agents
                    .iter()
                    .find(|a| a.id == instance)
                    .map(|a| a.first_seen)
                    .unwrap_or(ts);
                self.harness.upsert_agent(AgentEntry {
                    id: instance.into(),
                    name: instance.into(),
                    project: project.into(),
                    status: AgentStatus::Idle,
                    first_seen,
                    last_seen: ts,
                })?;
                self.harness.append_context(ContextRecord {
                    instance: instance.into(),
                    content: content.into(),
                    source: RecordSource::Hook,
                    ts,
                })?;
                let len = content.chars().count();
                self.harness.append_queue(QueueMessage::new(
                    Role::System,
                    format!("{instance} 完成，Context 已更新（{len} 字）。评估是否通知。"),
                    ts,
                ))?;
            }
            _ => {}
        }
        self.run_trigger(ts).await
    }

    fn execute_tool(&mut self, call: &ToolCall) -> (Value, Vec<Effect>) {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
        match call.name.as_str() {
            "call_component" => {
                let spec = args.get("spec").cloned().unwrap_or(Value::Null);
                let id = spec
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                (
                    json!({ "ok": true, "rendered": id }),
                    vec![Effect::RenderComponent(spec)],
                )
            }
            "fetch_terminal" => {
                let inst = args.get("instance").and_then(Value::as_str).unwrap_or("");
                let content = self
                    .harness
                    .context
                    .latest(inst)
                    .map(|r| r.content.clone())
                    .unwrap_or_else(|| "（无记录）".into());
                (json!({ "instance": inst, "content": content }), vec![])
            }
            "set_autonomy" => (
                json!({ "ok": true }),
                vec![Effect::SetAutonomy {
                    face: args.get("face").and_then(Value::as_str).map(String::from),
                    motion: args
                        .get("motion")
                        .and_then(Value::as_str)
                        .map(String::from),
                    ttl_ms: args.get("ttlMs").and_then(Value::as_u64),
                }],
            ),
            "edit_config" => {
                let key = args.get("key").and_then(Value::as_str).unwrap_or("");
                let face = args.get("face").and_then(Value::as_str);
                let motion = args.get("motion").and_then(Value::as_str);
                if face.is_none() && motion.is_none() {
                    return (
                        json!({ "ok": false, "error": "face/motion 至少传一个" }),
                        vec![],
                    );
                }
                let entry = self
                    .config
                    .kaomoji
                    .entry(key.to_string())
                    .or_insert_with(|| KaomojiEntry {
                        face: String::new(),
                        motion: "still".into(),
                    });
                if let Some(f) = face {
                    entry.face = f.to_string();
                }
                if let Some(m) = motion {
                    entry.motion = m.to_string();
                }
                let saved = self.config.save(self.harness.storage_dir()).is_ok();
                (
                    json!({ "ok": saved, "key": key }),
                    vec![Effect::ConfigChanged],
                )
            }
            other => (
                json!({ "ok": false, "error": format!("unknown tool: {other}") }),
                vec![],
            ),
        }
    }
}

/// debug 模式压缩摘要 stub（真实 LLM 摘要随真实 API 接入）
fn stub_summary(messages: &[QueueMessage]) -> String {
    let n = messages.len();
    let first = messages
        .get(1)
        .and_then(|m| m.content.as_deref())
        .unwrap_or("")
        .chars()
        .take(20)
        .collect::<String>();
    let last = messages
        .last()
        .and_then(|m| m.content.as_deref())
        .unwrap_or("")
        .chars()
        .take(20)
        .collect::<String>();
    format!("共 {n} 条历史：首「{first}」末「{last}」")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::DebugAgent;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("overseer-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn make_overseer(tag: &str) -> Overseer<DebugAgent> {
        let dir = tmp_dir(tag);
        let harness = Harness::load(&dir, "PREFIX".into(), 100_000, 0).unwrap();
        Overseer::new(harness, Config::default(), DebugAgent::default())
    }

    #[tokio::test]
    async fn stop_long_content_notifies() {
        let mut ov = make_overseer("notify");
        let long = "x".repeat(120);
        let effects = ov.handle_hook("stop", "ft", "proj", &long, 1).await.unwrap();
        assert!(effects.iter().any(|e| matches!(e, Effect::RenderComponent(_))));
        assert!(effects.iter().any(|e| matches!(e, Effect::SetAutonomy { .. })));
        let roles: Vec<Role> = ov.harness.queue.messages().iter().map(|m| m.role).collect();
        // prefix + system(hook) + assistant(tool_calls) + tool + tool
        assert_eq!(
            roles,
            vec![Role::System, Role::System, Role::Assistant, Role::Tool, Role::Tool]
        );
        // agent 注册为 idle
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Idle);
        let _ = std::fs::remove_dir_all(tmp_dir("notify"));
    }

    #[tokio::test]
    async fn stop_short_content_silence() {
        let mut ov = make_overseer("silence");
        let effects = ov.handle_hook("stop", "oss", "proj", "清理了 2 行注释", 1).await.unwrap();
        assert!(effects.is_empty());
        let roles: Vec<Role> = ov.harness.queue.messages().iter().map(|m| m.role).collect();
        // prefix + system(hook)，沉默不追加 assistant
        assert_eq!(roles, vec![Role::System, Role::System]);
        let _ = std::fs::remove_dir_all(tmp_dir("silence"));
    }

    #[tokio::test]
    async fn session_start_registers_and_triggers() {
        let mut ov = make_overseer("register");
        ov.handle_hook("session_start", "new-feature", "proj", "启动画面", 1)
            .await
            .unwrap();
        assert_eq!(ov.harness.agents.len(), 1);
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Processing);
        assert!(ov
            .harness
            .queue
            .messages()
            .iter()
            .any(|m| m.content.as_deref() == Some("新实例 new-feature 已注册")));
        let _ = std::fs::remove_dir_all(tmp_dir("register"));
    }

    #[tokio::test]
    async fn user_followup_triggers_fetch_loop() {
        let mut ov = make_overseer("fetch");
        let long = "y".repeat(100);
        ov.handle_hook("stop", "ft", "proj", &long, 1).await.unwrap();
        ov.harness
            .append_queue(QueueMessage::new(Role::User, "那个 bug 具体怎么回事？", 2))
            .unwrap();
        ov.run_trigger(3).await.unwrap();
        let msgs = ov.harness.queue.messages();
        // fetch_terminal 被执行，tool result 含 Context 全文
        assert!(msgs.iter().any(|m| m.role == Role::Tool
            && m.content.as_deref().unwrap_or("").contains(&"y".repeat(100))));
        // 最终 assistant 汇总
        assert_eq!(msgs.last().unwrap().role, Role::Assistant);
        assert!(msgs
            .last()
            .unwrap()
            .content
            .as_deref()
            .unwrap_or("")
            .starts_with("[debug] 查到："));
        let _ = std::fs::remove_dir_all(tmp_dir("fetch"));
    }

    #[tokio::test]
    async fn event_buffer_merged_on_trigger() {
        let mut ov = make_overseer("merge");
        ov.harness.event_buffer.push("用户关闭了 text_card「摘要」");
        ov.harness.event_buffer.push("用户勾选了 todobox 条目「跑测试」");
        ov.run_trigger(1).await.unwrap();
        let sys: Vec<_> = ov
            .harness
            .queue
            .messages()
            .iter()
            .filter(|m| m.role == Role::System)
            .collect();
        // prefix + 合并的一条
        assert_eq!(sys.len(), 2);
        let merged = sys[1].content.as_deref().unwrap();
        assert!(merged.contains("用户关闭了 text_card「摘要」"));
        assert!(merged.contains("用户勾选了 todobox 条目「跑测试」"));
        assert!(ov.harness.event_buffer.is_empty());
        let _ = std::fs::remove_dir_all(tmp_dir("merge"));
    }

    #[tokio::test]
    async fn plain_user_message_replies() {
        let mut ov = make_overseer("reply");
        ov.harness
            .append_queue(QueueMessage::new(Role::User, "你好", 1))
            .unwrap();
        ov.run_trigger(2).await.unwrap();
        let last = ov.harness.queue.messages().last().unwrap();
        assert_eq!(last.role, Role::Assistant);
        assert!(last.content.as_deref().unwrap_or("").contains("[debug] 收到：你好"));
        let _ = std::fs::remove_dir_all(tmp_dir("reply"));
    }

    #[tokio::test]
    async fn edit_config_updates_and_persists() {
        let mut ov = make_overseer("cfg");
        let call = ToolCall {
            id: "c1".into(),
            name: "edit_config".into(),
            arguments: json!({ "key": "celebrate", "face": "(≧▽≦)", "motion": "bounce" }).to_string(),
        };
        let (result, effects) = ov.execute_tool(&call);
        assert_eq!(result["ok"], json!(true));
        assert!(effects.contains(&Effect::ConfigChanged));
        assert_eq!(ov.config.kaomoji["celebrate"].face, "(≧▽≦)");
        // config.json 已持久化
        let reloaded = Config::load_or_default(ov.harness.storage_dir());
        assert_eq!(reloaded.kaomoji["celebrate"].motion, "bounce");
        let _ = std::fs::remove_dir_all(tmp_dir("cfg"));
    }
}
