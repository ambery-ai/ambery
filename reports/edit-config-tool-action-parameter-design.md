# edit_config 多操作模式设计：单一工具 + 显式 action 参数

## 1. 问题

当前 `edit_config`（`docs/agent-loop.md` §Tool Set、`core/src/llm.rs` `tool_set()`）只支持 `{path, value}`（set-by-path），功能 = 写。Config 领域已有完整基础设施：

- `reflect()` → 点分路径节点列表（path/type/desc/value），覆盖全量字段
- `GET /config/schema` → schema + 当前值
- `POST /config {path, value}` → 验证 → 持久化 → 广播

但 LLM Tool Set 只暴露了 `update`，缺少**查询**（"取值看看"）和**搜索**（"找包含某关键词的 path"）。

设计约束来自 `docs/agent-loop.md` §Tool Set 原则：

> **不做特殊规则，用语义明确行为**——tool 参数由 schema 完全定义，禁止隐式约定（缺参切换模式、空值触发查询等）。
>
> **渐进披露，按需查**——Config 多层嵌套，LLM 通过 tool 调用-反馈逐层发现 path 和类型，不依赖外部 Schema 注入。

即：不能通过省略 `value` 隐式触发查询、不能通过传空字符串触发搜索、不能有任何"猜测"语义。

---

## 2. 外部参考

### 2.1 Anthropic 工具设计最佳实践

来源：`platform.claude.com/docs/en/docs/agents-and-tools/tool-use/define-tools`

> **Consolidate related operations into fewer tools.** Rather than creating a separate tool for every action (`create_pr`, `review_pr`, `merge_pr`), group them into a single tool with an `action` parameter. Fewer, more capable tools reduce selection ambiguity and make your tool surface easier for Claude to navigate.

来源：[Writing tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents)（Anthropic 工程博客）：

> More tools don't always lead to better outcomes. A few thoughtful tools targeting specific high-impact workflows is better. Too many tools or overlapping tools can distract agents from pursuing efficient strategies.

### 2.2 Anthropic Text Editor Tool（权威上位参考）

Anthropic 发布的 `text_editor_20250728` 工具使用**显式 `command` 参数**区分操作：

| command | 语义 | 特有参数 |
|---------|------|---------|
| `view` | 读文件/列目录 | `view_range`（可选） |
| `str_replace` | 精确替换 | `old_str`, `new_str` |
| `create` | 创建新文件 | `file_text` |
| `insert` | 指定行插入 | `insert_line`, `insert_text` |
| `undo_edit` | 撤销 | — |

**关键设计特征**：

- `command` 是**必填 enum**，每个 command 有独立的 required 参数子集
- Schema 通过 `anyOf`/`oneOf` 声明每个分支的必填字段与参数组合
- 不会通过"某参数缺失"来推断操作——操作由 `command` 明确指定
- 与我们"不做特殊规则"原则完全一致

### 2.3 其他系统参考

| 系统 | 模式 | 与我们相关的点 |
|------|------|--------------|
| **etcd v3 API** | get/put/delete + range + prefix | 前缀查询 = 分层穿透；我们的 search 类似 |
| **JSON Pointer (RFC 6901)** | `/path/to/leaf` 标准化路径 | 本项目用点分 `.`，语义等价 |
| **JSON Patch (RFC 6902)** | add/remove/replace/move/copy/test | 操作列表原子性；我们是单次单操作（LLM 逐轮），非批量 |
| **kubectl** | get/describe/explain/api-resources | 渐进披露：粗发现 → 细详情；search → query → update 对应 |

---

## 3. 方案对比

### 方案 A：单一 `edit_config` + 显式 `action` 枚举（推荐）

一个工具，通过必填 `action` 参数区分操作。

**优点**：

- 符合 Anthropic 官方最佳实践（"consolidate related operations into fewer tools"）
- 与 Text Editor Tool 的 command 模式完全对齐（权威上位参考）
- 完全消除隐式语义——操作由 `action` 明文表达，零歧义
- 现有代码只有一个 `edit_config` 工具名，不改变工具数量
- 可渐进扩展（新增 action = 新增 enum 值 + 新增 anyOf 分支）
- 降低 LLM 选择负担（4 工具 vs 6-7 工具）

**缺点**：

- 单工具 schema 更复杂（`anyOf`/`oneOf` 分支需要仔细设计）
- `search` 和 `query` 的语义边界需要文档明确

### 方案 B：三工具拆分（`query_config` + `edit_config` + `search_config`）

**优点**：

- 每工具 schema 更简单
- 职责一眼分明

**缺点**：

- 违反 Anthropic "consolidate" 建议
- 工具数量从 4→6，增加 LLM 选择认知负担
- "我要查/改配置"实质是一个意图域，不应因只读/读写而拆分
- `search_config` 与 `query_config` 容易混淆触发

### 方案 C：隐式模式探测（当前隐含行为，被原则禁止）

`{path}` = query，`{path, value}` = update，`{pattern}` = search——缺参决定模式。

**缺点**：

- 直接违反"不做特殊规则，禁止隐式约定"原则
- LLM 可能无意省略 value 却触发了 query（破坏性操作变成无害读取还好，反过来更危险）
- 无法区分"query intent" 和 "LLM 忘了传 value"

### 方案 D：两工具（read/write 分离）

`get_config`（query + search）+ `edit_config`（update）。

**优点**：

- 读/写语意最安全

**缺点**：

- search 塞进 get 有强制语义耦合
- 仍违反 "consolidate" 建议
- 与现有 `edit_config` 命名冲突，需要重命名

---

## 4. 推荐方案：方案 A 详细设计

### 4.1 操作定义

| action | 语义 | 必填参数 | 可选参数 | result 主字段 |
|--------|------|---------|---------|-------------|
| `query` | 查一个 path 的值 | `path` | — | `path`, `value`, `type`, `desc` |
| `update` | 改一个 path 的值 | `path`, `value` | — | `ok`, `restartRequired` |
| `search` | 模糊搜索匹配的 path 列表 | `query` | `limit`（默认 20） | `matches[{path, value, desc}]` |

`query` 不仅返回 value，还返回 type 和 desc——供 LLM 理解该字段语义后决定是否更新、该如何更新（渐进披露）。

`search` 用 `query` 而非 `pattern` 命名，与 `"query"` action 作概念区分：action=query 查确切 path，action=search 用关键词/子串搜索。

### 4.2 Tool 定义（JSON Schema，含 anyOf 分支）

```json
{
  "name": "edit_config",
  "description":
    "配置操作。action=query 查一个点分路径的值和描述；action=update 修改值（验证通过后持久化，非法值拒绝）；action=search 按关键词搜索匹配的配置项路径列表。渐进披露：先用 search 发现路径，再用 query 确认值，最后用 update 修改——不要猜测 path。",
  "input_schema": {
    "type": "object",
    "properties": {
      "action": {
        "type": "string",
        "enum": ["query", "update", "search"],
        "description": "操作类型：query=查单个path的值，update=修改值，search=按关键词找匹配路径"
      },
      "path": {
        "type": "string",
        "description": "点分配置路径，如 llm.providers.deepseek.model 或 kaomoji.idle.face"
      },
      "value": {
        "description": "新值（JSON），仅 action=update 时需要。字符串/数字/布尔/对象均可。嵌套 map 条目需传完整对象（如新增 kaomoji 状态传 {\"face\":\"...\",\"motion\":\"...\"}）"
      },
      "query": {
        "type": "string",
        "description": "搜索关键词（子串匹配路径或描述），仅 action=search 时需要"
      },
      "limit": {
        "type": "integer",
        "description": "search 返回上限，默认 20，仅 action=search 时有效"
      }
    },
    "required": ["action"],
    "allOf": [
      {
        "if": { "properties": { "action": { "const": "query" } } },
        "then": { "required": ["path"] }
      },
      {
        "if": { "properties": { "action": { "const": "update" } } },
        "then": { "required": ["path", "value"] }
      },
      {
        "if": { "properties": { "action": { "const": "search" } } },
        "then": { "required": ["query"] }
      }
    ]
  },
  "input_examples": [
    {"action": "query", "path": "llm.active"},
    {"action": "query", "path": "kaomoji.idle.face"},
    {"action": "update", "path": "compression_reserve_default", "value": 5000},
    {"action": "update", "path": "kaomoji.celebrate", "value": {"face": "(≧▽≦)", "motion": "bounce"}},
    {"action": "search", "query": "timer"},
    {"action": "search", "query": "llm", "limit": 5}
  ]
}
```

**设计要点**：

- `allOf` + `if/then` 约束非对称 required：query 要 path、update 要 path+value、search 要 query——不依赖隐式推断
- `input_examples` 提供每个 action 的标准用法（Anthropic 建议复杂工具加此字段）
- `value` 的类型为 open（无 type 约束），因为不同 path 的值形态不同（int/string/object/boolean/float）
- Schema 由 tool schema 定义 → LLM 遵守 → 后端反序列化回 Config 验证（现有 serde 管道复用）

### 4.3 后端执行逻辑（伪代码）

```rust
fn execute_edit_config(input: EditConfigInput, config: &mut Config, schema_nodes: &[ConfigNode]) -> Result<Value> {
    match input.action {
        Action::Query => {
            let path = input.path.ok_or("query 需要 path")?;
            let node = schema_nodes.iter().find(|n| n.path == path)
                .ok_or_else(|| format!("未知 path: {path}"))?;
            Ok(json!({
                "path": node.path,
                "value": node.value,
                "type": node.ty,   // 来自 reflect() 的 type tag
                "desc": node.desc,
            }))
        }
        Action::Update => {
            let path = input.path.ok_or("update 需要 path")?;
            let value = input.value.ok_or("update 需要 value")?;
            // 复用现有 set_by_path + serde 反序列化验证管道
            let mut v = serde_json::to_value(&*config)?;
            reflect::set_by_path(&mut v, &path, value)?;
            let new_config: Config = serde_json::from_value(v)
                .map_err(|e| format!("验证失败: {e}"))?;
            // 热应用: 识别 restart_required（行为即真相）
            let restart_required = hot_apply_diff(config, &new_config);
            *config = new_config;
            config.save(config_dir)?;
            Ok(json!({"ok": true, "restartRequired": restart_required}))
        }
        Action::Search => {
            let q = input.query.ok_or("search 需要 query")?;
            let limit = input.limit.unwrap_or(20);
            let matches: Vec<_> = schema_nodes.iter()
                .filter(|n| n.path.contains(&q) || n.desc.as_deref().unwrap_or("").contains(&q))
                .take(limit)
                .map(|n| json!({"path": n.path, "value": n.value, "desc": n.desc}))
                .collect();
            Ok(json!({"matches": matches}))
        }
    }
}
```

### 4.4 渐进披露流程

LLM 从不知道 config 结构到成功修改的典型路径：

```
1. pet: edit_config({action:"search", query:"定时"})
   → {matches: [{path:"timer_interval_ms", desc:"Timer 兜底扫描间隔", value:300000},
                {path:"timer_stagger_ms", desc:"Timer 错峰窗口", value:30000},
                {path:"timer_tick_ms",    desc:"Timer 主循环粒度", value:60000}]}

2. pet: edit_config({action:"query", path:"timer_interval_ms"})
   → {path:"timer_interval_ms", type:{kind:"int", min:-1}, desc:"Timer 兜底扫描间隔",
       value:300000}

3. pet: edit_config({action:"update", path:"timer_interval_ms", value:600000})
   → {ok:true, restartRequired:[]}  // 热生效，无需重启
```

零猜测：每一步都有明确的工具反馈，LLM 不依赖外部 schema 注入。

### 4.5 结果形状

**query 成功**：
```json
{
  "path": "compression_reserve_default",
  "value": 10000,
  "type": {"kind": "int", "min": 0},
  "desc": "Compression 输出预留默认值"
}
```

**update 成功**：
```json
{
  "ok": true,
  "restartRequired": []
}
```

**update 验证失败**：
```json
{
  "ok": false,
  "error": "验证失败: invalid type: string \"oops\", expected usize at compression_reserve_default"
}
```

**search 成功**：
```json
{
  "matches": [
    {"path": "timer_interval_ms", "value": 300000, "desc": "Timer 兜底扫描间隔"},
    {"path": "timer_stagger_ms", "value": 30000,  "desc": "Timer 错峰窗口"}
  ]
}
```

**未知 path**（query/update 共用）：
```json
{
  "ok": false,
  "error": "未知 path: llm.providers.xxx.yyy"
}
```

---

## 5. 与现有架构的衔接

| 现有组件 | 复用方式 |
|---------|---------|
| `reflect::config_nodes()` | `query` 结果取自节点列表（path/type/desc/value 全部已有） |
| `reflect::set_by_path()` | `update` 写值复用现有函数 |
| `Config` 的 serde 反序列化 | `update` 验证完全复用（"验证集中"原则不变） |
| `POST /config {path, value}` | `update` action 走同一管道 |
| 只读降级 | 保持不变：`action=update` 在 `read_only: true` 时返回 error |
| Tool Set 协议 | 四个 tool 数量不变，只扩展 `edit_config` 的 schema |
| 原则 | "不做特殊规则"——action 必填 enum，零隐式；"渐进披露"——search→query→update 三步 |
| 上报管道 | update 效果照旧经 `Effect::ConfigChanged` 广播 |

**唯一的手工钩子保持不变**：`valid_options()` 动态 enum 校验在 update 时仍然生效（如 `llm.active` 的合法值 = providers keys + "debug"）。

---

## 6. 可选扩展（本次不下结论）

| 扩展点 | 说明 | 风险 |
|--------|------|------|
| `delete` action | 删除 map 中的 key（如 kaomoji 条目） | 大多字段不可删除（结构性字段设 default 不等价于删除）；map 条目删除的语义需要额外定义 |
| `list` action | 列某一前缀下的所有子节点（如 `llm.providers` 下列出所有 provider key） | 与现有 `reflect()` 返回 flat list 的设计重复；可通过 search 部分覆盖 |
| 多 path batch update | 一次改多个 path | 当前一轮 LLM 可多次 tool call（disable_parallel_tool_use 控制），batch 不必要；且与 JSON Patch 原子性语义冲突 |
| `reset` action | 恢复某字段为 serde default | 语义清晰但需求不明确，不优先 |

---

## 7. 结论

**推荐方案 A**——在单一 `edit_config` 工具中增加必填 `action` 枚举（`query` / `update` / `search`），通过 `allOf` + `if/then` 约束每个 action 的必填参数子集。

**理由三条**：

1. **对齐上位参考**：Anthropic 官方 text_editor 工具的 `command` 参数模式是最权威的实现参考，Anthropic 工程博客也明确建议用 action 参数合并相关操作而非拆工具
2. **明确满足项目约束**：`action` 必填 enum 彻底消除"缺参切换模式"的隐式语义空间，"不做特殊规则"原则得以成立；search→query→update 三步实现"渐进披露，按需查"，LLM 每步都有反馈，不猜测
3. **最小增量改动**：不拆分工具数量（4 工具不变），不改变 `edit_config` 命名，值写入完全复用现有 `set_by_path` + serde 反序列化管道，零额外验证代码

**下一步**（如需落地）：将 `edit_config` 的 `input_schema` 从当前 `{path, value: open}` 替换为上述带 `action` + `allOf/if-then` 的完整 schema，在 `ambery.rs` 的 tool 执行分支按 `action` 分派。
