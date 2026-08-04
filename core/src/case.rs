//! case-runner（docs/case-runner.md）：两段式 .case 解析、step 类型与概念观测。
//! feature "case-runner" gate。

use crate::llm::Llm;
use crate::observe::{
    AgentSnapshot, CronSnapshot, FilteredContentSnapshot, MemoryNoteSnapshot, MessageSnapshot,
    Observable,
};
use crate::overseer::OverseerBackend;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 概念结构观测快照（内容级，docs/case-runner.md §observe 输出）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseObserve {
    pub agents: Vec<AgentSnapshot>,
    pub panorama: Option<String>,
    /// Context 消息数组（role / content / tool_calls）
    pub context: Vec<MessageSnapshot>,
    /// Filtered 内容（归一全文现算：terminal-content.jsonl 原文 digest，不持久化）
    pub filtered_content: Vec<FilteredContentSnapshot>,
    /// Queue 待放行输入
    pub queue: Vec<crate::queue::QueueInput>,
    /// Event Buffer 积压原文
    pub event_buffer: Vec<String>,
    /// 最近一次 LLM 调用真值（#16；无 = 未调用过/重启后）
    pub usage: Option<crate::llm::Usage>,
    /// usage 写入时的 ts（时间锚点；usage 为 None 时同为 None）
    pub usage_ts: Option<i64>,
    /// 自真值落点后的 est 增量（无真值时 = 全量 est）
    pub context_est_delta: usize,
    /// 最后一条 assistant 消息原文（回答准确度扫读位）
    pub answer: Option<String>,
    /// Memory index 摘要（name / description；不默认展开正文，docs/observability.md）
    pub memory: Vec<MemoryNoteSnapshot>,
    /// Cron 持久化计划投影（id / schedule / message / next_due；不含 sleep waiter）
    pub cron: Vec<CronSnapshot>,
    /// 动作流（从 effect.jsonl 读）：后端副作用 + 前端非只读调用（docs/storage.md §effect.jsonl）
    pub effects: Vec<crate::EffectRecord>,
}

/// 观测当前概念结构：模块快照走 Observable 投影（docs/observability.md），
/// 派生项（panorama / context_est_delta / answer）手写组装。
pub fn observe<L: Llm>(ov: &OverseerBackend<L>) -> CaseObserve {
    let h = &ov.harness;
    let panorama = crate::panorama(&h.agents);
    let context_est_delta = h.context.est_tokens_since(h.last_usage_msg_len);
    let answer = h
        .context
        .messages()
        .iter()
        .rev()
        .find(|m| m.role == crate::context::Role::Assistant)
        .and_then(|m| m.content.clone());
    CaseObserve {
        agents: h.agents.observe(),
        panorama,
        context: h.context.observe(),
        filtered_content: ov
            .filtered_content()
            .into_iter()
            .map(|r| FilteredContentSnapshot {
                instance: r.instance,
                filtered_content: r.filtered_content,
                source: format!("{:?}", r.source).to_lowercase(),
                ts: r.ts,
            })
            .collect(),
        queue: h.queue.observe(),
        event_buffer: h.event_buffer.observe(),
        usage: h.last_usage.observe(),
        usage_ts: h.last_usage_ts,
        context_est_delta,
        answer,
        memory: h.memory.observe(),
        cron: h.cron.observe(),
        effects: h.read_effects().unwrap_or_default(),
    }
}

// ── 两段式 .case 格式（docs/case-runner.md §Case 文件格式）──

/// 解析规则（唯一一条）：首个无缩进 `{"__section":` 行之前 = JSON 头；
/// 之后按 marker 行分节，数据行归当前节。
pub const SECTION_MARKER: &str = "{\"__section\":";

/// memory 节文件标记（docs/case-runner.md §数据节）：`{"__file":"notes/<name>.md"}`
/// 行起到下一标记的行为该文件原文（Markdown 原文区，不转义、不按 JSONL 解析）。
pub const FILE_MARKER: &str = "{\"__file\":";

/// case memory 节的一个文件（路径 + Markdown 原文）
#[derive(Debug)]
pub struct CaseMemoryFile {
    /// `AGENTS.md` 或 `notes/<name>.md`（index.md 不进 case，沙盒按已选普通记忆重建）
    pub path: String,
    pub content: String,
}

#[derive(Debug)]
pub struct CaseFile {
    pub meta: CaseMeta,
    pub config: Value,
    pub steps: Vec<CaseStep>,
    /// 数据节原文（行序原样）
    pub work_agents: String,
    pub context: String,
    pub queue: String,
    /// cron.jsonl 原文（行序原样；默认过滤，--keep-cron + --cron-ids 才进 case）
    pub cron: String,
    /// memory/ 原文文件区（默认过滤，--keep-memory + --memory 才进 case）
    pub memory: Vec<CaseMemoryFile>,
}

#[derive(Debug, Deserialize)]
struct CaseHead {
    meta: CaseMeta,
    #[serde(default)]
    config: Value,
    #[serde(default)]
    steps: Vec<CaseStep>,
}

/// LLM 模式（docs/case-runner.md §LLM 模式）：两种平级无默认，case 头部 meta 必填声明
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmMode {
    /// DebugAgent，零网络，决策由外部决策源注入（沉默/脚本/CLI），确定性
    Debug,
    /// OpenAI 兼容真实端点：provider 从生产 providers 合并，key 只取环境变量
    Real,
}

#[derive(Debug, Deserialize)]
pub struct CaseMeta {
    pub case_id: String,
    pub created: String,
    #[serde(default)]
    pub notes: String,
    /// 必填：缺声明 = case 不合法（serde 缺字段报错），不隐含退回任一模式
    pub llm_mode: LlmMode,
}

/// no_case_visible（docs/case-runner.md §case 隐私）：case 禁止携带 llm.providers.*
/// （base_url / api_key_env 等敏感字段），覆盖校验拒绝——case 绝不携带 apikey。
/// 扁平静 key（"llm.providers.x"）与嵌套形（{"llm":{"providers":...}}）都拦。
fn check_no_case_visible(config: &Value) -> Result<(), String> {
    fn walk(v: &Value, path: String, out: &mut Vec<String>) {
        if let Value::Object(map) = v {
            for (k, val) in map {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                out.push(p.clone());
                walk(val, p, out);
            }
        }
    }
    let mut paths = vec![];
    walk(config, String::new(), &mut paths);
    for p in paths {
        if p == "llm.providers" || p.starts_with("llm.providers.") {
            return Err(format!(
                "no_case_visible：config 禁止携带 {p}（provider 配置/key 不进 case）"
            ));
        }
    }
    Ok(())
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
        } else {
            // memory 节是 Markdown 原文区：空行是文件内容的一部分，不按 JSONL 节丢弃
            let in_memory = sections.last().is_some_and(|(n, _)| n == "memory");
            if !line.trim().is_empty() || in_memory {
                sections.last_mut().unwrap().1.push(line.to_string());
            }
        }
    }
    let head: CaseHead = serde_json::from_str(&head_lines.join("\n"))
        .map_err(|e| format!("JSON 头解析失败: {e}"))?;
    check_no_case_visible(&head.config)?;
    let take = |name: &str| -> String {
        sections
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, rows)| rows.join("\n"))
            .unwrap_or_default()
    };
    let memory_rows = sections
        .iter()
        .find(|(n, _)| n == "memory")
        .map(|(_, rows)| rows.as_slice())
        .unwrap_or(&[]);
    Ok(CaseFile {
        meta: head.meta,
        config: head.config,
        steps: head.steps,
        work_agents: take("work_agents"),
        context: take("context"),
        queue: take("queue"),
        cron: take("cron"),
        memory: parse_memory_section(memory_rows)?,
    })
}

/// memory 节解析：文件标记行分文件，行间原文保留（含空行；文件尾空行视为分节 padding 裁掉）
fn parse_memory_section(rows: &[String]) -> Result<Vec<CaseMemoryFile>, String> {
    let mut files: Vec<CaseMemoryFile> = vec![];
    let mut cur: Option<(String, Vec<String>)> = None;
    let mut flush = |cur: &mut Option<(String, Vec<String>)>| {
        if let Some((path, mut body)) = cur.take() {
            while body.last().is_some_and(|l| l.trim().is_empty()) {
                body.pop();
            }
            files.push(CaseMemoryFile {
                path,
                content: body.join("\n"),
            });
        }
    };
    for row in rows {
        if row.starts_with(FILE_MARKER) {
            flush(&mut cur);
            let v: Value =
                serde_json::from_str(row).map_err(|e| format!("memory 文件标记行非法 JSON: {e}"))?;
            let path = v["__file"]
                .as_str()
                .ok_or_else(|| "memory 文件标记缺 __file 字符串".to_string())?;
            check_memory_path(path)?;
            cur = Some((path.to_string(), vec![]));
        } else if row.starts_with("{\"__") {
            // 原文行不得与标记语法撞车（否则被误认为新标记，静默断文件）
            return Err(format!(
                "memory 节原文行不得以 `{{\"__` 行首开头（与节/文件标记冲突）: {}",
                &row[..row.len().min(40)]
            ));
        } else {
            match &mut cur {
                Some((_, body)) => body.push(row.clone()),
                None => {
                    if row.trim().is_empty() {
                        continue; // 节首 padding
                    }
                    return Err(format!(
                        "memory 节首行必须是 {FILE_MARKER} 文件标记，得到: {}",
                        &row[..row.len().min(40)]
                    ));
                }
            }
        }
    }
    flush(&mut cur);
    Ok(files)
}

/// memory 节路径白名单（docs/case-runner.md §数据节）：`AGENTS.md` 或
/// `notes/<name>.md`（name 经文件名 grammar 且非保留名）；index.md 不导出（沙盒重建）、
/// cards/ 不经此节（契约另定）。
fn check_memory_path(path: &str) -> Result<(), String> {
    if path == "AGENTS.md" {
        return Ok(());
    }
    let name = path
        .strip_prefix("notes/")
        .and_then(|s| s.strip_suffix(".md"))
        .filter(|n| crate::memory::valid_name(n) && !crate::memory::RESERVED.contains(n))
        .ok_or_else(|| {
            format!(
                "memory 节路径 '{path}' 不合法（只允许 AGENTS.md 或 notes/<name>.md；\
                 index.md 不进 case、沙盒重建，cards/ 不经此节）"
            )
        })?;
    let _ = name;
    Ok(())
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
    Store { store: std::collections::HashMap<String, StoreValue> },
    Terminal { terminal: Value },
    TerminalGone { terminal_gone: Value },
    Observe { observe: Vec<ObserveItem> },
}

/// store step 的变量设置（docs/case-eval-system.md §变量）：
/// `{ "<name>": { "type": "expr|var|int|str", "value": "<字符串>" } }`
#[derive(Debug, Deserialize)]
pub struct StoreValue {
    #[serde(rename = "type")]
    pub ty: String,
    pub value: String,
}

/// observe 项（统一对象列表）：路径类 target（context/effects）可带 lines 读取文件切片
#[derive(Debug, Deserialize)]
pub struct ObserveItem {
    pub target: String,
    #[serde(default)]
    pub lines: Option<String>,
}

impl ObserveItem {
    /// 路径类 target（完整数据在沙盒文件，observe 给文件指针+摘要 / lines 切片）
    pub fn is_path_class(&self) -> bool {
        matches!(self.target.as_str(), "context" | "effects")
    }
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
            CaseStep::Store { .. } => "store",
            CaseStep::Terminal { .. } => "terminal",
            CaseStep::TerminalGone { .. } => "terminal_gone",
            CaseStep::Observe { .. } => "observe",
        }
    }
}

/// pre-parse 预检（docs/case-eval-system.md §checkhealth）：静态校验，不执行 case。
/// 检查项：① 表达式 try_parse 语法合法；② 变量引用有效（$tail 预定义、用户变量使用前
/// 已 store）；③ store 类型合法（expr/var/int/str）；④ 类型可落（eval_store 的
/// DirectToString 泛型约束编译期保证）；⑤ observe target 合法（可观测模块；lines 仅路径类）。
/// 返回失败清单（空 = 通过）。
pub fn pre_parse_check(case: &CaseFile) -> Vec<String> {
    use crate::eval::{ExprParser, IntParser, Parser, RangeParser, VarEnv, VarIntParser};
    const TARGETS: &[&str] = &[
        "agents", "panorama", "context", "filtered_content", "queue", "event_buffer",
        "usage", "effects", "answer", "memory", "cron",
    ];
    let mut failures = vec![];
    // 已 store 的用户变量名（预检环境用占位值：引用有效性与语法在同一遍检查）
    let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mk_env = |known: &std::collections::HashSet<String>| VarEnv {
        tail: 0,
        vars: known.iter().map(|k| (k.clone(), "0".into())).collect(),
    };
    for (i, step) in case.steps.iter().enumerate() {
        match step {
            CaseStep::Store { store } => {
                let env = mk_env(&known);
                for (name, sv) in store {
                    if !matches!(sv.ty.as_str(), "expr" | "var" | "int" | "str") {
                        failures.push(format!(
                            "step {} store ${name}: 类型 {:?} 不合法（expr/var/int/str）",
                            i + 1,
                            sv.ty
                        ));
                        continue;
                    }
                    let r = match sv.ty.as_str() {
                        "expr" => ExprParser { env: &env }.try_parse(sv.value.as_str()),
                        "var" => VarIntParser { env: &env }.try_parse(sv.value.as_str()),
                        "int" => IntParser.try_parse(sv.value.as_str()),
                        _ => Ok(()), // str 直存不解析
                    };
                    if let Err(e) = r {
                        failures.push(format!("step {} store ${name}（{:?}）: {e}", i + 1, sv.value));
                    }
                }
                // 同 step 内变量不互相可见：先全量校验，再统一入册
                for name in store.keys() {
                    known.insert(name.clone());
                }
            }
            CaseStep::Observe { observe } => {
                let env = mk_env(&known);
                for item in observe {
                    if !TARGETS.contains(&item.target.as_str()) {
                        failures.push(format!(
                            "step {} observe: 未知 target {:?}（可观测模块：{}）",
                            i + 1,
                            item.target,
                            TARGETS.join("/")
                        ));
                        continue;
                    }
                    if let Some(lines) = &item.lines {
                        if !item.is_path_class() {
                            failures.push(format!(
                                "step {} observe {:?}: 值类 target 不支持 lines（路径类：context/effects）",
                                i + 1,
                                item.target
                            ));
                        } else if let Err(e) =
                            (RangeParser { env: &env }).try_parse(lines.as_str())
                        {
                            failures.push(format!(
                                "step {} observe {:?} lines {:?}: {e}",
                                i + 1,
                                item.target,
                                lines
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_two_stage_format() {
        let text = r#"{
  "meta": { "case_id": "t", "created": "2026-07-30T00:00:00Z", "llm_mode": "debug" },
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
        assert_eq!(case.meta.llm_mode, LlmMode::Debug);
        assert_eq!(case.steps.len(), 2);
        assert_eq!(case.steps[1].name(), "terminal_gone");
        assert_eq!(case.work_agents.lines().count(), 1);
        assert!(case.work_agents.contains("a1"));
        assert_eq!(case.context.lines().count(), 1);
        assert!(case.queue.is_empty());
    }

    #[test]
    fn parse_empty_sections_kept_distinct() {
        let text = "{\n \"meta\": {\"case_id\":\"t\",\"created\":\"x\",\"llm_mode\":\"real\"}\n}\n{\"__section\":\"=== work_agents ===\"}\n{\"__section\":\"=== context ===\"}\n{\"type\":\"message\",\"role\":\"system\",\"content\":\"hi\",\"ts\":1}\n";
        let case = parse(text).unwrap();
        assert_eq!(case.meta.llm_mode, LlmMode::Real);
        assert!(case.work_agents.is_empty()); // 空节 ≠ 缺节
        assert_eq!(case.context.lines().count(), 1);
    }

    #[test]
    fn llm_mode_required_and_validated() {
        // 缺声明 = case 不合法（两种平级无默认，不隐含退回）
        let missing = r#"{ "meta": { "case_id": "t", "created": "x" } }"#;
        let err = parse(missing).unwrap_err();
        assert!(err.contains("llm_mode"), "{err}");
        // 非法值同样不合法
        let bogus = r#"{ "meta": { "case_id": "t", "created": "x", "llm_mode": "bogus" } }"#;
        assert!(parse(bogus).is_err());
    }

    #[test]
    fn no_case_visible_rejects_providers() {
        // 扁平点路径形态
        let flat = r#"{ "meta": { "case_id": "t", "created": "x", "llm_mode": "real" },
  "config": { "llm.providers.foo.base_url": "https://x" } }"#;
        let err = parse(flat).unwrap_err();
        assert!(err.contains("no_case_visible"), "{err}");
        // 嵌套形态
        let nested = r#"{ "meta": { "case_id": "t", "created": "x", "llm_mode": "real" },
  "config": { "llm": { "providers": { "foo": { "base_url": "https://x" } } } } }"#;
        assert!(parse(nested).unwrap_err().contains("no_case_visible"));
        // llm.active 是 provider 名引用（不是 provider 配置）——允许
        let ok = r#"{ "meta": { "case_id": "t", "created": "x", "llm_mode": "real" },
  "config": { "llm.active": "foo" } }"#;
        assert!(parse(ok).is_ok());
    }

    // ── memory/cron 数据节（docs/case-runner.md §数据节）──

    #[test]
    fn parse_memory_section_files_and_blank_lines() {
        let text = "{\n  \"meta\": { \"case_id\": \"t\", \"created\": \"x\", \"llm_mode\": \"debug\" }\n}\n\
            {\"__section\":\"=== memory ===\"}\n\
            {\"__file\":\"AGENTS.md\"}\n# Memory Workspace\n导航\n\
            {\"__file\":\"notes/work-preferences.md\"}\n---\ndescription: 用户的工作偏好\n---\n\n- 不擅自提交\n\n\
            {\"__section\":\"=== cron ===\"}\n\
            {\"op\":\"create\",\"id\":\"a1\",\"schedule\":{\"every_ms\":60000},\"message\":\"日报\",\"next_due\":61000,\"ts\":1000}\n";
        let case = parse(text).unwrap();
        assert_eq!(case.memory.len(), 2);
        assert_eq!(case.memory[0].path, "AGENTS.md");
        assert_eq!(case.memory[0].content, "# Memory Workspace\n导航");
        assert_eq!(case.memory[1].path, "notes/work-preferences.md");
        // frontmatter 与正文间空行是原文的一部分（不按 JSONL 节丢弃）；文件尾空行裁掉
        assert_eq!(
            case.memory[1].content,
            "---\ndescription: 用户的工作偏好\n---\n\n- 不擅自提交"
        );
        assert!(case.cron.contains("\"id\":\"a1\""));
    }

    #[test]
    fn parse_memory_section_rejects_bad_paths_and_marker_collision() {
        let head = "{ \"meta\": { \"case_id\": \"t\", \"created\": \"x\", \"llm_mode\": \"debug\" } }\n{\"__section\":\"=== memory ===\"}\n";
        // index.md 不进 case（沙盒重建）
        let bad = format!("{head}{{\"__file\":\"index.md\"}}\nx");
        assert!(parse(&bad).unwrap_err().contains("不合法"), "index.md");
        // cards/ 不经此节
        let bad = format!("{head}{{\"__file\":\"cards/a.card.json\"}}\n{{}}");
        assert!(parse(&bad).unwrap_err().contains("不合法"), "cards/");
        // 保留名不能当 note 路径
        let bad = format!("{head}{{\"__file\":\"notes/AGENTS.md\"}}\nx");
        assert!(parse(&bad).unwrap_err().contains("不合法"), "reserved");
        // 原文行与标记语法撞车
        let bad = format!("{head}{{\"__file\":\"notes/a.md\"}}\n{{\"__comment\":true}}");
        assert!(parse(&bad).unwrap_err().contains("行首"), "collision");
        // 节首非空行必须是文件标记
        let bad = format!("{head}裸文本");
        assert!(parse(&bad).unwrap_err().contains("文件标记"), "bare");
    }

    // ── pre-parse 预检（docs/case-eval-system.md §checkhealth）──

    fn head(steps: &str) -> String {
        format!(
            r#"{{ "meta": {{ "case_id": "t", "created": "x", "llm_mode": "debug" }}, "steps": {steps} }}"#
        )
    }

    #[test]
    fn pre_parse_valid_case_passes() {
        let text = head(
            r#"[
            { "load": {} },
            { "observe": [{"target":"context","lines":"($tail-50,$tail]"}] },
            { "store": { "cursor": { "type": "expr", "value": "$tail" } } },
            { "observe": [{"target":"context","lines":"($cursor,$tail]"},{"target":"effects"}] },
            { "store": { "n": { "type": "int", "value": "42" }, "s": { "type": "str", "value": "$任意" } } }
        ]"#,
        );
        let case = parse(&text).unwrap();
        assert_eq!(pre_parse_check(&case), Vec::<String>::new());
    }

    #[test]
    fn observe_includes_memory_and_cron() {
        let dir = std::env::temp_dir().join(format!("overseer-case-obs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let harness = crate::Harness::load(&dir, &dir, 100_000, 0).unwrap();
        let mut ov = OverseerBackend::new(
            harness,
            crate::config::Config::default(),
            crate::llm::DebugAgent::silent(),
        );
        ov.harness
            .memory
            .write("work-preferences", "正文", "用户的工作偏好")
            .unwrap();
        ov.harness
            .cron
            .create(crate::cron::Schedule::EveryMs(60_000), "日报", 1000)
            .unwrap();
        let obs = observe(&ov);
        assert_eq!(obs.memory.len(), 1);
        assert_eq!(obs.memory[0].name, "work-preferences");
        assert_eq!(obs.memory[0].description, "用户的工作偏好");
        assert_eq!(obs.cron.len(), 1);
        assert_eq!(obs.cron[0].message, "日报");
        assert_eq!(obs.cron[0].schedule, crate::cron::Schedule::EveryMs(60_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pre_parse_accepts_memory_and_cron_targets() {
        let text = head(r#"[{ "observe": [{"target":"memory"},{"target":"cron"}] }]"#);
        let case = parse(&text).unwrap();
        assert_eq!(pre_parse_check(&case), Vec::<String>::new());
        // 值类 target 不支持 lines
        let c = parse(&head(r#"[{ "observe": [{"target":"memory","lines":"[1,2]"}] }]"#)).unwrap();
        assert!(pre_parse_check(&c)[0].contains("值类"));
    }

    #[test]
    fn pre_parse_catches_all_five_rules() {
        // ① 语法错误
        let c = parse(&head(r#"[{ "observe": [{"target":"context","lines":"[$tail"}] }]"#)).unwrap();
        assert!(pre_parse_check(&c)[0].contains("lines"), "①");
        // ② 引用未 store 的变量（在 observe lines）
        let c = parse(&head(r#"[{ "observe": [{"target":"context","lines":"($cursor,$tail]"}] }]"#)).unwrap();
        assert!(pre_parse_check(&c)[0].contains("未知变量"), "②");
        // ② store value 引用未 store 的变量
        let c = parse(&head(r#"[{ "store": { "x": { "type": "expr", "value": "$later" } } }]"#)).unwrap();
        assert!(pre_parse_check(&c)[0].contains("未知变量"), "②store");
        // ② 同 step 内变量不互相可见（统一后入册）
        let c = parse(&head(
            r#"[{ "store": { "a": { "type": "expr", "value": "$b" }, "b": { "type": "int", "value": "1" } } }]"#,
        ))
        .unwrap();
        assert!(pre_parse_check(&c)[0].contains("未知变量"), "②same-step");
        // ③ store 类型不合法
        let c = parse(&head(r#"[{ "store": { "x": { "type": "float", "value": "1.5" } } }]"#)).unwrap();
        assert!(pre_parse_check(&c)[0].contains("不合法"), "③");
        // ⑤ 未知 target
        let c = parse(&head(r#"[{ "observe": [{"target":"bogus"}] }]"#)).unwrap();
        assert!(pre_parse_check(&c)[0].contains("未知 target"), "⑤");
        // ⑤ 值类 target 带 lines
        let c = parse(&head(r#"[{ "observe": [{"target":"agents","lines":"[1,2]"}] }]"#)).unwrap();
        assert!(pre_parse_check(&c)[0].contains("值类"), "⑤lines");
    }
}
