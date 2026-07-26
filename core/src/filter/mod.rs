//! Filter（concepts §11，docs/filter.md）：终端文本结构理解。
//! 策略文件：claude.rs（Claude Code）；opencode.rs（骨架，待真实样本）。

pub mod claude;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Change {
    Unchanged,
    /// 相似度（spinner 残留、计数器微变级别）
    Minor(f64),
    /// 相似度（实质内容变化，值得通知）
    Substantive(f64),
}

/// 结构理解产物（docs/filter.md）：filter 每次处理终端文本按字段填一份新数据；
/// filter 看不到/判不准的字段留 None（不硬填）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TerminalDigest {
    pub blocks: Vec<ContentBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ContentBlock {
    /// 用户输入（❯ 头，可跨折行）
    UserPrompt { text: String },
    /// 助手正文（● 头，续行缩进无 glyph）
    AssistantText { text: String },
    /// tool 调用：● Tool(args) 头 + ⎿ 结果 + 展开体 + 内置折叠尾
    ToolCall {
        head: String,
        result: Option<String>,
        body_lines: usize,
        truncated: bool,
    },
    /// 会话压缩摘要（※ recap: 头）
    Recap { text: String },
    /// 无 ● 头的系统注入行（⎿ 3 skills available）
    SystemInject { text: String },
    /// 未能归类但非噪声（兜底，不丢信息）
    Info { text: String },
}

impl TerminalDigest {
    /// 归一文本：ToolCall 折叠（head + ⎿ result + 行数），其余块正文原样；
    /// 不做截断（设计决定：限量问题不解决）
    pub fn render(&self) -> String {
        self.blocks
            .iter()
            .map(|b| match b {
                ContentBlock::UserPrompt { text } => format!("❯ {text}"),
                ContentBlock::AssistantText { text } => format!("● {text}"),
                ContentBlock::ToolCall {
                    head,
                    result,
                    body_lines,
                    truncated,
                } => {
                    let mut s = head.clone();
                    if let Some(r) = result {
                        s.push_str(&format!("\n  ⎿  {r}"));
                    }
                    if *body_lines > 0 || *truncated {
                        s.push_str(&format!(
                            "\n  … ({body_lines} 行{})",
                            if *truncated { ", 原文有折叠" } else { "" }
                        ));
                    }
                    s
                }
                ContentBlock::Recap { text } => format!("※ {text}"),
                ContentBlock::SystemInject { text } => format!("  ⎿  {text}"),
                ContentBlock::Info { text } => text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub trait Filter {
    /// 去噪 + 归一
    fn apply(&self, raw: &str) -> String;
    /// 结构理解（默认实现：无块切分，整篇 Info——策略可只实现 apply）
    fn digest(&self, raw: &str) -> TerminalDigest {
        let text = self.apply(raw);
        TerminalDigest {
            blocks: if text.is_empty() {
                vec![]
            } else {
                vec![ContentBlock::Info { text }]
            },
        }
    }
    /// 变化检测（作用于归一后文本）
    fn detect_change(&self, prev: &str, next: &str) -> Change;
}

/// 按 Config.filter_strategy 选择实现（concepts §11 可替换策略）；
/// "default" 兼容映射 claude
pub fn by_name(name: &str) -> Box<dyn Filter + Send> {
    match name {
        "claude" | "default" => Box::new(claude::ClaudeFilter::default()),
        _ => Box::new(claude::ClaudeFilter::default()), // 未知策略回退 claude
    }
}
