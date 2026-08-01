//! OverseerBackend（concepts §1）：触发循环 + tool 执行 + hook 处理（docs/agent-loop.md）。

use crate::content::RecordSource;
use crate::filter::{Change, Filter};
use crate::lifecycle::Lifecycle;
use crate::llm::{tool_set, Llm};
use crate::context::{ContextMessage, Role, ToolCall};
use crate::timer::TimerWheel;
use crate::{
    default_agents_md, AgentEntry, AgentStatus, Config, ContentRecord, Harness,
    TerminalContentRecord, AGENTS_MD_FILE,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// 副作用：经 WS 广播给前端（docs/agent-loop.md §协议）
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    RenderComponent(Value),
    /// 显式关闭卡片（Component 持续管理协议：action="close"）
    CloseComponent(String),
    SetAutonomy {
        face: Option<String>,
        motion: Option<String>,
        ttl_ms: Option<u64>,
    },
    /// llm_changed=true 时 server 广播前重建 LlmBackend
    ConfigChanged { llm_changed: bool },
    /// 流式增量（docs/streaming.md）：LLM 回复片段——纯显示优化，不经 Queue/Context。
    /// 不走 effects Vec（实时性），经 effect_sink 旁路直推。
    AssistantDelta {
        content: Option<String>,
        reasoning_content: Option<String>,
    },
    /// 一轮触发结束（loading 收尾信号，完整回复已写 Context）
    AssistantDone,
}

/// claude 检测（实测 54/54 命中、0 误伤）：✳ 前缀（活动 glyph）或标题 == claude
fn is_claude_title(t: &str) -> bool {
    let t = t.trim_start();
    t.starts_with('✳') || t == "claude"
}

/// 去标题 glyph/空白（marker 解析与占位名共用）
fn strip_glyphs(t: &str) -> String {
    t.trim_start_matches(['✳', ' ']).trim().to_string()
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
    /// VD 切换器（docs/hook.md §VD 切换能力）：instance → 切到目标窗口所在桌面（不切回）
    pub vd_switcher: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
    /// sidecar 在读通道链中时，Timer 读到 None 才判定 tab 消亡（closed）；
    /// 纯 MockTerminals 下 None 只是「未注入」，不能当消亡证据（设计决定）
    pub sidecar_enabled: bool,
    /// 存活卡片注册表（Component 持续管理协议：create/update/close 判定依据
    /// + 生命周期事件元数据，docs/components.md）
    pub cards: std::collections::HashMap<String, crate::lifecycle::CardMeta>,
    /// 流式 delta 旁路（docs/streaming.md）：run_trigger 每收到 delta 即发——
    /// 显示优化事件（AssistantDelta/AssistantDone）不进 effects Vec，由 server 层接广播
    pub effect_sink: Option<Arc<dyn Fn(&Effect) + Send + Sync>>,
    max_tool_iters: usize,
}

impl<L: Llm> OverseerBackend<L> {
    pub fn new(harness: Harness, config: Config, llm: L) -> Self {
        let filter = crate::filter::by_name(&config.filter_strategy);
        let timers = TimerWheel::new(config.timer_interval_ms, config.timer_stagger_ms);
        let mut backend = Self {
            harness,
            config,
            llm,
            filter,
            timers,
            terminal_reader: None,
            vd_switcher: None,
            sidecar_enabled: false,
            cards: std::collections::HashMap::new(),
            effect_sink: None,
            max_tool_iters: 8, // 防 tool 循环死转
        };
        // 启动调度（concepts §1a 兜底覆盖）：TimerWheel 不 replay，
        // 对投影中全部存活实例批量 reset——无 hook 实例（僵尸）也进兜底扫描集，
        // 否则它们永不入调度、永不判 closed（#10 reopen：调度盲区）
        let now = crate::server::now_ms();
        let alive: Vec<String> = backend
            .harness
            .agents
            .iter()
            .filter(|a| a.status != AgentStatus::Closed)
            .map(|a| a.name.clone())
            .collect();
        for name in alive {
            backend.timers.reset(&name, now);
        }
        backend
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
        // 统一 validation（docs/config.md：任一 validator 失败原子拒绝整次更新）
        let pool_errors = crate::config::validate_kaomoji_pools(&new.kaomoji);
        if !pool_errors.is_empty() {
            return Err(format!("验证失败: {}", pool_errors.join("；")));
        }
        // 动态 enum 校验（OPTIONS 注册表，验证集中一份）
        if let (Some(opts), Value::String(s)) =
            (crate::config::reflect::valid_options(&new, path), &value)
        {
            if !opts.contains(s) {
                return Err(format!("{path}: '{s}' 不在合法选项 {opts:?} 中"));
            }
        }
        let old = std::mem::replace(&mut self.config, new);
        // 热应用：filter 重建（其余字段每轮现读/经 effective_* 出口现取，天然热）
        if self.config.filter_strategy != old.filter_strategy {
            self.filter = crate::filter::by_name(&self.config.filter_strategy);
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
        // 请求头只带系统池（concepts §10b：用户表情池按需经 edit_config 查询，不自动注入）
        let mut keys: Vec<_> = self.config.kaomoji.system.keys().collect();
        keys.sort();
        for k in keys {
            let v = &self.config.kaomoji.system[k];
            s.push_str(&format!("- {k}: {} ({})\n", v.face, v.motion));
        }
        s
    }

    /// AGENTS.md 每轮现读（热生效：改完下一个触发就用）；读不到回退内置默认
    fn read_agents_md(&self) -> String {
        std::fs::read_to_string(self.harness.config_dir().join(AGENTS_MD_FILE))
            .unwrap_or_else(|_| default_agents_md())
    }

    /// 存活实例数（投影口径：status ≠ Closed）——生命周期簿记的 post-count（#16 ①）
    fn alive_count(&self) -> usize {
        self.harness
            .agents
            .iter()
            .filter(|a| a.status != AgentStatus::Closed)
            .count()
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
            .kaomoji_resolve(key)
            .map(|k| k.motion.as_str())
            .unwrap_or("still");
        format!("[face: {key}, motion: {motion}]")
    }

    /// 入队一条输入（concepts §10c：hook 内容 = system，user 消息 = user）。
    /// 生产者只入队不触发——放行由 drain_queue / server 消费者任务驱动。
    pub fn enqueue(&mut self, role: Role, content: String, ts: i64) -> std::io::Result<()> {
        self.harness
            .enqueue_input(crate::queue::QueueInput { role, content, ts })
    }

    /// 放行一条输入：Context 写输入 → run_trigger（一轮完整处理，concepts §10c）
    /// Event Buffer 附带（concepts §10e）：放行时 merge 清空——system 输入与之合并为
    /// 一条消息；user 输入则 buffer 以独立 system 消息先行附带（与 user role 严格分离）。
    pub async fn release_one(
        &mut self,
        input: crate::queue::QueueInput,
        pending_notifications: usize,
    ) -> std::io::Result<Vec<Effect>> {
        let ts = input.ts;
        let merged = self.harness.event_buffer.merge_and_clear();
        match (input.role, merged) {
            (Role::System, Some(buf)) => {
                // 附带合并为一条 system 消息
                self.harness.append_context(ContextMessage::new(
                    Role::System,
                    format!("{}\n\n{}", input.content, buf),
                    ts,
                ))?;
            }
            (role, maybe_buf) => {
                if let Some(buf) = maybe_buf {
                    self.harness
                        .append_context(ContextMessage::new(Role::System, buf, ts))?;
                }
                self.harness
                    .append_context(ContextMessage::new(role, input.content, ts))?;
            }
        }
        self.run_trigger(ts, pending_notifications).await
    }

    /// 放行循环：有输入就一轮一条处理完（server 消费者任务与测试共用）
    pub async fn drain_queue(&mut self, pending_notifications: usize) -> std::io::Result<Vec<Effect>> {
        let mut effects = vec![];
        while let Some(input) = self.harness.queue.release() {
            effects.append(&mut self.release_one(input, pending_notifications).await?);
        }
        Ok(effects)
    }

    /// 一轮触发（docs/agent-loop.md §一轮触发）
    /// 调用前输入已写 Context、Event Buffer 已在放行点附带（release_one）。
    /// pending_notifications：未决通知数（server 层计数传入，推导 notify key 用）
    pub async fn run_trigger(
        &mut self,
        ts: i64,
        pending_notifications: usize,
    ) -> std::io::Result<Vec<Effect>> {
        // 1. 现拼 system prompt 请求头（不落 Context）；变化才写 head 快照（docs/storage.md）
        let head = self.assemble_system_prompt();
        if self.harness.last_head.as_deref() != Some(head.as_str()) {
            self.harness.log_head(head.clone(), ts)?;
        }
        // 2. Autonomy 状态：每轮一条写 context.jsonl，最新一条挂请求末端（concepts §4）
        let autonomy = self.state_key(pending_notifications);
        self.harness.log_autonomy(autonomy.clone(), ts)?;
        // 3. Compression（auto-compact，concepts §10d / #16 真值触发）：
        //    判定式 = 最近 usage 真值 + 其后新增消息 est 增量 vs window − reserve；
        //    无真值（首轮/重启）→ 全量 est；无窗口事实（None）→ 不压缩
        let trigger_tokens = match self.harness.last_usage {
            Some(u) => {
                u.prompt_tokens as usize
                    + self
                        .harness
                        .context
                        .est_tokens_since(self.harness.last_usage_msg_len)
            }
            None => self.harness.context.total_tokens(),
        };
        let compress = self
            .config
            .effective_compression_limit()
            .is_some_and(|limit| trigger_tokens > limit);
        if compress {
            let pre_tokens = trigger_tokens; // 同尺：触发瞬间的真值锚点+增量（#16 ④）
            let t0 = std::time::Instant::now();
            // summarize 返回（摘要, usage 真值）；摘要调用也留真值（#16）
            let (summary, summary_usage) = self
                .llm
                .summarize(self.harness.context.messages())
                .await
                .map_err(std::io::Error::other)?;
            if let Some(u) = summary_usage {
                self.harness.log_usage(u, ts)?;
            }
            self.harness.context.compress(summary.clone(), ts);
            let post_tokens = self.harness.context.total_tokens(); // 同尺：压缩后 est（真值下轮刷新）
            self.harness.log_compact_boundary(
                summary,
                pre_tokens,
                post_tokens,
                t0.elapsed().as_millis() as u64,
                ts,
            )?;
            if let Some(p) = crate::panorama(&self.harness.agents) {
                self.harness
                    .append_context(ContextMessage::new(Role::System, p, ts))?;
            }
        }
        // 4. tool 循环（请求 = 请求头 + Context 全部消息 + Autonomy 末端）
        //    流式：complete_streaming 边收边经 effect_sink 发 AssistantDelta（docs/streaming.md）
        let tools = tool_set();
        let mut effects = vec![];
        for _ in 0..self.max_tool_iters {
            let mut request = Vec::with_capacity(self.harness.context.messages().len() + 2);
            request.push(ContextMessage::new(Role::System, head.clone(), ts));
            request.extend_from_slice(self.harness.context.messages());
            request.push(ContextMessage::new(Role::System, autonomy.clone(), ts));
            let sink = self.effect_sink.clone();
            let on_delta = move |d: &crate::llm::Delta| {
                if let Some(sink) = &sink {
                    sink(&Effect::AssistantDelta {
                        content: d.content.clone(),
                        reasoning_content: d.reasoning_content.clone(),
                    });
                }
            };
            let out = self
                .llm
                .complete_streaming(&request, &tools, &on_delta)
                .await
                .map_err(std::io::Error::other)?;
            // usage 真值留痕（#16：每轮一条，覆盖刷新 last_usage）
            if let Some(u) = out.usage {
                self.harness.log_usage(u, ts)?;
            }
            if out.tool_calls.is_empty() {
                // 沉默语义：空 content 不追加（docs/agent-loop.md）
                if let Some(content) = out.content.filter(|c| !c.is_empty()) {
                    let mut msg = ContextMessage::new(Role::Assistant, content, ts);
                    // thinking 全保真留痕（记录≠回放：build_body 仅 tool_calls
                    // 消息带 reasoning_content 进请求，纯文本回复不花 token）
                    msg.reasoning_content = out.reasoning_content.clone();
                    self.harness.append_context(msg)?;
                }
                break;
            }
            let mut assistant_msg = ContextMessage::assistant_tool_calls(out.tool_calls.clone(), ts);
            // thinking 模型：存思维链，回放时必须带回（docs/agent-loop.md）
            assistant_msg.reasoning_content = out.reasoning_content.clone();
            self.harness.append_context(assistant_msg)?;
            for call in &out.tool_calls {
                let (result, mut eff) = self.execute_tool(call);
                effects.append(&mut eff);
                self.harness
                    .append_context(ContextMessage::tool_result(&call.id, result.to_string(), ts))?;
            }
        }
        // 一轮完毕：loading 收尾（docs/streaming.md，完整回复已写 Context）
        if let Some(sink) = &self.effect_sink {
            sink(&Effect::AssistantDone);
        }
        Ok(effects)
    }

    /// 启动扫描（docs/hook.md §启动扫描）：全 VD 枚举 → claude 检测 →
    /// marker 解注册 / 无 marker 占位入册（uia:<标题>）→ N/M/K 三方对账进 EventBuffer。
    /// call = sidecar 请求转发（参数化便于测试注入）
    pub async fn startup_sweep(
        &mut self,
        call: &(dyn Fn(&Value) -> Option<Value> + Send + Sync),
        ts: i64,
    ) -> std::io::Result<()> {
        let Some(resp) = call(&json!({ "cmd": "list_windows" })) else {
            return Ok(());
        };
        let (mut located, mut marked, mut placeholder, mut cloaked_n) = (0usize, 0usize, 0usize, 0usize);
        let mut seen_titles: Vec<String> = Vec::new();
        for w in resp["windows"].as_array().cloned().unwrap_or_default() {
            let title = w["title"].as_str().unwrap_or("").to_string();
            let cloaked = w["cloaked"].as_bool().unwrap_or(false);
            if cloaked {
                cloaked_n += 1;
            }
            if !is_claude_title(&title) {
                continue;
            }
            if cloaked {
                // cloaked 窗口只有窗口级标题（= 活动 tab 标题），登记无 tab 定位
                seen_titles.push(title.clone());
                self.sweep_register(&title, None, ts, &mut marked, &mut placeholder)?;
                located += 1;
                continue;
            }
            let hwnd = w["hwnd"].as_i64().unwrap_or(0);
            let Some(tabs) = call(&json!({ "cmd": "list_tabs", "hwnd": hwnd })) else {
                continue;
            };
            for t in tabs["tabs"].as_array().cloned().unwrap_or_default() {
                let name = t["name"].as_str().unwrap_or("").to_string();
                if !is_claude_title(&name) {
                    continue;
                }
                seen_titles.push(name.clone());
                let tab_ref = Some(crate::TabRef {
                    hwnd,
                    index: t["index"].as_i64().unwrap_or(0),
                });
                self.sweep_register(&name, tab_ref, ts, &mut marked, &mut placeholder)?;
                located += 1;
            }
        }
        // 占位尸体清理：uia: 占位条目的标题已不在可见集 → closed（append 日志）
        let ghosts: Vec<AgentEntry> = self
            .harness
            .agents
            .iter()
            .filter(|a| {
                a.hash.starts_with("uia:")
                    && a.status != AgentStatus::Closed
                    && !seen_titles.iter().any(|t| strip_glyphs(t) == a.name)
            })
            .cloned()
            .collect();
        for g in ghosts {
            self.harness.upsert_agent(AgentEntry {
                status: AgentStatus::Closed,
                last_seen: ts,
                ..g
            })?;
        }
        // N/M/K 三方对账（N 为启发式参考值，K 是硬信号）
        let n = call(&json!({ "cmd": "count_processes", "name": "claude" }))
            .and_then(|r| r["count"].as_i64())
            .unwrap_or(0);
        let mut line = format!(
            "启动扫描: {located} tab 已定位（{marked} marker / {placeholder} 占位），claude.exe 进程 {n}，cloaked 窗口 {cloaked_n}"
        );
        if cloaked_n > 0 {
            line.push_str("（有窗口对其他桌面不可读，可开 WT「全桌面显示」）");
        }
        self.harness.event_buffer.push(line);
        Ok(())
    }

    /// claude 检测（实测 54/54 命中、0 误伤）：✳ 前缀（活动 glyph）或标题 == claude
    fn sweep_register(
        &mut self,
        title: &str,
        tab: Option<crate::TabRef>,
        ts: i64,
        marked: &mut usize,
        placeholder: &mut usize,
    ) -> std::io::Result<()> {
        let clean = strip_glyphs(title);
        // marker 解析：<project>·<sid8>（sid8 = 末尾 8 位）
        if let Some((project, sid8)) = clean.rsplit_once('·') {
            if sid8.chars().count() == 8 && !project.is_empty() {
                *marked += 1;
                let hash = sid8.to_string();
                let prev = self.harness.agents.iter().rev().find(|a| a.hash == hash);
                return self.harness.upsert_agent(AgentEntry {
                    hash: hash.clone(),
                    name: format!("{project}·{sid8}"),
                    project: project.into(),
                    kind: Some("claude".into()),
                    status: AgentStatus::Idle,
                    tab: tab.or_else(|| prev.and_then(|a| a.tab)),
                    first_seen: prev.map(|a| a.first_seen).unwrap_or(ts),
                    last_seen: ts,
                });
            }
        }
        *placeholder += 1;
        let hash = format!("uia:{clean}");
        let prev = self.harness.agents.iter().rev().find(|a| a.hash == hash);
        self.harness.upsert_agent(AgentEntry {
            hash,
            name: clean.clone(),
            project: "unknown".into(),
            kind: Some("claude".into()),
            status: AgentStatus::Idle,
            tab,
            first_seen: prev.map(|a| a.first_seen).unwrap_or(ts),
            last_seen: ts,
        })
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
    ) -> std::io::Result<()> {
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
            // 静默簿记（EventBuffer，pet 不醒）；post-count 标注（#16：LLM 免对账）
            "session_start" => {
                upsert(AgentStatus::Idle)?;
                let alive = self.alive_count();
                self.harness
                    .event_buffer
                    .push(format!("+ {name} 注册 → 存活 {alive}"));
            }
            "session_end" => {
                upsert(AgentStatus::Closed)?;
                let alive = self.alive_count();
                self.harness
                    .event_buffer
                    .push(format!("− {name} 关闭 → 存活 {alive}"));
            }
            // Queue 注入（放行后触发）
            "user_prompt" => {
                upsert(AgentStatus::Processing)?;
                let p = prompt.unwrap_or("").trim();
                self.enqueue(Role::System, format!("[观察] 用户在 {name} 输入：{p}"), ts)?;
            }
            "notification" => {
                let m = message.unwrap_or("").trim();
                self.enqueue(Role::System, format!("[{name}] 请求注意：{m}"), ts)?;
            }
            "stop" => {
                upsert(AgentStatus::Idle)?;
                let hint = last_assistant_message.unwrap_or("").trim();
                // stop_hook_mode 三模式（docs/hook.md，Config 热生效）
                let text = match self.config.stop_hook_mode.as_str() {
                    // A：stop 到达即读通道全量（tab 切换限流见 timer，此处只读）
                    "auto_read" => {
                        let content = self
                            .terminal_reader
                            .as_ref()
                            .and_then(|r| r(&name))
                            .map(|raw| {
                                let _ = self.harness.append_terminal_content(
                                    TerminalContentRecord {
                                        instance: name.clone(),
                                        raw: raw.clone(),
                                        source: RecordSource::Hook,
                                        ts,
                                    },
                                );
                                let filtered = self.filter.digest(&raw).render();
                                let _ = self.harness.append_content(ContentRecord {
                                    instance: name.clone(),
                                    content: filtered.clone(),
                                    source: RecordSource::Hook,
                                    ts,
                                });
                                filtered
                            });
                        match content {
                            Some(filtered) => {
                                let len = filtered.chars().count();
                                format!(
                                    "{name} 完成，Context 已更新（{len} 字）。评估是否通知。"
                                )
                            }
                            None => format!("{name} 完成：{hint}。评估是否通知。"),
                        }
                    }
                    // C：汇报原文直达（零 UIA）
                    "message" => {
                        if hint.is_empty() {
                            format!("{name} 完成，无汇报内容。评估是否通知。")
                        } else {
                            format!("[汇报] {name} 完成：{hint}")
                        }
                    }
                    // B（默认）：hint 注入，宠物按需 fetch
                    _ => {
                        if hint.is_empty() {
                            format!("{name} 完成，无汇报内容。评估是否通知。")
                        } else {
                            format!("{name} 完成：{hint}。评估是否通知。")
                        }
                    }
                };
                self.enqueue(Role::System, text, ts)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// mock hook（docs/agent-loop.md §Mock Hook 契约）
    pub async fn handle_hook(
        &mut self,
        event: &str,
        instance: &str,
        project: &str,
        content: &str,
        ts: i64,
    ) -> std::io::Result<()> {
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
            // 静默簿记（EventBuffer，pet 不醒；docs/agent-loop.md mock 契约对齐真实分层）
            "session_start" => {
                self.harness.upsert_agent(AgentEntry {
                    hash: crate::agent_hash(instance, project, ts),
                    name: instance.into(),
                    project: project.into(),
                    kind: None,
                    status: AgentStatus::Idle,
                    tab: None,
                    first_seen: ts,
                    last_seen: ts,
                })?;
                self.harness.append_content(ContentRecord {
                    instance: instance.into(),
                    content: filtered,
                    source: RecordSource::Hook,
                    ts,
                })?;
                let alive = self.alive_count();
                self.harness
                    .event_buffer
                    .push(format!("+ {instance} 注册 → 存活 {alive}"));
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
                self.harness.append_content(ContentRecord {
                    instance: instance.into(),
                    content: filtered.clone(),
                    source: RecordSource::Hook,
                    ts,
                })?;
                let len = filtered.chars().count();
                self.enqueue(
                    Role::System,
                    format!("{instance} 完成，Context 已更新（{len} 字）。评估是否通知。"),
                    ts,
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    /// 提取到期的兜底扫描实例（docs/timer.md）
    /// 提取到期的兜底扫描实例（docs/timer.md）；
    /// timer_interval_ms ≤ 0 = 禁用（真实 hook 接入初期只留 hook 驱动，设计决定）
    pub fn due_timer_scans(&mut self, now: i64, batch: usize) -> Vec<String> {
        if self.config.timer_interval_ms <= 0 {
            return vec![];
        }
        self.timers.due(now, batch)
    }

    /// Timer 兜底扫描处理（docs/timer.md §扫描处理流程）：
    /// Filter → 变化检测 → Substantive 才注入 Queue 评估；Minor/Unchanged 只存档不打扰
    pub async fn handle_timer_scan(
        &mut self,
        instance: &str,
        content: &str,
        ts: i64,
    ) -> std::io::Result<()> {
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
            .content
            .latest(instance)
            .map(|r| r.content.clone())
            .unwrap_or_default();
        let change = self.filter.detect_change(&prev, &filtered);
        let len = filtered.chars().count();
        self.harness.append_content(ContentRecord {
            instance: instance.into(),
            content: filtered,
            source: RecordSource::Timer,
            ts,
        })?;
        if matches!(change, Change::Substantive(_)) {
            self.enqueue(
                Role::System,
                format!("{instance} 兜底扫描发现变化，Context 已更新（{len} 字）。评估是否通知。"),
                ts,
            )?;
        }
        Ok(())
    }

    /// Timer 兜底扫描发现 tab 不复存在 → closed 终态（docs/storage.md：永久日志的消亡语义）
    /// Timer 判死（读通道返回 None）：该名字全部未 closed 生命周期各 append 一条
    /// closed 快照——读通道按 name 读，同名实例在读取侧不可区分，判死须同判
    ///（同名不同命：每 hash 独立快照，append-only 语义不变）。
    /// 判死 diff 事件化（#16 case 跑红实锤）：EventBuffer 簿记，下次放行附带入
    /// Context——否则 LLM 的全景认知停在旧快照（判死后仍答错实例数）。
    pub fn mark_instance_closed(&mut self, instance: &str, ts: i64) -> std::io::Result<()> {
        let targets: Vec<AgentEntry> = self
            .harness
            .agents
            .iter()
            .filter(|a| a.name == instance && a.status != AgentStatus::Closed)
            .cloned()
            .collect();
        if targets.is_empty() {
            return Ok(());
        }
        for a in targets {
            let name = a.name.clone();
            self.harness.upsert_agent(AgentEntry {
                status: AgentStatus::Closed,
                tab: None,
                last_seen: ts,
                ..a
            })?;
            // 判死 diff 事件化 + post-count（#16 ①：每条 hash 一条，post-count 逐条现算，
            // 同名连坐自然形成递减序列；LLM 直接读数免对账）
            let alive = self.alive_count();
            self.harness
                .event_buffer
                .push(format!("− {name} 关闭（Timer 判死）→ 存活 {alive}"));
        }
        Ok(())
    }

    /// 执行 tool call（run_trigger tool 循环与 case-runner tool_call step 共用）
    pub fn execute_tool(&mut self, call: &ToolCall) -> (Value, Vec<Effect>) {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
        match call.name.as_str() {
            "call_component" => {
                let spec = args.get("spec").cloned().unwrap_or(Value::Null);
                let id = spec
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // Tauri label 规则：只允许 [A-Za-z0-9_\-/.]+
                if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/') {
                    return (
                        json!({ "ok": false, "error": format!("spec.id '{id}' 不合法：窗口名只允许 A-Z a-z 0-9 _ - . /，不含空格、中文或特殊字符") }),
                        vec![],
                    );
                }
                // 显式关闭（持续管理协议，docs/components.md）：action="close" 只需合法 id，
                // 不要求 type 必填字段（关闭卡片不需要内容）
                // #23 两级兼容：LLM 有时把 action 放在 args 顶层（与 spec 并列），
                // spec 内查不到时回退 args 顶层，否则 close 会被当成空 update 渲染空卡
                let action = spec
                    .get("action")
                    .or_else(|| args.get("action"))
                    .and_then(Value::as_str);
                if action == Some("close") {
                    let ts = crate::server::now_ms();
                    if let Some(meta) = self.cards.remove(&id) {
                        // closed_by_agent 生命周期事件（一行，进 EventBuffer 静默簿记）
                        let alive = self.cards.len();
                        let line = crate::lifecycle::DefaultLifecycle.closed_line(&meta, alive, ts);
                        self.harness.event_buffer.push(line);
                    }
                    return (
                        json!({ "ok": true, "closed": id }),
                        vec![Effect::CloseComponent(id)],
                    );
                }
                // 校验 type 合法性 + 按 type 校验必填字段
                let VALID_TYPES: &[&str] = &["text_card", "quick_jump", "git_display", "data_chart", "todobox"];
                if let Some(typ) = spec.get("type").and_then(Value::as_str) {
                    if !VALID_TYPES.contains(&typ) {
                        return (
                            json!({ "ok": false, "error": format!("未知 Component type：'{typ}'，合法值：{}", VALID_TYPES.join("/")) }),
                            vec![],
                        );
                    }
                    let required: &[(&str, &str)] = match typ {
                        "text_card" => &[("title", "text_card 缺 title"), ("text", "text_card 缺 text")],
                        "quick_jump" => &[("label", "quick_jump 缺 label"), ("target", "quick_jump 缺 target")],
                        "git_display" => &[("title", "git_display 缺 title"), ("entries", "git_display 缺 entries")],
                        "data_chart" => &[("title", "data_chart 缺 title"), ("chart", "data_chart 缺 chart")],
                        "todobox" => &[("title", "todobox 缺 title"), ("items", "todobox 缺 items")],
                        _ => &[],
                    };
                    let missing: Vec<&str> = required.iter()
                        .filter(|(f, _)| spec.get(f).map_or(true, |v| match v {
                            Value::String(s) => s.is_empty(),
                            Value::Array(a) => a.is_empty(),
                            Value::Object(o) => o.is_empty(),
                            _ => true,
                        }))
                        .map(|(_, msg)| *msg)
                        .collect();
                    if !missing.is_empty() {
                        return (
                            json!({ "ok": false, "error": format!("type={typ} 缺少必填字段：{}。字段在 spec 顶层，不要包在 props 里", missing.join("、")) }),
                            vec![],
                        );
                    }
                    // todobox items 结构校验（toolset.md）：[{text, done}]
                    if typ == "todobox" {
                        let bad = spec["items"].as_array().map(|arr| arr.iter().any(|it| {
                            it["text"].as_str().map_or(true, str::is_empty) || it["done"].as_bool().is_none()
                        })).unwrap_or(true);
                        if bad {
                            return (
                                json!({ "ok": false, "error": "todobox items 结构不合法：需 [{text: string, done: boolean}]" }),
                                vec![],
                            );
                        }
                    }
                }
                // 创建 / 原地更新（同 id 不再 toggle 关闭）
                let typ = spec.get("type").and_then(Value::as_str).unwrap_or("").to_string();
                let title = spec.get("title").and_then(Value::as_str).unwrap_or("").to_string();
                if !self.cards.contains_key(&id) {
                    // created 生命周期事件（进 EventBuffer 静默簿记；agent 更新不产事件）
                    let ts = crate::server::now_ms();
                    let meta = crate::lifecycle::CardMeta {
                        id: id.clone(),
                        typ,
                        title,
                        created: ts,
                    };
                    let line = crate::lifecycle::DefaultLifecycle.created_line(&meta, self.cards.len() + 1);
                    self.harness.event_buffer.push(line);
                    self.cards.insert(id.clone(), meta);
                    (json!({ "ok": true, "rendered": id }), vec![Effect::RenderComponent(spec)])
                } else {
                    (json!({ "ok": true, "updated": id }), vec![Effect::RenderComponent(spec)])
                }
            }
            "fetch_terminal" => {
                let inst = args.get("instance").and_then(Value::as_str).unwrap_or("");
                if inst.is_empty() {
                    return (json!({ "ok": false, "error": "instance 必填" }), vec![]);
                }
                // vd_switch 必填（docs/hook.md §VD 切换能力）：打断性决策每次显式面对
                let Some(vd_switch) = args.get("vd_switch").and_then(Value::as_bool) else {
                    return (
                        json!({ "ok": false, "error": "vd_switch 必填（false=不切桌面；true=目标在其他虚拟桌面时切过去读，不切回）" }),
                        vec![],
                    );
                };
                // 读通道优先（sidecar/MockTerminals）：读到原文先存档再过滤（docs/storage.md 读取链）
                let read_fresh = |ov: &mut Self| {
                    ov.terminal_reader
                        .as_ref()
                        .and_then(|r| r(inst))
                        .map(|raw| {
                            let _ = ov.harness.append_terminal_content(TerminalContentRecord {
                                instance: inst.into(),
                                raw: raw.clone(),
                                source: RecordSource::FetchTerminal,
                                ts: crate::server::now_ms(),
                            });
                            let filtered = ov.filter.digest(&raw).render();
                            let _ = ov.harness.append_content(ContentRecord {
                                instance: inst.into(),
                                content: filtered.clone(),
                                source: RecordSource::FetchTerminal,
                                ts: crate::server::now_ms(),
                            });
                            filtered
                        })
                };
                if let Some(content) = read_fresh(self) {
                    return (json!({ "instance": inst, "content": content }), vec![]);
                }
                // 新鲜读失败 → Context 最新记录回退（有历史给历史）
                if let Some(rec) = self.harness.content.latest(inst) {
                    return (json!({ "instance": inst, "content": rec.content.clone() }), vec![]);
                }
                // 什么都没有：vd_switch=false → 失败教学；true → 切桌面重试
                if !vd_switch {
                    return (
                        json!({ "ok": false, "error": format!("读不到 {inst}：可能不存在，也可能在另一个虚拟桌面；确认存在的话用 vd_switch=true 重试") }),
                        vec![],
                    );
                }
                let switched = self.vd_switcher.as_ref().map(|f| f(inst)).unwrap_or(false);
                if !switched {
                    return (
                        json!({ "ok": false, "error": format!("切换失败：全 VD 窗口标题无 {inst} 匹配（可能不存在，或它是 cloaked 窗口的背景 tab）") }),
                        vec![],
                    );
                }
                let content = read_fresh(self)
                    .unwrap_or_else(|| "（已切换到目标桌面，但仍读不到内容）".into());
                (json!({ "instance": inst, "content": content }), vec![])
            }
            "set_autonomy" => {
                let mut face = args.get("key").and_then(Value::as_str).map(String::from);
                let motion = args.get("motion").and_then(Value::as_str).map(String::from);
                // key 传状态 key 名：解析为映射表本体；motion 不连带——
                // 「仅传参的字段被覆盖」，缺省即不碰（docs/autonomy.md）
                if let Some(f) = &face {
                    if let Some(entry) = self.config.kaomoji_resolve(f.as_str()) {
                        face = Some(entry.face.clone());
                    } else {
                        return (
                            json!({ "ok": false, "error": format!("无效 key：'{f}'") }),
                            vec![],
                        );
                    }
                }
                if let Some(m) = &motion {
                    let valid = ["still", "float", "bounce", "shake"];
                    if !valid.contains(&m.as_str()) {
                        return (
                            json!({ "ok": false, "error": format!("motion '{m}' 不合法，合法值：{}", valid.join("/")) }),
                            vec![],
                        );
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
        usage: None,
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
        usage: None,
        }
    }

    fn silence() -> LlmOutput {
        LlmOutput {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
        usage: None,
        }
    }

    #[tokio::test]
    async fn stop_hook_scripted_notify_flow() {
        // mock 脚本：hook 触发后决定通知（set_autonomy + call_component），然后沉默
        let agent = scripted(vec![
            calls(vec![
                (
                    "set_autonomy",
                    json!({"key": "notify", "motion": "bounce", "ttlMs": 5000}),
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
        ov.handle_hook("stop", "ft", "proj", &long, 1).await.unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        assert!(effects.iter().any(|e| matches!(e, Effect::RenderComponent(_))));
        assert!(effects.iter().any(|e| matches!(e, Effect::SetAutonomy { .. })));
        let roles: Vec<Role> = ov.harness.context.messages().iter().map(|m| m.role).collect();
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
        ov.handle_hook("stop", "oss", "proj", "清理了 2 行注释", 1).await.unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        assert!(effects.is_empty());
        let roles: Vec<Role> = ov.harness.context.messages().iter().map(|m| m.role).collect();
        // system(hook)，沉默不追加 assistant
        assert_eq!(roles, vec![Role::System]);
        let _ = std::fs::remove_dir_all(tmp_dir("silence"));
    }

    #[tokio::test]
    async fn session_start_silent_bookkeeping() {
        // 定案（concepts §9b / docs/agent-loop.md mock 契约）：session_start = 静默簿记，
        // pet 不醒——注册 Idle + EventBuffer，不进 Context 不触发 LLM
        let mut ov = make_overseer("register");
        ov.handle_hook("session_start", "new-feature", "proj", "启动画面", 1)
            .await
            .unwrap();
        assert_eq!(ov.harness.agents.len(), 1);
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Idle);
        assert!(ov.harness.context.messages().is_empty()); // 无 Queue 注入
        assert_eq!(ov.harness.event_buffer.len(), 1); // 簿记待附带
        // mock 读链存档仍发生（原文 → terminal-content → Filter → content 存档）
        assert_eq!(
            ov.harness.content.latest("new-feature").unwrap().content,
            "启动画面"
        );
        let _ = std::fs::remove_dir_all(tmp_dir("register"));
    }

    #[tokio::test]
    async fn user_followup_triggers_fetch_loop() {
        // mock 脚本：hook 沉默 → 追问时 fetch_terminal → 汇总回复
        let agent = scripted(vec![
            silence(),
            calls(vec![("fetch_terminal", json!({"instance": "ft", "vd_switch": false}))]),
            say("[debug] 查到：全文"),
        ]);
        let mut ov = make_overseer_with("fetch", agent);
        let long = "y".repeat(100);
        ov.handle_hook("stop", "ft", "proj", &long, 1).await.unwrap();
        ov.drain_queue(0).await.unwrap(); // stop 放行（脚本帧 1 沉默）
        ov.harness
            .append_context(ContextMessage::new(Role::User, "那个 bug 具体怎么回事？", 2))
            .unwrap();
        ov.run_trigger(3, 0).await.unwrap();
        let msgs = ov.harness.context.messages();
        // fetch_terminal 被执行，tool result 含 Context 全文
        assert!(msgs.iter().any(|m| m.role == Role::Tool
            && m.content.as_deref().unwrap_or("").contains(&"y".repeat(100))));
        // 最终 assistant 汇总（脚本原文）
        assert_eq!(msgs.last().unwrap().role, Role::Assistant);
        assert_eq!(msgs.last().unwrap().content.as_deref(), Some("[debug] 查到：全文"));
        let _ = std::fs::remove_dir_all(tmp_dir("fetch"));
    }

    #[tokio::test]
    async fn event_buffer_attached_on_release() {
        // 定案（concepts §10e）：放行 system 输入时 Event Buffer 附带合并为一条消息
        let mut ov = make_overseer("merge");
        ov.harness.event_buffer.push("用户关闭了 text_card「摘要」");
        ov.harness.event_buffer.push("用户勾选了 todobox 条目「跑测试」");
        ov.enqueue(Role::System, "ft 完成。评估是否通知。".into(), 1)
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let sys: Vec<_> = ov
            .harness
            .context
            .messages()
            .iter()
            .filter(|m| m.role == Role::System)
            .collect();
        // 附带合并的一条（输入 + buffer，不独立成条）
        assert_eq!(sys.len(), 1);
        let merged = sys[0].content.as_deref().unwrap();
        assert!(merged.contains("ft 完成"));
        assert!(merged.contains("用户关闭了 text_card「摘要」"));
        assert!(merged.contains("用户勾选了 todobox 条目「跑测试」"));
        assert!(ov.harness.event_buffer.is_empty());
        let _ = std::fs::remove_dir_all(tmp_dir("merge"));
    }

    #[tokio::test]
    async fn event_buffer_keeps_user_role_clean() {
        // 定案（concepts §10e 末句）：与 user role 严格分离——user 输入放行时
        // buffer 以独立 system 消息先行附带，不污染 user 消息
        let mut ov = make_overseer("merge-user");
        ov.harness.event_buffer.push("用户关闭了 text_card「摘要」");
        ov.enqueue(Role::User, "那个 bug 怎么回事？".into(), 1)
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let msgs = ov.harness.context.messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::System);
        assert!(msgs[0]
            .content
            .as_deref()
            .unwrap()
            .contains("用户关闭了 text_card「摘要」"));
        assert_eq!(msgs[1].role, Role::User);
        assert_eq!(msgs[1].content.as_deref(), Some("那个 bug 怎么回事？"));
        let _ = std::fs::remove_dir_all(tmp_dir("merge-user"));
    }

    #[tokio::test]
    async fn plain_user_message_replies() {
        let agent = scripted(vec![say("[debug] 收到：你好")]);
        let mut ov = make_overseer_with("reply", agent);
        ov.harness
            .append_context(ContextMessage::new(Role::User, "你好", 1))
            .unwrap();
        ov.run_trigger(2, 0).await.unwrap();
        let last = ov.harness.context.messages().last().unwrap();
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
        ov.handle_hook("stop", "ft", "proj", &raw, 1).await.unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        assert!(effects.is_empty());
        assert_eq!(ov.harness.content.latest("ft").unwrap().content, "● 完成");
        let _ = std::fs::remove_dir_all(tmp_dir("filter"));
    }

    #[tokio::test]
    async fn timer_scan_substantive_notifies_and_records() {
        // mock 脚本：兜底触发后通知（call_component）→ 沉默
        //（session_start 定案后为静默簿记，不消耗脚本帧）
        let agent = scripted(vec![
            calls(vec![(
                "call_component",
                json!({"spec": {"id": "notify-cship", "type": "text_card", "title": "cship 有变化", "text": "去看看", "direction": "auto"}}),
            )]),
            silence(),
        ]);
        let mut ov = make_overseer_with("timer-sub", agent);
        ov.handle_hook("session_start", "cship", "proj", "旧内容", 1)
            .await
            .unwrap();
        // 兜底扫描读到全新长内容 → Substantive → 存 Context(timer) + 入队 → 放行触发通知
        let new_content = "z".repeat(150);
        ov.handle_timer_scan("cship", &new_content, 2)
            .await
            .unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        let rec = ov.harness.content.latest("cship").unwrap();
        assert_eq!(rec.source, RecordSource::Timer);
        assert_eq!(rec.content, new_content);
        assert!(ov
            .harness
            .context
            .messages()
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("兜底扫描发现变化")));
        assert!(effects.iter().any(|e| matches!(e, Effect::RenderComponent(_))));
        let _ = std::fs::remove_dir_all(tmp_dir("timer-sub"));
    }

    #[tokio::test]
    async fn timer_scan_minor_stays_silent() {
        let mut ov = make_overseer("timer-min");
        ov.handle_hook("session_start", "cship", "proj", "内容不变", 1)
            .await
            .unwrap();
        let msgs_before = ov.harness.context.messages().len();
        // 内容相同 → Unchanged → 存档但不入队不打扰
        ov.handle_timer_scan("cship", "内容不变", 2)
            .await
            .unwrap();
        let effects = ov.drain_queue(0).await.unwrap();
        assert!(effects.is_empty());
        assert_eq!(ov.harness.context.messages().len(), msgs_before);
        assert_eq!(ov.harness.content.latest("cship").unwrap().source, RecordSource::Timer);
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
        ov.handle_hook("stop", "ft", "proj", &raw, 1).await.unwrap();
        let archive = std::fs::read_to_string(
            ov.harness.storage_dir().join(crate::TERMINAL_CONTENT_FILE),
        )
        .unwrap();
        assert!(archive.contains("✻ Crunched for 12s")); // 原文噪音还在
        assert!(archive.contains("\"source\":\"hook\""));
        assert_eq!(ov.harness.content.latest("ft").unwrap().content, "● 完成");
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
            .context
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
        // #16 起触发上限来自 config（effective_compression_limit），不再是 Harness 构造参数
        let dir = tmp_dir("compact");
        let harness = Harness::load(&dir, &dir, 10, 0).unwrap();
        let mut config = Config::default();
        config.llm.providers.insert("debug".into(), crate::config::LlmProvider {
            base_url: String::new(), model: String::new(), api_key_env: None, temperature: None,
            context_window: Some(10), compression_reserve: Some(0),
        });
        let mut ov = OverseerBackend::new(harness, config, DebugAgent::silent());
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
                .append_context(ContextMessage::new(Role::User, format!("第 {i} 条消息内容内容"), i as i64))
                .unwrap();
        }
        ov.run_trigger(10, 0).await.unwrap();
        let msgs = ov.harness.context.messages();
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
        ov.handle_hook("session_start", "ft", "proj", "启动", 1)
            .await
            .unwrap();
        assert!(crate::panorama(&ov.harness.agents).is_some());
        // Timer 发现 tab 不复存在 → closed 终态，全景不再包含
        ov.mark_instance_closed("ft", 2).unwrap();
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Closed);
        assert!(crate::panorama(&ov.harness.agents).is_none());
        // 同名再注册 = 新生命周期（同名不同命，hash 不同）
        ov.handle_hook("session_start", "ft", "proj", "又开了", 3)
            .await
            .unwrap();
        assert_eq!(ov.harness.agents.len(), 2);
        assert_ne!(ov.harness.agents[0].hash, ov.harness.agents[1].hash);
        // stop 沿用最近一条未 closed 的生命周期
        ov.handle_hook("stop", "ft", "proj", "完成", 4).await.unwrap();
        assert_eq!(ov.harness.agents[0].status, AgentStatus::Closed);
        assert_eq!(ov.harness.agents[1].status, AgentStatus::Idle);
        let _ = std::fs::remove_dir_all(tmp_dir("closed"));
    }

    #[tokio::test]
    async fn hook_resets_timer_wheel() {
        let mut ov = make_overseer("timer-reset");
        ov.handle_hook("session_start", "a", "proj", "x", 1000)
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
            arguments: json!({ "key": "notify", "ttlMs": 3000 }).to_string(),
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
        // 非 key 的 face 拒绝（docs/toolset.md：必须用 key 名）
        let call2 = ToolCall {
            id: "c2".into(),
            name: "set_autonomy".into(),
            arguments: json!({ "key": "(・ω・)ノ" }).to_string(),
        };
        let (result2, _) = ov.execute_tool(&call2);
        assert_eq!(result2["ok"], json!(false));
        assert!(result2["error"].as_str().unwrap().contains("无效 key"));
        let _ = std::fs::remove_dir_all(tmp_dir("face-key"));
    }

    #[tokio::test]
    async fn streaming_delta_flows_to_sink() {
        // 默认回落路径（docs/streaming.md）：complete 一次性 → 全文单 delta → AssistantDone
        let agent = scripted(vec![say("流式回复全文")]);
        let mut ov = make_overseer_with("stream", agent);
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Effect>::new()));
        let got2 = got.clone();
        ov.effect_sink = Some(std::sync::Arc::new(move |e: &Effect| {
            got2.lock().unwrap().push(e.clone());
        }));
        ov.enqueue(Role::User, "打个招呼".into(), 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        let got = got.lock().unwrap();
        // 完整回复作为单个 delta 到达 + Done 收尾；回复本体已写 Context
        assert!(got.iter().any(|e| matches!(
            e,
            Effect::AssistantDelta { content: Some(c), .. } if c == "流式回复全文"
        )));
        assert!(got.iter().any(|e| matches!(e, Effect::AssistantDone)));
        assert_eq!(
            ov.harness.context.messages().last().unwrap().content.as_deref(),
            Some("流式回复全文")
        );
        let _ = std::fs::remove_dir_all(tmp_dir("stream"));
    }

    #[tokio::test]
    async fn no_sink_no_delta_no_panic() {
        // 未接 sink 时流式路径静默无副作用（debug/测试模式默认）
        let agent = scripted(vec![say("你好")]);
        let mut ov = make_overseer_with("stream-none", agent);
        ov.enqueue(Role::User, "hi".into(), 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        assert_eq!(
            ov.harness.context.messages().last().unwrap().content.as_deref(),
            Some("你好")
        );
        let _ = std::fs::remove_dir_all(tmp_dir("stream-none"));
    }

    #[tokio::test]
    async fn call_component_continuous_management() {
        // 持续管理协议（docs/components.md）：同 id = 原地更新，close action 显式关闭
        let mut ov = make_overseer("cmp-mgmt");
        let mk = |text: &str| crate::context::ToolCall {
            id: "c1".into(),
            name: "call_component".into(),
            arguments: json!({"spec": {"id": "todo-1", "type": "todobox", "title": "t", "items": [{"text": text, "done": false}]}}).to_string(),
        };
        // 创建 → rendered
        let (r1, e1) = ov.execute_tool(&mk("a"));
        assert_eq!(r1["rendered"], json!("todo-1"));
        assert!(matches!(e1[0], Effect::RenderComponent(_)));
        assert!(ov.cards.contains_key("todo-1"));
        // 同 id → updated（不再 toggle 关闭）
        let (r2, e2) = ov.execute_tool(&mk("b"));
        assert_eq!(r2["updated"], json!("todo-1"));
        assert!(matches!(e2[0], Effect::RenderComponent(_)));
        assert!(ov.cards.contains_key("todo-1"));
        // close action → closed + CloseComponent effect
        let close_call = crate::context::ToolCall {
            id: "c2".into(),
            name: "call_component".into(),
            arguments: json!({"spec": {"id": "todo-1", "type": "todobox", "action": "close"}}).to_string(),
        };
        let (r3, e3) = ov.execute_tool(&close_call);
        assert_eq!(r3["closed"], json!("todo-1"));
        assert!(matches!(e3[0], Effect::CloseComponent(_)));
        assert!(!ov.cards.contains_key("todo-1"));
        // 生命周期事件（docs/components.md）：created 一行 + closed 一行，均进 EventBuffer 静默簿记
        let owned = ov.harness.event_buffer.events();
        let events: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
        assert!(events.iter().any(|l| l.starts_with("card created: todobox「t」(todo-1) @ ") && l.ends_with(", → 存活 1")), "created 事件: {events:?}");
        assert!(events.iter().any(|l| l.starts_with("card closed: todobox「t」(todo-1), ") && l.contains(" / ") && l.ends_with(", → 存活 0")), "closed 事件: {events:?}");
        let _ = std::fs::remove_dir_all(tmp_dir("cmp-mgmt"));
    }

    #[tokio::test]
    async fn call_component_close_action_outside_spec() {
        // #23：LLM 把 action="close" 放在 args 顶层（与 spec 并列）时，
        // 回退识别为 close，而不是当成空 update 渲染空卡
        let mut ov = make_overseer("cmp-close-outside");
        let create = crate::context::ToolCall {
            id: "c1".into(),
            name: "call_component".into(),
            arguments: json!({"spec": {"id": "demo_line", "type": "text_card", "title": "t", "text": "x"}}).to_string(),
        };
        let (r1, _) = ov.execute_tool(&create);
        assert_eq!(r1["rendered"], json!("demo_line"));
        let close = crate::context::ToolCall {
            id: "c2".into(),
            name: "call_component".into(),
            arguments: json!({"action": "close", "spec": {"id": "demo_line"}}).to_string(),
        };
        let (r2, e2) = ov.execute_tool(&close);
        assert_eq!(r2["closed"], json!("demo_line"));
        assert!(matches!(e2[0], Effect::CloseComponent(_)));
        assert!(!ov.cards.contains_key("demo_line"));
        let _ = std::fs::remove_dir_all(tmp_dir("cmp-close-outside"));
    }

    #[tokio::test]
    async fn plain_reply_keeps_reasoning_content_in_context() {
        // thinking 全保真：纯文本回复的 reasoning_content 也落 Context（记录≠回放）
        let agent = DebugAgent::new(|_| LlmOutput {
            content: Some("答案".into()),
            tool_calls: vec![],
            reasoning_content: Some("先想三步再答".into()),
            usage: None,
        });
        let mut ov = make_overseer_with("reason-keep", agent);
        ov.enqueue(Role::User, "问".into(), 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        let last = ov.harness.context.messages().last().unwrap();
        assert_eq!(last.role, Role::Assistant);
        assert_eq!(last.reasoning_content.as_deref(), Some("先想三步再答"));
        let _ = std::fs::remove_dir_all(tmp_dir("reason-keep"));
    }

    #[tokio::test]
    async fn compression_triggers_on_usage_truth() {

        // #16 真值触发：last_usage.prompt_tokens + est 增量 > 阈值 → 压缩
        let big = crate::llm::Usage { prompt_tokens: 900_000, completion_tokens: 0 };
        let agent = DebugAgent::new(move |_| LlmOutput {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
            usage: Some(big),
        });
        let mut ov = make_overseer_with("compress-truth", agent);
        ov.config.llm.providers.insert("debug".into(), crate::config::LlmProvider {
            base_url: String::new(), model: String::new(), api_key_env: None, temperature: None,
            context_window: Some(100), compression_reserve: Some(0),
        });
        // 第一轮：last_usage 还是 None → est 兜底不触发；当轮落真值
        ov.enqueue(Role::User, "第一轮".into(), 1).unwrap();
        ov.drain_queue(0).await.unwrap();
        assert_eq!(ov.harness.last_usage, Some(big));
        // 第二轮：真值 900K + 增量 ≫ 100 → 触发压缩
        ov.enqueue(Role::User, "第二轮".into(), 2).unwrap();
        ov.drain_queue(0).await.unwrap();
        let first = ov.harness.context.messages()[0]
            .content
            .as_deref()
            .unwrap_or("");
        assert!(first.contains("[历史摘要]"), "应触发压缩: {first}");
        let _ = std::fs::remove_dir_all(tmp_dir("compress-truth"));
    }

    #[tokio::test]
    async fn compression_triggers_on_est_fallback_without_usage() {
        // #16 兜底：DebugAgent 默认无 usage → 全量 est 触发（现状路径）
        let mut ov = make_overseer("compress-est");
        ov.config.llm.providers.insert("debug".into(), crate::config::LlmProvider {
            base_url: String::new(), model: String::new(), api_key_env: None, temperature: None,
            context_window: Some(50), compression_reserve: Some(0),
        });
        for i in 0..30 {
            ov.enqueue(Role::User, format!("第 {i} 条消息内容内容内容"), i as i64)
                .unwrap();
        }
        ov.drain_queue(0).await.unwrap();
        let first = ov.harness.context.messages()[0]
            .content
            .as_deref()
            .unwrap_or("");
        assert!(first.contains("[历史摘要]"), "est 兜底应触发压缩: {first}");
        let _ = std::fs::remove_dir_all(tmp_dir("compress-est"));
    }

    #[tokio::test]
    async fn real_hook_stop_three_modes() {
        let sid = "dddd3333-4444-5555";
        // B（默认 queue_only）：hint 形态
        let mut ov = make_overseer("rh5");
        let _ = ov
            .handle_real_hook("stop", sid, r"/tmp/p", None, None, None, Some("修了 3 个文件"), 1000)
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let m = ov.harness.context.messages().last().unwrap().content.clone().unwrap();
        assert!(m.contains("完成：修了 3 个文件。评估是否通知"), "B: {m}");
        let _ = std::fs::remove_dir_all(tmp_dir("rh5"));

        // C（message）：汇报原文直达
        let mut ov = make_overseer("rh6");
        ov.config.stop_hook_mode = "message".into();
        let _ = ov
            .handle_real_hook("stop", sid, r"/tmp/p", None, None, None, Some("修了 3 个文件"), 1000)
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let m = ov.harness.context.messages().last().unwrap().content.clone().unwrap();
        assert_eq!(m, "[汇报] p·dddd3333 完成：修了 3 个文件");
        let _ = std::fs::remove_dir_all(tmp_dir("rh6"));

        // A（auto_read）：读通道全量,Context 更新
        let mut ov = make_overseer("rh7");
        ov.config.stop_hook_mode = "auto_read".into();
        ov.terminal_reader = Some(std::sync::Arc::new(|inst: &str| {
            assert_eq!(inst, "p·dddd3333");
            Some("● 完成。hooks 已配置".to_string())
        }));
        let _ = ov
            .handle_real_hook("stop", sid, r"/tmp/p", None, None, None, None, 1000)
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let m = ov.harness.context.messages().last().unwrap().content.clone().unwrap();
        assert!(m.contains("Context 已更新"), "A: {m}");
        let ctx = ov.harness.content.latest("p·dddd3333").expect("context 已写");
        assert!(ctx.content.contains("hooks 已配置"));
        let _ = std::fs::remove_dir_all(tmp_dir("rh7"));
    }

    #[tokio::test]
    async fn startup_sweep_full_flow() {
        let mut ov = make_overseer("sweep");
        // 预置一具占位尸体（标题已消失）
        ov.harness
            .upsert_agent(AgentEntry {
                hash: "uia:gone".into(),
                name: "gone".into(),
                project: "unknown".into(),
                kind: None,
                status: AgentStatus::Idle,
                tab: None,
                first_seen: 0,
                last_seen: 0,
            })
            .unwrap();
        let call = |req: &Value| -> Option<Value> {
            match req["cmd"].as_str()? {
                "list_windows" => Some(json!({"windows":[
                    {"hwnd":100,"title":"✳ npc-prof·3f8a2c1e","cloaked":false},
                    {"hwnd":200,"title":"✳ gumtree","cloaked":false},
                    {"hwnd":300,"title":"✳ 别的桌面·aaaa0000","cloaked":true},
                    {"hwnd":400,"title":"Neovim","cloaked":false}
                ]})),
                "list_tabs" => match req["hwnd"].as_i64()? {
                    100 => Some(json!({"tabs":[{"index":2,"name":"✳ npc-prof·3f8a2c1e","selected":true}]})),
                    200 => Some(json!({"tabs":[{"index":0,"name":"✳ gumtree","selected":false}]})),
                    _ => None,
                },
                "count_processes" => Some(json!({"count":54})),
                _ => None,
            }
        };
        ov.startup_sweep(&call, 1000).await.unwrap();
        // marker 注册（带 tab 定位 + kind）
        let a = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "3f8a2c1e")
            .expect("marker 注册");
        assert_eq!(a.name, "npc-prof·3f8a2c1e");
        assert_eq!(a.tab, Some(crate::TabRef { hwnd: 100, index: 2 }));
        assert_eq!(a.kind.as_deref(), Some("claude"));
        // 占位入册
        let ph = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "uia:gumtree")
            .expect("占位入册");
        assert_eq!(ph.kind.as_deref(), Some("claude"));
        // cloaked 窗口标题带 marker:注册但无 tab 定位
        let c = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "aaaa0000")
            .expect("cloaked 注册");
        assert_eq!(c.tab, None);
        // 占位尸体 closed
        let g = ov
            .harness
            .agents
            .iter()
            .find(|a| a.hash == "uia:gone")
            .unwrap();
        assert_eq!(g.status, AgentStatus::Closed);
        // 对账行进 EventBuffer
        let line = ov.harness.event_buffer.merge_and_clear().unwrap_or_default();
        assert!(line.contains("54") && line.contains("cloaked"), "{line}");
        let _ = std::fs::remove_dir_all(tmp_dir("sweep"));
    }

    #[tokio::test]
    async fn fetch_terminal_vd_switch_semantics() {
        // 必填:忘传报错教学
        let mut ov = make_overseer("vd1");
        let call = ToolCall { id: "c1".into(), name: "fetch_terminal".into(), arguments: json!({"instance":"x"}).to_string() };
        let (r, _) = ov.execute_tool(&call);
        assert!(r["error"].as_str().unwrap_or("").contains("vd_switch 必填"), "{r}");
        // false 且读不到:报错含重试提示
        let call2 = ToolCall { id: "c2".into(), name: "fetch_terminal".into(), arguments: json!({"instance":"x","vd_switch":false}).to_string() };
        let (r2, _) = ov.execute_tool(&call2);
        assert!(r2["error"].as_str().unwrap_or("").contains("vd_switch=true 重试"), "{r2}");
        let _ = std::fs::remove_dir_all(tmp_dir("vd1"));

        // true + 切换成功:重读命中
        let mut ov = make_overseer("vd2");
        ov.terminal_reader = Some(std::sync::Arc::new(|inst: &str| {
            if std::env::var("VD_TEST_READY").is_ok() { Some(format!("内容:{inst}")) } else { None }
        }));
        ov.vd_switcher = Some(std::sync::Arc::new(|_: &str| {
            std::env::set_var("VD_TEST_READY", "1");
            true
        }));
        let call3 = ToolCall { id: "c3".into(), name: "fetch_terminal".into(), arguments: json!({"instance":"x","vd_switch":true}).to_string() };
        let (r3, _) = ov.execute_tool(&call3);
        std::env::remove_var("VD_TEST_READY");
        assert_eq!(r3["content"].as_str().unwrap_or(""), "内容:x");
        let _ = std::fs::remove_dir_all(tmp_dir("vd2"));
    }

    #[tokio::test]
    async fn real_hook_first_sight_registers_silently() {
        let mut ov = make_overseer("rh1");
        ov.handle_real_hook(
                "session_start",
                "3f8a2c1e-9b7d-4e5f-a6c1-02d4e6f8a9b0",
                r"/tmp/p",
                Some("claude"),
                None,
                None,
                None,
                1000,
            )
            .await
            .unwrap();
        // 静默：只入 EventBuffer 簿记，不触发 LLM
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
        assert!(ov.harness.context.messages().is_empty()); // 无 queue 注入
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
            )
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
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
            .context
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
            .handle_real_hook("session_start", sid, r"/tmp/p", None, None, None, None, 1000)
            .await
            .unwrap();
        let _ = ov
            .handle_real_hook("user_prompt", sid, r"/tmp/p", None, Some("帮我修 bug"), None, None, 2000)
            .await
            .unwrap();
        ov.drain_queue(0).await.unwrap();
        let a = ov.harness.agents.iter().find(|a| a.hash == "cccc2222").unwrap();
        assert_eq!(a.status, AgentStatus::Processing); // 派活驱动
        assert!(ov
            .harness
            .context
            .messages()
            .iter()
            .any(|m| m.content.as_deref().unwrap_or("").contains("[观察] 用户在")));
        let _ = ov
            .handle_real_hook("session_end", sid, r"/tmp/p", None, None, None, None, 3000)
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
                "path": "kaomoji.user.celebrate",
                "value": { "face": "(≧▽≦)", "motion": "bounce" }
            })
            .to_string(),
        };
        let (result, effects) = ov.execute_tool(&call);
        assert_eq!(result["ok"], json!(true));
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::ConfigChanged { llm_changed: false })));
        assert_eq!(ov.config.kaomoji.user["celebrate"].face, "(≧▽≦)");
        // config.json 已持久化
        let reloaded = Config::load_or_default(ov.harness.config_dir());
        assert_eq!(reloaded.kaomoji.user["celebrate"].motion, "bounce");
        let _ = std::fs::remove_dir_all(tmp_dir("cfg"));
    }

    #[tokio::test]
    async fn kaomoji_pools_invariants_enforced_on_update() {
        // 两池校验（docs/config.md §表情池）：写入管道原子拒绝违反不变量的 candidate
        let mut ov = make_overseer("pools");
        // ① 交集为空：user 池新增与 system 重复的 key → 拒绝
        assert!(ov
            .apply_config_by_path("kaomoji.user.idle", json!({"face": "x", "motion": "still"}))
            .is_err());
        // ② 基础 key 在并集：移除 system 池（整体替换为空）→ 拒绝
        assert!(ov.apply_config_by_path("kaomoji.system", json!({})).is_err());
        // 合法：基础 key 移到 user 池（单次整节点写入 = 原子移动，并集仍齐）→ 通过
        let mut pools = serde_json::to_value(&ov.config.kaomoji).unwrap();
        let idle = pools["system"].as_object().unwrap()["idle"].clone();
        pools["system"].as_object_mut().unwrap().remove("idle");
        pools["user"]
            .as_object_mut()
            .unwrap()
            .insert("idle".into(), idle);
        ov.apply_config_by_path("kaomoji", pools).unwrap();
        assert!(ov.config.kaomoji.user.contains_key("idle"));
        assert!(!ov.config.kaomoji.system.contains_key("idle"));
        // 并集解析不受池归属影响（docs/config.md：移动后仍参与默认状态与按 key 解析）
        assert_eq!(ov.config.kaomoji_resolve("idle").unwrap().face, "(´ω`)");
        let _ = std::fs::remove_dir_all(tmp_dir("pools"));
    }

    #[test]
    fn apply_config_by_path_hot_apply_and_persist() {
        let mut ov = make_overseer("apply");
        let out = ov
            .apply_config_by_path("compression_reserve_default", json!(5000))
            .unwrap();
        assert!(out.restart_required.is_empty());
        assert!(!out.llm_changed);
        assert_eq!(ov.config.compression_reserve_default, 5000); // 热应用
        let reloaded = Config::load_or_default(ov.harness.config_dir());
        assert_eq!(reloaded.compression_reserve_default, 5000); // persist
        let _ = std::fs::remove_dir_all(tmp_dir("apply"));
    }

    #[test]
    fn apply_config_by_path_validates_and_reports_restart() {
        let mut ov = make_overseer("apply2");
        // serde 验证失败
        assert!(ov.apply_config_by_path("compression_reserve_default", json!("oops")).is_err());
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
        assert!(ov.apply_config_by_path("compression_reserve_default", json!(1)).is_err());
        let _ = std::fs::remove_dir_all(tmp_dir("apply3"));
    }
}
