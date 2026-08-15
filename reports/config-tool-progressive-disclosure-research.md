# edit_config 多操作模式：单一工具 + 显式 action 参数

## 问题

当前 `edit_config`（`docs/agent-loop.md`，`core/src/llm.rs`）只支持 `{path, value}`（set-by-path），仅覆盖 **写**。
Config 基础设施已有 `reflect()` 产生全量节点列表、`GET /config/schema`、`POST /config {path, value}`，
但 LLM Tool Set 缺少**查询**（取值）和**搜索**（找匹配路径）。

约束（`docs/agent-loop.md`、commit `a4511da`）：

- **不做特殊规则**：参数由 schema 完全定义，禁止隐式约定 —— 缺参不能切换模式、空值不能触发查询。
- **渐进披露，按需查**：Config 多层嵌套，LLM 通过 tool 调用-反馈逐层发现，不依赖外部 Schema 注入。

---

## 外部参考

### Anthropic 工具设计最佳实践

来源：[platform.claude.com — Define tools](https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/define-tools)（2025）

> **Consolidate related operations into fewer tools.** Rather than creating a separate tool
> for every action (`create_pr`, `review_pr`, `merge_pr`), group them into a single tool
> with an `action` parameter. Fewer, more capable tools reduce selection ambiguity and make
> your tool surface easier for Claude to navigate.

来源：[Writing tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents)（Anthropic 工程博客）

> More tools don't always lead to better outcomes. A few thoughtful tools targeting specific
> high-impact workflows is better. Too many tools or overlapping tools can distract agents.

### Anthropic Text Editor Tool（上位参考）

来源：[platform.claude.com — Text editor tool](https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/text-editor-tool)（2025）

Anthropic 的 `text_editor_20250728` 工具用显式 `command` 参数区分操作：

| command | 语义 | 特有参数 |
|---------|------|---------|
| `view` | 读文件/列目录 | `view_range`（可选） |
| `str_replace` | 精确替换 | `old_str`, `new_str` |
| `create` | 创建新文件 | `file_text` |
| `insert` | 指定行插入 | `insert_line`, `insert_text` |
| `undo_edit` | 撤销 | — |

每个 command 有独立的参数组合、无隐式模式切换。

### JSON Pointer / JSON Patch（RFC 6901 / 6902）

来源：[RFC 6901 — RFC Editor](https://www.rfc-editor.org/rfc/rfc6901)、[RFC 6902 — RFC Editor](https://www.rfc-editor.org/rfc/rfc6902)

- **JSON Pointer**：`/path/to/leaf` 标准化语法，标识嵌套 JSON 中的唯一值；`~0` = `~`，`~1` = `/`。
- **JSON Patch**：add / remove / replace / move / copy / test 六操作，操作列表原子执行。

本项目的点分路径 `.` 与 JSON Pointer `/` 语义等价（不同分隔符），dot-path 在 JSON Schema 中更自然。

### kubectl 渐进披露

来源：kubectl 命令行工具设计模式（Kubernetes 文档）

四层发现：`api-resources`（列类型）→ `describe`（实例详情）→ `explain`（字段文档）→ `get`（取出对象）。
对应我们：search（类型发现）→ query（字段粒度获取）→ update（修改）。

### etcd v3 API

来源：[etcd v3 API 设计](https://etcd.io/docs/latest/learning/api/)（CoreOS）

get / put / delete + range / prefix 前缀查询，通过 key 前缀分层组织配置。与我们的 search（按前缀/关键词匹配）功能一致。

---

## 方案对比

| 维度 | 单一工具 + action 枚举（推荐） | 三工具拆分（query/edit/search） | 隐式模式探测 | 两工具（read/write 分离） |
|------|-----------------------------|-------------------------------|------------|--------------------------|
| 工具数量 | 1 | 3 | 1 | 2 |
| 隐式语义 | 无（action 必填 enum） | 无 | 有（缺参 = query） | 无 |
| 符合 Anthropic 建议 | 是 | 否（应 consolidate） | 否 | 否 |
| 符合项目原则 | 是 | 工具面膨胀但语义清楚 | 否（违反"不做特殊规则"） | 部分（search 难以归入 get 或 edit） |
| LLM 选择负担 | 低（1 工具） | 高（6 工具总计） | 低但有歧义风险 | 中（5 工具总计） |
| 扩展性 | enum 新值 + anyOf 新分支 | 新增工具 | 新隐式规则 | 往 get 或 edit 塞 |

**推荐方案 A**：单一 `edit_config` + 必填 `action` 枚举 —— 对齐 Anthropic 文本编辑器 `command` 参数模式、
满足 "不做特殊规则"、"渐进披露"，与现有代码（`edit_config` 名称不変、`set_by_path()` 复用）兼容。

---

## 推荐设计

### 操作

| action | 语义 | 必填 | 可选 | 结果关键字段 |
|--------|------|------|------|-----------|
| `query` | 查单个 path 的值和描述 | `path` | — | `path`, `value`, `type`, `desc` |
| `update` | 修改值（验证通过写盘） | `path`, `value` | — | `ok`, `restartRequired` |
| `search` | 按关键词搜匹配的 path 列表 | `query` | `limit`（默认 20） | `matches[{path, value, desc}]` |

`query` 返回 type + desc（不光是 value），以支持渐进披露：LLM 知道字段含义后才决定是否改、怎么改。

`search` 用 `query` 参数（非 `action=query`），语义分离：action=query 查确切 path，action=search 按关键词/子串搜。

### JSON Schema（含 anyOf 条件 required）

```json
{
  "name": "edit_config",
  "description": "配置操作。action=query 查一个点分路径的值和描述；action=update 修改值（验证通过后持久化）；action=search 按关键词搜索匹配的配置项路径列表。渐进披露：先用 search 发现路径，再用 query 确认值，最后用 update 修改——不要猜测 path。",
  "input_schema": {
    "type": "object",
    "properties": {
      "action": {
        "type": "string",
        "enum": ["query", "update", "search"],
        "description": "操作类型：query=查单个path的值和描述，update=修改值，search=按关键词搜匹配路径"
      },
      "path": {
        "type": "string",
        "description": "点分配置路径。如 llm.providers.deepseek.model、kaomoji.idle.face"
      },
      "value": {
        "description": "新值（JSON）。仅 action=update。含嵌套对象时传完整值（如新增 kaomoji 条目：{\"face\":\"...\",\"motion\":\"...\"}）"
      },
      "query": {
        "type": "string",
        "description": "搜索关键词（子串匹配 path 或 desc），仅 action=search"
      },
      "limit": {
        "type": "integer",
        "description": "search 返回上限，默认 20，仅 action=search"
      }
    },
    "required": ["action"],
    "allOf": [
      {
        "if": {"properties": {"action": {"const": "query"}}},
        "then": {"required": ["path"]}
      },
      {
        "if": {"properties": {"action": {"const": "update"}}},
        "then": {"required": ["path", "value"]}
      },
      {
        "if": {"properties": {"action": {"const": "search"}}},
        "then": {"required": ["query"]}
      }
    ]
  },
  "input_examples": [
    {"action": "query", "path": "llm.active"},
    {"action": "query", "path": "kaomoji.idle.face"},
    {"action": "update", "path": "compression_reserve_default", "value": 5000},
    {
      "action": "update",
      "path": "kaomoji.celebrate",
      "value": {"face": "(≧▽≦)", "motion": "bounce"}
    },
    {"action": "search", "query": "timer"},
    {"action": "search", "query": "llm", "limit": 5}
  ]
}
```

**要点**：

- `allOf` + `if/then` 非对称约束：query 要 path、update 要 path+value、search 要 query —— 零隐式
- `input_examples` 提供每个 action 的标准调用（符合 Anthropic 文档 "for complex tools, consider using input_examples"）
- `value` 无 type 约束（open type），因不同 path 的合法值类型不同

### 后端执行伪代码

```rust
fn execute(match input.action {
    Action::Query => {
        let node = schema_nodes.find(|n| n.path == input.path)?;
        Ok(json!({ "path": node.path, "value": node.value, "type": node.ty, "desc": node.desc }))
    }
    Action::Update => {
        let mut v = serde_json::to_value(&config)?;
        reflect::set_by_path(&mut v, &input.path, input.value)?;
        let new = serde_json::from_value::<Config>(v)
            .map_err(|e| format!("验证失败: {e}"))?;
        let restart = hot_apply_diff(&config, &new);
        config = new;
        config.save(dir)?;
        Ok(json!({ "ok": true, "restartRequired": restart }))
    }
    Action::Search => {
        let matches = schema_nodes.iter()
            .filter(|n| n.path.contains(&input.query) || n.desc.contains(&input.query))
            .take(input.limit.unwrap_or(20))
            .map(|n| json!({ "path": n.path, "value": n.value, "desc": n.desc }))
            .collect();
        Ok(json!({ "matches": matches }))
    }
})
```

### 渐进披露流程

```
1. search "定时"
   → {matches: [{path:"timer_interval_ms", desc:"Timer 兜底扫描间隔", value:300000}, ...]}

2. query path="timer_interval_ms"
   → {path:"timer_interval_ms", type:{kind:"int", min:-1}, desc:"Timer 兜底扫描间隔", value:300000}

3. update path="timer_interval_ms" value=600000
   → {ok:true, restartRequired:[]}
```

### 结果形状

**query 成功**：
```json
{ "path": "compression_reserve_default", "value": 10000,
  "type": {"kind": "int", "min": 0}, "desc": "Compression 输出预留默认值" }
```

**update 成功 / 失败**：
```json
{ "ok": true, "restartRequired": [] }
{ "ok": false, "error": "验证失败: invalid type: ... at compression_reserve_default" }
```

**search 成功**：
```json
{ "matches": [
    {"path": "timer_interval_ms",  "value": 300000, "desc": "Timer 兜底扫描间隔"},
    {"path": "timer_stagger_ms",   "value": 30000,  "desc": "Timer 错峰窗口"}
] }
```

**未知 path**（query/update 共用）：
```json
{ "ok": false, "error": "未知 path: llm.providers.xxx.yyy" }
```

---

## 与现有架构衔接

| 现有组件 | 复用 |
|---------|------|
| `reflect::config_nodes()` | query 和 search 结果直接取节点列表 |
| `reflect::set_by_path()` | update 值写入复用 |
| Config serde 反序列化 | update 验证完全复用（"验证集中"不变） |
| `POST /config` 管道 | update 走同一管道 |
| `Effect::ConfigChanged` | 广播机制不变 |
| 只读降级 | `action=update` 在 `read_only` 时报错 |
| Tool Set 数量 | 4 工具不变 |
| 手工钩子 | `valid_options()` 动态 enum 校验不变 |

---

## 可选扩展（本次不下结论）

| 扩展 | 说明 | 风险 |
|------|------|------|
| `reset` action | 恢复某字段为 serde default | 语义清晰但需求不明确 |
| `list` action | 列某前缀下的所有子节点 | 可能覆盖 search 场景，冗余 |
| 批量 update | 一次改多个 path | 与 LLM 多轮 tool call 能力重复；原子性语义复杂 |

---

## 结论

**推荐**：在现有 `edit_config` 工具中增加必填 `action` enum（`query` / `update` / `search`），
通过 JSON Schema 的 `allOf` + `if/then` 对每个 action 做非对称必填约束。

**三条理由**：

1. **对齐上位参考**：Anthropic 官方 text_editor 工具的 `command` 参数模式
   （view/str_replace/create/insert/undo_edit）是最权威的实现参考；
   文档明确建议 "consolidate related operations into fewer tools with an action parameter"
   （[define-tools](https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/define-tools)，
   [Writing tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents)）

2. **满足项目约束**：`action` 必填 enum 彻底消除隐式语义空间 —— "不做特殊规则" 原则完全成立；
   search → query → update 三步实现 "渐进披露" —— LLM 每步得到反馈，不猜测 path。

3. **最小增量**：不拆工具、不改名称、值写入完全复用 `set_by_path()` + serde 管道、零额外验证代码。
   与 etcd 的 prefix 查询、kubectl 的渐进发现、JSON Pointer 的路径访问等模式一致。

**参考来源**：
- Anthropic "Define tools" <https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/define-tools>
- Anthropic "Text editor tool" <https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/text-editor-tool>
- Anthropic Engineering "Writing tools for agents" <https://www.anthropic.com/engineering/writing-tools-for-agents>
- JSON Pointer (RFC 6901) <https://www.rfc-editor.org/rfc/rfc6901>
- JSON Patch (RFC 6902) <https://www.rfc-editor.org/rfc/rfc6902>
- etcd v3 API <https://etcd.io/docs/latest/learning/api/>
