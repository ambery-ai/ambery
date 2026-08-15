# Filter 设计

> 概念定义见 concepts.md §11。本文档定策略规则与结构理解数据类型。
> **结构理解**：规则取自 3 个真实 Claude Code 终端的 UIA 样本（UIA 返回渲染后纯文本，无 ANSI 码）。

## 结构理解数据类型

filter 每次处理终端文本时**按字段填一份新数据**；filter 看不到/判不准的字段留 `None`（不硬填）。

```rust
pub struct TerminalDigest {
    pub blocks: Vec<ContentBlock>,
}

pub enum ContentBlock {
    /// 用户输入（❯ 头，可跨折行）
    UserPrompt { text: String },
    /// 助手正文（● 头，续行缩进无 glyph，可跨多行）
    AssistantText { text: String },
    /// tool 调用：● Tool(args) 头 + ⎿ 结果 + 展开体全文 + 原文自带折叠标记
    ToolCall {
        head: String,               // ● Write(path) 整头行
        result: Option<String>,     // ⎿ 后的结果摘要（可能没有）
        body: String,               // 展开体全文（diff/编号内容行）——不做任何省略
        truncated: bool,            // 原文自带折叠尾（… +N lines，源已丢信息，仅标记）
    },
    /// 会话压缩摘要（※ recap: 头，多行）
    Recap { text: String },
    /// 无 ● 头的系统注入行（⎿ 3 skills available）
    SystemInject { text: String },
    /// 未能归类但非噪声（兜底，不丢信息）
    Info { text: String },
}
```

注意：**diff 不单列块型**——它是 ToolCall 展开体的内容（Write/Edit 的展示形态），全量保留在 `body`。样本依据：win0 的 `594 +| ...` 行全部挂在 Write 工具下。

## 处理管线（按序）

1. **R0 trim_end**：去 UIA 网格右填充
2. **折行合并**：物理行 → 逻辑行。终端宽度把长行硬折成多行，续行带对齐空格/diff 前缀列（win0 的 markdown 表折得七零八落）。切分规则：
   - 新逻辑行：glyph 头（`❯ ● ⎿ ※ ✻`）、diff 行（`^\s*\d+\s*[+-]|`）、Write 展开的编号内容行（`^\s{4,}\d+ `）
   - 其余非空行 = 续行候选：仅当上一物理行宽 ≥ 90% 网格宽才判硬折并拼接进当前逻辑行（硬折无空格）；短行保留为独立逻辑行（不盲拼）
3. **去噪**（行级，规则见策略文件）
4. **块切分 + 填 TerminalDigest**：glyph 头开新块，缩进续行归当前块
5. **render**：digest → 归一文本——**全量保留，不做任何省略**（设计决定：限量问题不解决；折叠 = 对 LLM 省略内容，同样不做）。原文自带折叠标记（`… +N lines`）如实保留在 body
6. **detect_change**：作用于 render 文本，行集 Jaccard（≥0.8 Minor / 否则 Substantive）。滚动误报接受（设计决定）

## 策略文件（concepts §11 可替换策略）

```
core/src/filter/
  mod.rs      # trait + TerminalDigest/ContentBlock + render + detect_change 默认实现 + by_name
  claude.rs   # Claude Code 规则
  opencode.rs # OpenCode 规则（glyph 表）
```

trait：

```rust
pub trait Filter {
    /// 去噪 + 归一文本（策略必须实现）
    fn apply(&self, raw: &str) -> String;
    /// 去噪 + 折行合并 + 块切分 + 填 TerminalDigest（默认实现 = apply + 整篇 Info）
    fn digest(&self, raw: &str) -> TerminalDigest;
    /// 变化检测（作用于 render 文本，默认实现可共享）
    fn detect_change(&self, prev: &str, next: &str) -> Change;
}
```

Filter 唯一按实例的 hook `kind` 选择（docs/hook.md §Payload）；当前支持 `"claude"` / `"opencode"`；缺失或不受支持的 kind 在实例状态更新、读 Terminal Content、Filter 与 Queue 之前直接拒绝。

## claude.rs 噪音清单（来自真实样本）

| 噪音 | 样例 | 为什么会变 |
|---|---|---|
| 行尾右填充 | UIA 按终端网格整行返回 | 宽度相关，无语义 |
| spinner/耗时行 | `✻ Crunched for 22s`、`Thought for 7s, ran 1 shell command`、braille 字符行 | 每帧都变 |
| **计划任务行** | `✻ Running scheduled task (Jul 26 8:30pm)` | 无 `for Xs` 后缀，与耗时行不同形 |
| 底部分隔线 | `─────── npc-prof ──` | 宽度相关 |
| 空 prompt | `❯`（无文字） | 恒在 |
| 模型/费用行 | `deepseek-v4-pro  $12.34` | 费用累积会变 |
| git 状态行 | `●● on  master`（整行 + 后缀变体） | 工作区变化 |
| 权限提示行 | `⏵⏵ bypass permissions on (shift+tab to cycle)` | 模式切换 |
| token 提示行 | `/clear to save 255.1k tokens` | token 数每轮变 |

**`※ recap:` 不是噪音**——是 Recap 块型（会话压缩摘要，有信息量）。

## 应用点

三个内容入口统一为 `digest() → render()`：

| 调用点 | 链路 |
|---|---|
| Hook（session_start/stop） | 原文存档 terminal-content.jsonl → digest → render 存 Context + 注入 Queue |
| Timer 扫描 | 原文存档 → digest → render → detect_change → Substantive 才注入 Queue |
| `fetch_terminal` tool | 原文存档 → digest → render（全量）返回 LLM |

digest 本体不落盘（可从原文重建，视图易失，docs/storage.md 哲学）；Context 存 render 文本，日志格式不变。

## 测试夹具

`core/tests/fixtures/`：`processing.txt` / `idle.txt` 为**合成**样本（按真实噪音模式构造，不含真实用户数据）——真实采集样本含工作内容，不入库。合成夹具覆盖：折行合并、recap 块、scheduled task、ToolCall 折叠渲染。
