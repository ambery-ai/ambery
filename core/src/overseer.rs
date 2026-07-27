//! OverseerBackend（concepts §1）：触发循环 + tool 执行 + hook 处理（docs/agent-loop.md）。

use crate::context::RecordSource;
use crate::filter::{Change, Filter};
use crate::llm::{tool_set, Llm};
use crate::queue::{QueueMessage, Role, ToolCall};
use crate::timer::TimerWheel;
use crate::{
    default_agents_md, AgentEntry, AgentStatus, Config, ContextRecord, Harness,
    TerminalContentRecord, AGENTS_MD_FILE,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// 副作用：经 WS 广播给前端（docs/agent-loop.md §协议）
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    RenderComponent(Value),
    SetAutonomy {
        face: Option<String>,
        motion: Option<String>,
        ttl_ms: Option<u64>,
    },
    /// llm_changed=true 时 server 广播前重建 LlmBackend
    ConfigChanged { llm_changed: bool },
}

pub struct OverseerBackend<L: Llm> {
    pub harness: Harness,
    pub config: Config,
    llm: L,
    /// Filter（concepts §11）：Content → Context 链路上应用
    filter: Box<dyn Filter + Send>,
    /// Timer（concepts §1a）：每实例兜底扫描调度
    pub timers: TimerWheel,
    /// 读通道（docs/timer.md §Scanner）：sidecar 或 MockTerminals；fetch_terminal 优先于 Context
    pub terminal_reader: Option<Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>,
    /// sidecar 在读通道链中时，Timer 读到 None 才判定 tab 消亡（closed）；
    /// 纯 MockTerminals 下 None 只是「未注入」，不能当消亡证据（设计决定）
    pub sidecar_enabled: bool,
    max_tool_iters: usize,
}

impl<L: Llm> OverseerBackend<L> {
    pub fn new(harness: Harness, config: Config, llm: L) -> Self {
        let filter = crate::filter::by_name(&config.filter_strategy);
        let timers = TimerWheel::new(config.timer_interval_ms, config.timer_stagger_ms);
        Self {
            harness,
            config,
            llm,
            filter,
            timers,
            terminal_reader: None,
            sidecar_enabled: false,
            max_tool_iters: 8, // 防 tool 循环死转
        }
    }

    /// 统一配置修改管道（docs/config.md「修改入口」）：CLI/面板/LLM tool 共用。
    /// set_by_path 写入 → serde 反序列化验证 → 动态 enum 校验 → 热应用 → persist。
    /// restart_required = 运行时 diff 如实上报（不假装生效，行为即真相）。
    pub fn apply_config_by_path(
        &mut self,
        path: &str,
        value: Value,
    ) -> Result<ConfigOutcome, String> {
        if self.config.read_only {
            return Err("只读降级模式：config 写被禁止（docs/config.md）".into());
        }
        let mut v = serde_json::to_value(&self.config).map_err(|e| e.to_string())?;
        crate::config::reflect::set_by_path(&mut v, path, value.clone())?;
        let new: Config = serde_json::from_value(v).map_err(|e| format!("验证失败: {e}"))?;
        // 动态 enum 校验（OPTIONS 注册表，验证集中一份）
        if let (Some(opts), Value::String(s)) =
            (crate::config::reflect::valid_options(&new, path), &value)
        {
            if !opts.contains(s) {
                return Err(format!("{path}: '{s}' 不在合法选项 {opts:?} 中"));
            }
        }
        let old = std::mem::replace(&mut self.config, new);
        // 热应用：filter 重建 / queue 阈值同步（其余字段每轮现读，天然热）
        if self.config.filter_strategy != old.filter_strategy {
            self.filter = crate::filter::by_name(&self.config.filter_strategy);
        }
        if self.config.token_threshold != old.token_threshold {
            self.harness.queue.token_threshold = self.config.token_threshold;
        }
        let llm_changed = self.config.llm != old.llm;
        self.config
            .save(self.harness.config_dir())
            .map_err(|e| format!("persist 失败: {e}"))?;
        Ok(ConfigOutcome {
            effects: vec![Effect::ConfigChanged { llm_changed }],
            llm_changed,
            restart_required: if restart_required_for(path) {
                vec![path.to_string()]
            } else {
                vec![]
            },
        })
    }

    /// llm_changed 后由 server 重建具体 LlmBackend 注入（overseer 泛型擦除不认识它）
    pub fn replace_llm(&mut self, llm: L) {
        self.llm = llm;
    }

    /// 现拼 system prompt 请求头（concepts §12：Config 引用的各概念数据运行时拼装）
    /// = base_prompt（Config）+ AGENTS.md（Storage，热生效）+ kaomoji 表。
    /// 内容稳定、天然 cache 友好，不落 Queue（docs/storage.md）。
    fn assemble_system_prompt(&self) -> String {
        let mut s = self.config.base_prompt.clone();
        s.push_str("\n\n");
        s.push_str(&self.read_agents_md());
        // kaomoji 表为什么不放进 AGENTS.md：
        // ① 表体是运行时数据（edit_config 可改 kaomoji 映射），须每轮现拼保持最新，
        //    写死在 AGENTS.md 会变成两个真相源；
        // ② 段头用途说明与组装表共位（贴着表解释「这是什么」），且作为不变量护栏——
        //    AGENTS.md 是用户可编辑文件，说明写在那里可能被无意删改。
        //    AGENTS.md 行为准则里已有禁令散文，此处是贴着表的强化，故意重复。
        s.push_str("\n\n## 颜文字映射（你的面部表情词汇表：仅用于 set_autonomy 工具，严禁写进对话文本）\n");
        let mut keys: Vec<_> = self.config.kaomoji.keys().collect();
        keys.sort();
        for k in keys {
            let v = &self.config.kaomoji[k];
            s.push_str(&format!("- {k}: {} ({})\n", v.face, v.motion));
        }
        s
    }

    /// AGENTS.md 每轮现读（热生效：改完下一个触发就用）；读不到回退内置默认
    fn read_agents_md(&self) -> String {
        std::fs::read_to_string(self.harness.config_dir().join(AGENTS_MD_FILE))
            .unwrap_or_else(|_| default_agents_md())
    }

    /// 状态 key 推导（concepts §4：key 切换由后端根据 Hook/Timer 驱动）：
    /// notify（有未决通知）> processing（任一实例在跑）> idle。
    /// 返回 `[face: key, motion: key]`——写默认推导 key；覆盖状态 LLM 从自己的
    /// tool_calls 历史已知（设计决定）。
    fn state_key(&self, pending_notifications: usize) -> String {
        let key = if pending_notifications > 0 {
            "notify"
        } else if self
            .harness
            .agents
            .iter()
            .any(|a| a.status == AgentStatus::Processing)
        {
            "processing"
        } else {
            "idle"
        };
        let motion = self
            .config
            .kaomoji
            .get(key)
            .map(|k| k.motion.as_str())
            .unwrap_or("still");
        format!("[face: {key}, motion: {motion}]")
    }

    /// 一轮触发（docs/agent-loop.md §一轮触发）
    /// pending_notifications：未决通知数（server 层计数传入，推导 notify key 用）
    pub async fn run_trigger(
        &mut self,
        ts: i64,
        pending_notifications: usize,
    ) -> std::io::Result<Vec<Effect>> {
        // 1. merge Event Buffer → 一条 system message
        if let Some(merged) = self.harness.event_buffer.merge_and_clear() {
            self.harness
                .append_queue(QueueMessage::new(Role::System, merged, ts))?;
        }
        // 2. 现拼 system prompt 请求头（不落 Queue）；变化才写 head 快照（docs/storage.md）
        let head = self.assemble_system_prompt();
        if self.harness.last_head.as_deref() != Some(head.as_str()) {
            self.harness.log_head(head.clone(), ts)?;
        }
        // 3. Autonomy 状态：每轮一条写 context.jsonl，最新一条挂请求末端（concepts §4）
        let autonomy = self.state_key(pending_notifications);
        self.harness.log_autonomy(autonomy.clone(), ts)?;
        // 4. Compression（auto-compact，concepts §10d）：专项摘要调用 → 内存 shaking
        //    → compact_boundary 标记（文件不删历史）→ 归零重 diff 全景
        if self.harness.queue.needs_compression() {
            let pre_tokens = self.harness.queue.total_tokens();
            let t0 = std::time::Instant::now();
            let summary = self
                .llm
                .summarize(self.harness.queue.messages())
                .await
                .map_err(std::io::Error::other)?;
            self.harness.queue.compress(summary.clone(), ts);
            let post_tokens = self.harness.queue.total_tokens();
            self.harness.log_compact_boundary(
                summary,
                pre_tokens,
                post_tokens,
                t0.elapsed().as_millis() as u64,
                ts,
            )?;
            if let Some(p) = crate::panorama(&self.harness.agents) {
                self.harness
                    .append_queue(QueueMessage::new(Role::System, p, ts))?;
            }
        }
        // 5. tool 循环（请求 = 请求头 + Queue 全部消息 + Autonomy 末端）
        let tools = tool_set();
        let mut effects = vec![];
        for _ in 0..self.max_tool_iters {
            let mut request = Vec::with_capacity(self.harness.queue.messages().len() + 2);
            request.push(QueueMessage::new(Role::System, head.clone(), ts));
            request.extend_from_slice(self.harness.queue.messages());
            request.push(QueueMessage::new(Role::System, autonomy.clone(), ts));
            let out = self
                .llm
                .complete(&request, &tools)
                .await
                .map_err(std::io::Error::other)?;
            if out.tool_calls.is_empty() {
                // 沉默语义：空 content 不追加（docs/agent-loop.md）
                if let Some(content) = out.content.filter(|c| !c.is_empty()) {
                    self.harness
                        .append_queue(QueueMessage::new(Role::Assistant, content, ts))?;
                }
                break;
            }
            let mut assistant_msg = QueueMessage::assistant_tool_calls(out.tool_calls.clone(), ts);
            // thinking 模型：存思维链，回放时必须带回（docs/agent-loop.md）
            assistant_msg.reasoning_content = out.reasoning_content.clone();
            self.harness.append_queue(assistant_msg)?;
            for call in &out.tool_calls {
                let (result, mut eff) = self.execute_tool(call);
                effects.append(&mut eff);
                self.harness
                    .append_queue(QueueMessage::tool_result(&call.id, result.to_string(), ts))?;
            }
        }
        Ok(effects)
    }

    /// 真实 hook（docs/hook.md）：session_id 身份 + register-on-first-sight + 事件分层。
    /// mock hook（handle_hook）保留为 debug 手段，两条路径并存。
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_real_hook(
        &mut self,
        event: &str,
        session_id: &str,
        cwd: &str,
        kind: Option<&str>,
        prompt: Option<&str>,
        message: Option<&str>,
        last_assistant_message: Option<&str>,
        ts: i64,
        pending_notifications: usize,
    ) -> std::io::Result<Vec<Effect>> {
        let project = std::path::Path::new(cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let hash = crate::sid8(session_id);
        let name = crate::instance_name(project, &hash);
        // register-on-first-sight：未知 session_id 先落注册（first_seen = 后端初见时刻），
        // 已有条目沿用 first_seen / tab / kind（快照字段不被事件覆盖）
        let prev = self.harness.agents.iter().rev().find(|a| a.hash == hash);
        let first_seen = prev.map(|a| a.first_seen).unwrap_or(ts);
        let tab = prev.and_then(|a| a.tab);
        let kind = kind
            .map(String::from)
            .or_else(|| prev.and_then(|a| a.kind.clone()));
        // Hook 到达 → Timer 重排（docs/timer.md）
        self.timers.reset(&name, ts);
        let mut upsert = |status: AgentStatus| {
            self.harness.upsert_agent(AgentEntry {
                hash: hash.clone(),
                name: name.clone(),
                project: project.into(),
                kind: kind.clone(),
                status,
                tab,
                first_seen,
                last_seen: ts,
            })
        };
        match event {
            // 静默簿记（EventBuffer，pet 不醒）
            "session_start" => {
                upsert(AgentStatus::Idle)?;
                self.harness.event_buffer.push(format!("+ {name} 注册"));
                Ok(vec![])
            }
            "session_end" => {
                upsert(AgentStatus::Closed)?;
                self.harness.event_buffer.push(format!("− {name} 关闭"));
                Ok(vec![])
            }
            // Queue 触发
            "user_prompt" => {
                upsert(AgentStatus::Processing)?;
                let p = prompt.unwrap_or("").trim();
                self.harness.append_queue(QueueMessage::new(
                    Role::System,
                    format!("[观察] 用户在 {name} 输入：{p}"),
                    ts,
                ))?;
                self.run_trigger(ts, pending_notifications).await
            }
            "notification" => {
                let m = message.unwrap_or("").trim();
                self.harness.append_queue(QueueMessage::new(
                    Role::System,
                    format!("[{name}] 请求注意：{m}"),
                    ts,
                ))?;
                self.run_trigger(ts, pending_notifications).await
            }
            "stop" => {
                upsert(AgentStatus::Idle)?;
                // stop_hook_mode 默认 queue_only（B）：hint 注入，宠物按需 fetch（docs/hook.md §stop 三模式）
                let hint = last_assistant_message.unwrap_or("").trim();
                let text = if hint.is_empty() {
                    format!("{name} 完成，无汇报内容。评估是否通知。")
                } else {
                    format!("{name} 完成：{hint}。评估是否通知。")
                };
                self.harness
                    .append_queue(QueueMessage::new(Role::System, text, ts))?;
                self.run_trigger(ts, pending_notifications).await
            }
            _ => Ok(vec![]),
        }
    }

    /// mock hook（docs/agent-loop.md §Mock Hook 契约）
    pub async fn handle_hook(
        &mut self,
        event: &str,
        instance: &str,
        project: &str,
        content: &str,
        ts: i64,
        pending_notifications: usize,
    ) -> std::io::Result<Vec<Effect>> {
        // 读取链（docs/storage.md）：原文先存 terminal-content.jsonl，再 Filter 存 context.jsonl
        self.harness.append_terminal_content(TerminalContentRecord {
            instance: instance.into(),
            raw: content.to_string(),
            source: RecordSource::Hook,
            ts,
        })?;
        // Filter：Content → Context 链路（concepts §11），存归一后文本，字数按归一后计
        let filtered = self.filter.digest(content).render();
        // Hook 到达 → Timer 重排（近期有 Hook 的实例不该被补扫，docs/timer.md）
        self.timers.reset(instance, ts);
        match event {
            "session_start" => {
                self.harness.upsert_agent(AgentEntry {
                    hash: crate::agent_hash(instance, project, ts),
                    name: instance.into(),
                    project: project.into(),
                    kind: None,
                    status: AgentStatus::Processing,
                    tab: None,
                    first_seen: ts,
                    last_seen: ts,
                })?;
                self.harness.append_context(ContextRecord {
                    instance: instance.into(),
                    content: filtered,
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
                // 同名不同命：沿用该名字最近一条未 closed 的生命周期（hash/first_seen）
                let (hash, first_seen) = self
                    .harness
                    .agents
                    .iter()
                    .rev()
                    .find(|a| a.name == instance && a.status != AgentStatus::Closed)
                    .map(|a| (a.hash.clone(), a.first_seen))
                    .unwrap_or_else(|| (crate::agent_hash(instance, project, ts), ts));
                self.harness.upsert_agent(AgentEntry {
                    hash,
                    name: instance.into(),
                    project: project.into(),
                    kind: None,
                    status: AgentStatus::Idle,
                    tab: None,
                    first_seen,
                    last_seen: ts,
                })?;
                self.harness.append_context(ContextRecord {
                    instance: instance.into(),
                    content: filtered.clone(),
                    source: RecordSource::Hook,
                    ts,
                })?;
                let len = filtered.chars().count();
                self.harness.append_queue(QueueMessage::new(
                    Role::System,
                    format!("{instance} 完成，Context 已更新（{len} 字）。评估是否通知。"),
                    ts,
                ))?;
            }
            _ => {}
        }
        self.run_trigger(ts, pending_notifications).await
    }

    /// 提取到期的兜底扫描实例（docs/timer.md）
    pub fn due_timer_scans(&mut self, now: i64, batch: usize) -> Vec<String> {
        self.timers.due(now, batch)
    }

    /// Timer 兜底扫描处理（docs/timer.md §扫描处理流程）：
    /// Filter → 变化检测 → Substantive 才注入 Queue 评估；Minor/Unchanged 只存档不打扰
    pub async fn handle_timer_scan(
        &mut self,
        instance: &str,
        content: &str,
        ts: i64,
        pending_notifications: usize,
    ) -> std::io::Result<Vec<Effect>> {
        // 原文先存档（docs/storage.md），再 Filter + 变化检测
        self.harness.append_terminal_content(TerminalContentRecord {
            instance: instance.into(),
            raw: content.to_string(),
            source: RecordSource::Timer,
            ts,
        })?;
        let filtered = self.filter.digest(content).render();
        let prev = self
            .harness
            .context
            .latest(instance)
            .map(|r| r.content.clone())
            .unwrap_or_default();
        let change = self.filter.detect_change(&prev, &filtered);
        let len = filtered.chars().count();
        self.harness.append_context(ContextRecord {
            instance: instance.into(),
            content: filtered,
            source: RecordSource::Timer,
            ts,
        })?;
        if matches!(change, Change::Substantive(_)) {
            self.harness.append_queue(QueueMessage::new(
                Role::System,
                format!("{instance} 兜底扫描发现变化，Context 已更新（{len} 字）。评估是否通知。"),
                ts,
            ))?;
            return self.run_trigger(ts, pending_notifications).await;
        }
        Ok(vec![])
    }

    /// Timer 兜底扫描发现 tab 不复存在 → closed 终态（docs/storage.md：永久日志的消亡语义）
    pub fn mark_instance_closed(&mut self, instance: &str, ts: i64) -> std::io::Result<()> {
        if let Some(a) = self
            .harness
            .agents
            .iter()
            .rev()
            .find(|a| a.name == instance && a.status != AgentStatus::Closed)
            .cloned()
        {
            self.harness.upsert_agent(AgentEntry {
                status: AgentStatus::Closed,
                    tab: None,
                last_seen: ts,
                ..a
            })?;
        }
        Ok(())
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
                // 读通道优先（sidecar/MockTerminals），回退 Context 最新记录（docs/timer.md）
                // 读到原文：先存档再过滤（docs/storage.md 读取链）；失败仅日志不阻断
                let content = self
                    .terminal_reader
                    .as_ref()
                    .and_then(|r| r(inst))
                    .map(|raw| {
                        let _ = self.harness.append_terminal_content(TerminalContentRecord {
                            instance: inst.into(),
                            raw: raw.clone(),
                            source: RecordSource::FetchTerminal,
                            ts: crate::server::now_ms(),
                        });
                        let filtered = self.filter.digest(&raw).render();
                        let _ = self.harness.append_context(ContextRecord {
                            instance: inst.into(),
                            content: filtered.clone(),
                            source: RecordSource::FetchTerminal,
                            ts: crate::server::now_ms(),
                        });
                        filtered
                    })
                    .or_else(|| self.harness.context.latest(inst).map(|r| r.content.clone()))
                    .unwrap_or_else(|| "（无记录）".into());
                (json!({ "instance": inst, "content": content }), vec![])
            }
            "set_autonomy" => {
                let mut face = args.get("face").and_then(Value::as_str).map(String::from);
                let mut motion = args.get("motion").and_then(Value::as_str).map(String::from);
                // face 传 key 名：仅解析为映射表本体；motion 不连带——
                // 「仅传参的字段被覆盖」，缺省即不碰（docs/autonomy.md）
                if let Some(f) = &face {
                    if let Some(entry) = self.config.kaomoji.get(f.as_str()) {
                        face = Some(entry.face.clone());
                    }
                }
                (
                    json!({ "ok": true }),
                    vec![Effect::SetAutonomy {
                        face,
                        motion,
                        ttl_ms: args.get("ttlMs").and_then(Value::as_u64),
                    }],
                )
            }
            "edit_config" => {
                let path = args.get("path").and_then(Value::as_str).unwrap_or("");
                let Some(value) = args.get("value").cloned() else {
                    return (json!({ "ok": false, "error": "path/value 都必传" }), vec![]);
                };
                match self.apply_config_by_path(path, value) {
                    Ok(outcome) => {
                        let mut r = json!({ "ok": true, "path": path });
                        if !outcome.restart_required.is_empty() {
                            r["restartRequired"] = json!(outcome.restart_required);
                        }
                        (r, outcome.effects)
                    }
                    Err(e) => (json!({ "ok": false, "error": e }), vec![]),
                }
            }
            other => (
                json!({ "ok": false, "error": format!("unknown tool: {other}") }),
                vec![],
            ),
        }
    }
}

/// 配置修改结果（apply_config_by_path 返回）
pub struct ConfigOutcome {
    pub effects: Vec<Effect>,
    pub llm_changed: bool,
    pub restart_required: Vec<String>,
}

/// 冷字段（行为即真相：本进程不重建 TimerWheel，错峰调度状态会丢 → 如实上报需重启）
fn restart_required_for(path: &str) -> bool {
    matches!(
        path.split('.').next().unwrap_or(""),
        "timer_interval_ms" | "timer_stagger_ms"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{DebugAgent, LlmOutput};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("overseer-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// 沉默 mock：不注入任何反应的测试用
    fn make_overseer(tag: &str) -> OverseerBackend<DebugAgent> {
        make_overseer_with(tag, DebugAgent::silent())
    }

    fn make_overseer_with(tag: &str, agent: DebugAgent) -> OverseerBackend<DebugAgent> {
        let dir = tmp_dir(tag);
        let harness = Harness::load(&dir, &dir, 100_000, 0).unwrap();
        OverseerBackend::new(harness, Config::default(), agent)
    }

    /// 脚本决策源：每次 LLM 调用按序弹出一条；耗尽后沉默
    fn scripted(outputs: Vec<LlmOutput>) -> DebugAgent {
        let rest = std::sync::Mutex::new(std::collections::VecDeque::from(outputs));
        DebugAgent::new(move |_| rest.lock().unwrap().pop_front().unwrap_or_else(silence))
    }

    fn say(text: &str) -> LlmOutput {
        LlmOutput {
            content: Some(text.into()),
            tool_calls: vec![],
            reasoning_content: None,
        }
    }

    fn calls(specs: Vec<(&str, Value)>) -> LlmOutput {
        LlmOutput {
            content: None,
            tool_calls: specs
                .into_iter()
                .enumerate()
                .map(|(i, (name, args))| ToolCall {
                    id: format!("script-{i}"),
                    name: name.into(),
                    arguments: args.to_string(),
                })
                .collect(),
            reasoning_content: None,
        }
    }

    fn silence() -> LlmOutput {
        LlmOutput {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
        }
    }

    #[tokio::test]
    async fn stop_hook_scripted_notify_flow() {
        // mock 脚本：hook 触发后决定通知（set_autonomy + call_component），然后沉默
        let agent = scripted(vec![
            calls(vec![
                (
                    "set_autonomy",
                    json!({"face": "✧*｡٩(ˊᗜˋ*)و✧*｡", "motion": "bounce", "ttlMs": 5000}),
                ),
                (
                    "call_component",
                    json!({"spec": {"id": "notify-ft", "type": "text_card", "title": "ft 完成", "text": "干完了", "direction": "auto"}}),
                ),
            ]),
            silence(),
        ]);
        let mut ov = make_overseer_with("notify", agent);
        let long = "x".repeat(120);
        let effects = ov.handle_hook("stop", "ft", "proj", &long, 1, 0).await.unwrap();
        assert!(effects.iter().any(|e| matches!(e, Effect::RenderComponent(_))));
        assert!(effects.iter().any(|e| matches!(e, Effect::SetAutonomy { .. })));
        let roles: Vec<Role> = ov.harness.queue.messages().iter().map(|m| m.role).collect();
        // system(hook) + assistant(tool_calls) + tool + tool
        assert_eq!(
            roles,
            vec![Role::System, Role::Assistant, Role::Tool, Role::Tool]
        );
        // agent 注册为 idle
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Idle);
        let _ = std::fs::remove_dir_all(tmp_dir("notify"));
    }

    #[tokio::test]
    async fn stop_short_content_silence() {
        let mut ov = make_overseer("silence");
        let effects = ov.handle_hook("stop", "oss", "proj", "清理了 2 行注释", 1, 0).await.unwrap();
        assert!(effects.is_empty());
        let roles: Vec<Role> = ov.harness.queue.messages().iter().map(|m| m.role).collect();
        // system(hook)，沉默不追加 assistant
        assert_eq!(roles, vec![Role::System]);
        let _ = std::fs::remove_dir_all(tmp_dir("silence"));
    }

    #[tokio::test]
    async fn session_start_registers_and_triggers() {
        // mock 脚本：问候 (・ω・)ノ + 实例一览卡片（Example A 的人为剧本）
        let agent = scripted(vec![
            calls(vec![
                ("set_autonomy", json!({"face": "(・ω・)ノ", "ttlMs": 3000})),
                (
                    "call_component",
                    json!({"spec": {"id": "roster", "type": "text_card", "title": "实例一览", "text": "- new-feature [Processing]", "direction": "auto"}}),
                ),
            ]),
            silence(),
        ]);
        let mut ov = make_overseer_with("register", agent);
        let effects = ov
            .handle_hook("session_start", "new-feature", "proj", "启动画面", 1, 0)
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
        // Example A：问候 (・ω・)ノ + 实例一览卡片
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::SetAutonomy { face: Some(f), .. } if f == "(・ω・)ノ"
        )));
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::RenderComponent(spec)
                if spec.get("id").and_then(|v| v.as_str()) == Some("roster")
                && spec.get("text").and_then(|v| v.as_str()).unwrap_or("").contains("new-feature")
        )));
        let _ = std::fs::remove_dir_all(tmp_dir("register"));
    }

    #[tokio::test]
    async fn user_followup_triggers_fetch_loop() {
        // mock 脚本：hook 沉默 → 追问时 fetch_terminal → 汇总回复
        let agent = scripted(vec![
            silence(),
            calls(vec![("fetch_terminal", json!({"instance": "ft"}))]),
            say("[debug] 查到：全文"),
        ]);
        let mut ov = make_overseer_with("fetch", agent);
        let long = "y".repeat(100);
        ov.handle_hook("stop", "ft", "proj", &long, 1, 0).await.unwrap();
        ov.harness
            .append_queue(QueueMessage::new(Role::User, "那个 bug 具体怎么回事？", 2))
            .unwrap();
        ov.run_trigger(3, 0).await.unwrap();
        let msgs = ov.harness.queue.messages();
        // fetch_terminal 被执行，tool result 含 Context 全文
        assert!(msgs.iter().any(|m| m.role == Role::Tool
            && m.content.as_deref().unwrap_or("").contains(&"y".repeat(100))));
        // 最终 assistant 汇总（脚本原文）
        assert_eq!(msgs.last().unwrap().role, Role::Assistant);
        assert_eq!(msgs.last().unwrap().content.as_deref(), Some("[debug] 查到：全文"));
        let _ = std::fs::remove_dir_all(tmp_dir("fetch"));
    }

    #[tokio::test]
    async fn event_buffer_merged_on_trigger() {
        let mut ov = make_overseer("merge");
        ov.harness.event_buffer.push("用户关闭了 text_card「摘要」");
        ov.harness.event_buffer.push("用户勾选了 todobox 条目「跑测试」");
        ov.run_trigger(1, 0).await.unwrap();
        let sys: Vec<_> = ov
            .harness
            .queue
            .messages()
            .iter()
            .filter(|m| m.role == Role::System)
            .collect();
        // 合并的一条
        assert_eq!(sys.len(), 1);
        let merged = sys[0].content.as_deref().unwrap();
        assert!(merged.contains("用户关闭了 text_card「摘要」"));
        assert!(merged.contains("用户勾选了 todobox 条目「跑测试」"));
        assert!(ov.harness.event_buffer.is_empty());
        let _ = std::fs::remove_dir_all(tmp_dir("merge"));
    }

    #[tokio::test]
    async fn plain_user_message_replies() {
        let agent = scripted(vec![say("[debug] 收到：你好")]);
        let mut ov = make_overseer_with("reply", agent);
        ov.harness
            .append_queue(QueueMessage::new(Role::User, "你好", 1))
            .unwrap();
        ov.run_trigger(2, 0).await.unwrap();
        let last = ov.harness.queue.messages().last().unwrap();
        assert_eq!(last.role, Role::Assistant);
        assert_eq!(last.content.as_deref(), Some("[debug] 收到：你好"));
        let _ = std::fs::remove_dir_all(tmp_dir("reply"));
    }

    #[tokio::test]
    async fn hook_content_is_filtered_before_decision() {
        let mut ov = make_overseer("filter");
        // 原文很长但全是噪音 + 4 字内容 → 归一后 4 字 → 沉默
        let raw = format!(
            "● 完成\n✻ Crunched for 12s\n⏵⏵ bypass permissions on (shift+tab to cycle)\n{}",
            "─".repeat(100)
        );
        let effects = ov.handle_hook("stop", "ft", "proj", &raw, 1, 0).await.unwrap();
        assert!(effects.is_empty());
        assert_eq!(ov.harness.context.latest("ft").unwrap().content, "● 完成");
        let _ = std::fs::remove_dir_all(tmp_dir("filter"));
    }

    #[tokio::test]
    async fn timer_scan_substantive_notifies_and_records() {
        // mock 脚本：session_start 沉默 → 兜底触发后通知（call_component）→ 沉默
        let agent = scripted(vec![
            silence(),
            calls(vec![(
                "call_component",
                json!({"spec": {"id": "notify-cship", "type": "text_card", "title": "cship 有变化", "text": "去看看", "direction": "auto"}}),
            )]),
            silence(),
        ]);
        let mut ov = make_overseer_with("timer-sub", agent);
        ov.handle_hook("session_start", "cship", "proj", "旧内容", 1, 0)
            .await
            .unwrap();
        // 兜底扫描读到全新长内容 → Substantive → 存 Context(timer) + 注入 + 触发通知
        let new_content = "z".repeat(150);
        let effects = ov
            .handle_timer_scan("cship", &new_content, 2, 0)
            .await
            .unwrap();
        let rec = ov.harness.context.latest("cship").unwrap();
        assert_eq!(rec.source, RecordSource::Timer);
        assert_eq!(rec.content, new_content);
        assert!(ov
            .harness
            .queue
            .messages()
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("兜底扫描发现变化")));
        assert!(effects.iter().any(|e| matches!(e, Effect::RenderComponent(_))));
        let _ = std::fs::remove_dir_all(tmp_dir("timer-sub"));
    }

    #[tokio::test]
    async fn timer_scan_minor_stays_silent() {
        let mut ov = make_overseer("timer-min");
        ov.handle_hook("session_start", "cship", "proj", "内容不变", 1, 0)
            .await
            .unwrap();
        let msgs_before = ov.harness.queue.messages().len();
        // 内容相同 → Unchanged → 存档但不打扰
        let effects = ov
            .handle_timer_scan("cship", "内容不变", 2, 0)
            .await
            .unwrap();
        assert!(effects.is_empty());
        assert_eq!(ov.harness.queue.messages().len(), msgs_before);
        assert_eq!(ov.harness.context.latest("cship").unwrap().source, RecordSource::Timer);
        let _ = std::fs::remove_dir_all(tmp_dir("timer-min"));
    }

    #[tokio::test]
    async fn head_includes_agents_md() {
        let ov = make_overseer("head-md");
        let head = ov.assemble_system_prompt();
        // bootstrap 写入的默认身份提示词拼进了请求头（§12：Config 引用数据运行时拼装）
        assert!(head.contains("# AGENTS.md — ペット"));
        assert!(head.contains("## 颜文字映射"));
        // 请求头只装稳定提示词：实例状态走 diff 事件，不进请求头
        assert!(!head.contains("## 当前实例状态"));
        let _ = std::fs::remove_dir_all(tmp_dir("head-md"));
    }

    #[tokio::test]
    async fn hook_archives_raw_before_filter() {
        let mut ov = make_overseer("raw-archive");
        // 原文含噪音 → terminal-content.jsonl 存 filter 前全文，context.jsonl 存归一后
        let raw = format!("● 完成\n✻ Crunched for 12s\n{}", "─".repeat(100));
        ov.handle_hook("stop", "ft", "proj", &raw, 1, 0).await.unwrap();
        let archive = std::fs::read_to_string(
            ov.harness.storage_dir().join(crate::TERMINAL_CONTENT_FILE),
        )
        .unwrap();
        assert!(archive.contains("✻ Crunched for 12s")); // 原文噪音还在
        assert!(archive.contains("\"source\":\"hook\""));
        assert_eq!(ov.harness.context.latest("ft").unwrap().content, "● 完成");
        let _ = std::fs::remove_dir_all(tmp_dir("raw-archive"));
    }

    /// 捕获帧 mock：记录每次 LLM 调用看到的完整请求内容
    fn capturing(
        frames: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    ) -> DebugAgent {
        DebugAgent::new(move |msgs| {
            frames.lock().unwrap().push(
                msgs.iter()
                    .map(|m| m.content.clone().unwrap_or_default())
                    .collect(),
            );
            silence()
        })
    }

    #[tokio::test]
    async fn autonomy_logged_and_appended_to_request_end() {
        let frames = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let mut ov = make_overseer_with("autonomy", capturing(frames.clone()));
        ov.run_trigger(1, 0).await.unwrap();
        // 请求帧：首条 = 现拼请求头，末条 = Autonomy 状态（concepts §4）
        let f = &frames.lock().unwrap()[0];
        assert!(f[0].contains("## 颜文字映射"));
        assert_eq!(f.last().unwrap(), "[face: idle, motion: still]");
        // 末端状态不落 Queue（内存视图无它）
        assert!(ov
            .harness
            .queue
            .messages()
            .iter()
            .all(|m| m.content.as_deref() != Some("[face: idle, motion: still]")));
        // context.jsonl：autonomy 行每轮一条 + head 行
        let log = std::fs::read_to_string(ov.harness.storage_dir().join(crate::CONTEXT_FILE))
            .unwrap();
        assert!(log.contains("\"type\":\"autonomy\""));
        assert!(log.contains("[face: idle, motion: still]"));
        assert!(log.contains("\"type\":\"head\""));
        let _ = std::fs::remove_dir_all(tmp_dir("autonomy"));
    }

    #[tokio::test]
    async fn head_written_only_on_change() {
        let mut ov = make_overseer("head-diff");
        ov.run_trigger(1, 0).await.unwrap();
        ov.run_trigger(2, 0).await.unwrap();
        let storage = ov.harness.storage_dir().to_path_buf();
        let count = || {
            std::fs::read_to_string(storage.join(crate::CONTEXT_FILE))
                .unwrap()
                .matches("\"type\":\"head\"")
                .count()
        };
        assert_eq!(count(), 1); // 不变不写
        // AGENTS.md 热编辑 → 请求头变化 → 第二条 head 快照
        std::fs::write(ov.harness.config_dir().join(AGENTS_MD_FILE), "# 改过的ペット").unwrap();
        ov.run_trigger(3, 0).await.unwrap();
        assert_eq!(count(), 2);
        let _ = std::fs::remove_dir_all(tmp_dir("head-diff"));
    }

    #[tokio::test]
    async fn pending_notifications_drives_notify_key() {
        let frames = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let mut ov = make_overseer_with("notify-key", capturing(frames.clone()));
        ov.run_trigger(1, 2).await.unwrap();
        let f = &frames.lock().unwrap()[0];
        assert_eq!(f.last().unwrap(), "[face: notify, motion: bounce]");
        let _ = std::fs::remove_dir_all(tmp_dir("notify-key"));
    }

    #[tokio::test]
    async fn compression_logs_boundary_and_resyncs_panorama() {
        // 阈值 10 token：几条消息就触发 auto-compact（DebugAgent → summarize 回退确定性 stub）
        let dir = tmp_dir("compact");
        let harness = Harness::load(&dir, &dir, 10, 0).unwrap();
        let mut ov = OverseerBackend::new(harness, Config::default(), DebugAgent::silent());
        ov.harness
            .upsert_agent(AgentEntry {
                hash: "h1".into(),
                name: "ft".into(),
                project: "p".into(),
                    kind: None,
                status: AgentStatus::Idle,
                    tab: None,
                first_seen: 0,
                last_seen: 0,
            })
            .unwrap();
        for i in 0..5 {
            ov.harness
                .append_queue(QueueMessage::new(Role::User, format!("第 {i} 条消息内容内容"), i as i64))
                .unwrap();
        }
        ov.run_trigger(10, 0).await.unwrap();
        let msgs = ov.harness.queue.messages();
        // 内存视图：摘要为首条（shaking）
        assert!(msgs[0].content.as_deref().unwrap().starts_with("[历史摘要]"));
        // 归零重 diff：全景消息在摘要之后（压缩不丢实例认知）
        assert!(msgs
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("实例全景同步")));
        // compact_boundary 标记落盘（文件不删历史，可审计）
        let log =
            std::fs::read_to_string(ov.harness.storage_dir().join(crate::CONTEXT_FILE)).unwrap();
        assert!(log.contains("\"type\":\"compact_boundary\""));
        assert!(log.contains("\"pre_tokens\":"));
        assert!(log.contains("\"duration_ms\":"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tab_gone_marks_closed_and_exits_panorama() {
        let mut ov = make_overseer("closed");
        ov.handle_hook("session_start", "ft", "proj", "启动", 1, 0)
            .await
            .unwrap();
        assert!(crate::panorama(&ov.harness.agents).is_some());
        // Timer 发现 tab 不复存在 → closed 终态，全景不再包含
        ov.mark_instance_closed("ft", 2).unwrap();
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Closed);
        assert!(crate::panorama(&ov.harness.agents).is_none());
        // 同名再注册 = 新生命周期（同名不同命，hash 不同）
        ov.handle_hook("session_start", "ft", "proj", "又开了", 3, 0)
            .await
            .unwrap();
        assert_eq!(ov.harness.agents.len(), 2);
        assert_ne!(ov.harness.agents[0].hash, ov.harness.agents[1].hash);
        // stop 沿用最近一条未 closed 的生命周期
        ov.handle_hook("stop", "ft", "proj", "完成", 4, 0).await.unwrap();
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Closed);
        assert_eq!(ov.harness.agents[1].status, AgentStatus::Idle);
        let _ = std::fs::remove_dir_all(tmp_dir("closed"));
    }

    #[tokio::test]
    async fn hook_resets_timer_wheel() {
        let mut ov = make_overseer("timer-reset");
        ov.handle_hook("session_start", "a", "proj", "x", 1000, 0)
            .await
            .unwrap();
        // reset 后 due ≥ 1000 + interval（Config 默认 300s）
        assert!(ov.due_timer_scans(1000 + 100_000, 10).is_empty());
        assert_eq!(ov.due_timer_scans(1000 + 400_000, 10), vec!["a".to_string()]);
        let _ = std::fs::remove_dir_all(tmp_dir("timer-reset"));
    }

    #[tokio::test]
    async fn set_autonomy_face_key_resolves_to_body() {
        let mut ov = make_overseer("face-key");
        let call = ToolCall {
            id: "c1".into(),
            name: "set_autonomy".into(),
            // face 传 key 名：仅解析 face 本体；motion 缺省不连带（保持未覆盖）
            arguments: json!({ "face": "notify", "ttlMs": 3000 }).to_string(),
        };
        let (result, effects) = ov.execute_tool(&call);
        assert_eq!(result["ok"], json!(true));
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::SetAutonomy {
                face: Some(f),
                motion: None,
                ..
            } if f == "✧*｡٩(ˊᗜˋ*)و✧*｡"
        )));
        // 颜文字本体原样透传（非 key 不解析）
        let call2 = ToolCall {
            id: "c2".into(),
            name: "set_autonomy".into(),
            arguments: json!({ "face": "(・ω・)ノ" }).to_string(),
        };
        let (_, effects2) = ov.execute_tool(&call2);
        assert!(effects2.iter().any(|e| matches!(
            e,
            Effect::SetAutonomy { face: Some(f), motion: None, .. } if f == "(・ω・)ノ"
        )));
        let _ = std::fs::remove_dir_all(tmp_dir("face-key"));
    }

    #[tokio::test]
    async fn real_hook_first_sight_registers_silently() {
        let mut ov = make_overseer("rh1");
        let effects = ov
            .handle_real_hook(
                "session_start",
                "3f8a2c1e-9b7d-4e5f-a6c1-02d4e6f8a9b0",
                r"/tmp/p",
                Some("claude"),
                None,
                None,
                None,
                1000,
                0,
            )
            .await
            .unwrap();
        assert!(effects.is_empty()); // 静默:不触发 LLM
        let a = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "3f8a2c1e")
            .expect("已注册");
        assert_eq!(a.name, "npc-prof·3f8a2c1e");
        assert_eq!(a.kind.as_deref(), Some("claude"));
        assert_eq!(a.status, AgentStatus::Idle);
        assert_eq!(a.first_seen, 1000);
        assert!(ov.harness.queue.messages().is_empty()); // 无 queue 注入
        let _ = std::fs::remove_dir_all(tmp_dir("rh1"));
    }

    #[tokio::test]
    async fn real_hook_late_start_self_heals() {
        let mut ov = make_overseer("rh2");
        // backend 当时不在线,start 丢失:初见恰好是 stop(register-on-first-sight)
        let _ = ov
            .handle_real_hook(
                "stop",
                "aaaa0000-1111-2222",
                r"/tmp/p",
                None,
                None,
                None,
                Some("修完了"),
                2000,
                0,
            )
            .await
            .unwrap();
        let a = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "aaaa0000")
            .expect("自愈注册");
        assert_eq!(a.status, AgentStatus::Idle);
        assert_eq!(a.first_seen, 2000); // first_seen = 后端初见时刻
        assert!(ov
            .harness
            .queue
            .messages()
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("修完了")));
        let _ = std::fs::remove_dir_all(tmp_dir("rh2"));
    }

    #[tokio::test]
    async fn real_hook_resume_upserts_no_duplicate() {
        let mut ov = make_overseer("rh3");
        for ts in [1000, 5000] {
            let _ = ov
                .handle_real_hook(
                    "session_start",
                    "bbbb1111-2222-3333",
                    r"/tmp/p",
                    None,
                    None,
                    None,
                    None,
                    ts,
                    0,
                )
                .await
                .unwrap();
        }
        assert_eq!(ov.harness.agents.len(), 1); // 同 sid8 自然 upsert
        assert_eq!(ov.harness.agents[0].first_seen, 1000); // first_seen 保留
        assert_eq!(ov.harness.agents[0].last_seen, 5000);
        let _ = std::fs::remove_dir_all(tmp_dir("rh3"));
    }

    #[tokio::test]
    async fn real_hook_prompt_processing_and_session_end_closed() {
        let mut ov = make_overseer("rh4");
        let sid = "cccc2222-3333-4444";
        let _ = ov
            .handle_real_hook("session_start", sid, r"/tmp/p", None, None, None, None, 1000, 0)
            .await
            .unwrap();
        let _ = ov
            .handle_real_hook("user_prompt", sid, r"/tmp/p", None, Some("帮我修 bug"), None, None, 2000, 0)
            .await
            .unwrap();
        let a = ov.harness.agents.iter().find(|a| a.hash == "cccc2222").unwrap();
        assert_eq!(a.status, AgentStatus::Processing); // 派活驱动
        assert!(ov
            .harness
            .queue
            .messages()
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("[观察] 用户在")));
        let _ = ov
            .handle_real_hook("session_end", sid, r"/tmp/p", None, None, None, None, 3000, 0)
            .await
            .unwrap();
        let a = ov.harness.agents.iter().find(|a| a.hash == "cccc2222").unwrap();
        assert_eq!(a.status, AgentStatus::Closed); // 真信号终态
        assert_eq!(a.tab, None);
        let _ = std::fs::remove_dir_all(tmp_dir("rh4"));
    }

    #[tokio::test]
    async fn edit_config_updates_and_persists() {
        let mut ov = make_overseer("cfg");
        let call = ToolCall {
            id: "c1".into(),
            name: "edit_config".into(),
            arguments: json!({
                "path": "kaomoji.celebrate",
                "value": { "face": "(≧▽≦)", "motion": "bounce" }
            })
            .to_string(),
        };
        let (result, effects) = ov.execute_tool(&call);
        assert_eq!(result["ok"], json!(true));
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::ConfigChanged { llm_changed: false })));
        assert_eq!(ov.config.kaomoji["celebrate"].face, "(≧▽≦)");
        // config.json 已持久化
        let reloaded = Config::load_or_default(ov.harness.config_dir());
        assert_eq!(reloaded.kaomoji["celebrate"].motion, "bounce");
        let _ = std::fs::remove_dir_all(tmp_dir("cfg"));
    }

    #[test]
    fn apply_config_by_path_hot_apply_and_persist() {
        let mut ov = make_overseer("apply");
        let out = ov
            .apply_config_by_path("token_threshold", json!(5000))
            .unwrap();
        assert!(out.restart_required.is_empty());
        assert!(!out.llm_changed);
        assert_eq!(ov.harness.queue.token_threshold, 5000); // 热同步
        let reloaded = Config::load_or_default(ov.harness.config_dir());
        assert_eq!(reloaded.token_threshold, 5000); // persist
        let _ = std::fs::remove_dir_all(tmp_dir("apply"));
    }

    #[test]
    fn apply_config_by_path_validates_and_reports_restart() {
        let mut ov = make_overseer("apply2");
        // serde 验证失败
        assert!(ov.apply_config_by_path("token_threshold", json!("oops")).is_err());
        // 动态 enum 校验
        assert!(ov.apply_config_by_path("llm.active", json!("nonexist")).is_err());
        // 合法 active
        let out = ov.apply_config_by_path("llm.active", json!("deepseek")).unwrap();
        assert!(out.llm_changed);
        // 冷字段如实上报
        let out2 = ov.apply_config_by_path("timer_interval_ms", json!(60000)).unwrap();
        assert_eq!(out2.restart_required, vec!["timer_interval_ms".to_string()]);
        let _ = std::fs::remove_dir_all(tmp_dir("apply2"));
    }

    #[test]
    fn apply_config_by_path_readonly_rejected() {
        let mut ov = make_overseer("apply3");
        ov.config.read_only = true;
        assert!(ov.apply_config_by_path("token_threshold", json!(1)).is_err());
        let _ = std::fs::remove_dir_all(tmp_dir("apply3"));
    }
}
