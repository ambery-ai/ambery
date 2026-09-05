# Config

[English](config.md) | 中文

Config 域（concepts §7，与 Storage 域并列）由 `config.json` 与 `AGENTS.md` 组成，落盘目录由 `core/src/paths.rs` 的 `config_root()` 决定。

## 核心模型

```text
config.json（历史 payload）
  → version 分派 / migration
  → null 归一 + 递归 reconcile
  → Config（运行时真值）
  ├─ 本地 schema 投影 → CLI / 设置面板
  └─ LLM 受限投影 → edit_config
```

**`Config` 类型是当前配置结构的单一事实源。** 字段的类型、说明、默认值、迁移规则、以及面向消费者的访问元数据都应与字段定义共位。serde 负责持久化，schemars 负责类型与 UI schema，项目 `config` metadata 负责版本演进和消费者访问范围。

文件级 `version` 不进入 `Config`：加载管线读取它，保存时注入它。它是单调整数，未带该字段的历史文件为 v0。

### Config path grammar

所有可寻址 Config path segment 统一采用小写 ASCII grammar：`^[a-z][a-z0-9_-]*$`。它保证点分 path 无歧义、跨入口稳定；不允许大写变体、Unicode 近形字符、`.`、空白或 emoji。`face` 等 value 可以保留任意 Unicode，这条规则只约束 path key。

- 静态 `Config` / object 字段名（含 serde rename 后的序列化名）由 `ConfigMeta` derive 在**编译期**检查；
- map 的动态 key 在加载、迁移/reconcile 后、以及每次 update 时由统一 validation 在**运行时**检查；
- map default 产生的 key 同样须通过运行时检查；
- 动态 key 校验与其他 validation 走同一失败语义：update 聚合错误后原子拒绝；加载聚合错误、写入加载报告且不阻断启动。动态 entry 的 key 无法 default 化时，修复结果为该 entry 不存在；其余 map entry 保留。
- `version` 是文件级控制字段，不属于 Config descriptor tree，不适用本 grammar。

```text
config_root/                   # Windows: %USERPROFILE%\.config\ambery\
  config.json                  # Config 持久化（Config::save，pretty JSON；保存时注入 version）
  config.bak/config-v0NN.json  # 版本替换前的对称备份
  AGENTS.md                    # 身份提示词；Harness::load 缺失时 bootstrap 默认
  storage/                     # Storage 域，见 docs/storage.md
```

API key 本体只在环境中（应用级 env 文件或进程环境——见 docs/llm-setup.md §key 存储模型），config 仅保存变量名；key 永不写入 config.json。

### LLM 组

provider profile 的 `base_url`、`model`、`api_key_env`、`temperature`、`context_window`、`compression_reserve` 都是 profile 级字段。`context_window` 是模型窗口的事实，不是全局策略；Compression 触发点为 `context_window − reserve`，其中 provider 未设 `compression_reserve` 时使用全局 `compression_reserve_default`。无 `context_window` 即不压缩；唯一生效入口为 `effective_compression_limit()`，计量使用 usage token 真值而非 chars/4 换算。

**默认预设 `api_key_env` 值遵循 `AMBERY_<NAME>_API_KEY` 约定**（`AMBERY_DEEPSEEK_API_KEY` / `AMBERY_MOONSHOT_API_KEY` / `AMBERY_ZHIPU_API_KEY` / `AMBERY_OPENAI_API_KEY`；ollama 无——本地端点，无需 key）。`api_key_env` 按 provider 可编辑（私有 provider 可指向自己的变量）。

**`llm.active` 有显式未配置值**，与 `debug`、provider keys 并列的合法选项（动态 enum：`["unconfigured", "debug", ...provider keys]`）。它是全新安装的默认值：运行时将其视为"无 LLM"（docs/llm-setup.md §后端变更）。

LLM tool 不可访问 `llm` 子树（见 [反射与消费者投影](#反射与消费者投影)），不改变上述本地运行时语义。

### 表情池

表情领域是一个固定 object，两个池是它的固定字段；只有池内表情名称才是动态 map key：

```rust
struct Config {
    #[config(validate = [Func(validate_kaomoji_pools)])]
    pub kaomoji: KaomojiConfig,
}

struct KaomojiConfig {
    pub system: HashMap<String, KaomojiEntry>,
    pub user: HashMap<String, KaomojiEntry>,
}
```

| 路径 | 归属与权限 | 用途 |
|---|---|---|
| `kaomoji.system` | 系统池；agent 可见、可整体更新 | 系统状态表情；窗口尺寸扫描只扫描此池；默认不要修改 |
| `kaomoji.user` | 用户池；agent 可见、可整体更新 | 用户自定义表情；初始可为空 |

agent 创建、修改或移除任一池中的表情时，不使用 `add` / `delete` action，而是先对目标池 `query(path="…", view="object")` 读取完整当前 map，再以 `update` 写回完整的新 map。`kaomoji.system` 的节点 `desc` 必须明确提示“默认不要修改”；两池的区别只在表情归属与尺寸扫描来源，不在 LLM 访问权限。

`validate_kaomoji_pools` 校验最终表情领域的两个不变量：

```text
keys(system) ∩ keys(user) = ∅
{ idle, processing, notify } ⊆ keys(system) ∪ keys(user)
```

两池 key 全局唯一，所以不存在 user 覆盖 system 的隐式优先级。用户可通过本地设置面板把表情在两池间原子移动；移动后的完整 Config 必须通过统一 validation。基础状态可以在两池间移动，仍参与默认状态和 `set_autonomy(key)` 的按 key 解析；系统池的额外职责仅是尺寸扫描来源。

## 字段 metadata

目标语法使用同一个项目属性承载字段语义；具体 derive 实现负责让 `#[config(...)]` 成为合法 attribute，并生成供 loader / reflect / tool 共用的 descriptor tree。

```rust
#[derive(Serialize, Deserialize, JsonSchema, ConfigMeta)]
#[config(migrate = [
    3..=3 => Func(migrate_config_v3),
])]
pub struct Config {
    #[serde(default = "default_timer_batch")]
    #[config(migrate = [
        0..=2 => Default,
    ])]
    pub timer_batch: usize,

    #[config(no_llm_visible)]
    pub llm: LlmConfig,
}
```

`no_llm_visible` 的语义是：该节点及整棵子树不进入 LLM 的 Config 投影。因此 `edit_config` 无法 grep、query 或 update 它；本地 CLI、设置面板、持久化、启动时选择 LLM backend 均不受影响。表情两池不标记 `no_llm_visible`，均对 agent 可见、可整体更新。

它不是 `serde(skip)`：后者会让字段不再属于持久化 Config，也会从所有 schema 消费者消失。

### Validation enum

validation 校验的是**最终当前 Config 是否允许生效**，与 migration 的“历史值如何变成当前值”不同。所有 validator 在 null 归一、reconcile 与 serde 结构验证之后运行。

```rust
enum Validation {
    Range { min: ..., max: ... }, // 固定数值边界
    OneOf([...]),                  // 固定候选集合
    Func(ValidationFn),            // 动态候选或跨字段不变量
}

type ValidationFn = fn(&Value) -> Vec<String>;
```

`Range` 与 `OneOf` 只表达静态规则；动态合法值必须用 `Func`，例如 `llm.active` 是否属于当前 provider keys。`Range` 只允许挂在静态已知的数值节点，`ConfigMeta` derive 必须在编译期拒绝将其挂在 string、bool、object、map 或其他非数值节点。`Range` 使用闭区间，`min` / `max` 都可选：`Range { min: Some(0), max: Some(1) }` 表示 `0 ≤ value ≤ 1`，只给 `min` 表示下界，只给 `max` 表示上界。`OneOf` 按严格 JSON 值相等比较，不做大小写折叠、trim、数值字符串转换或其他正规化。`Func` 接收它所挂节点的最终值，只返回 message，不携带 path；框架补上挂载节点的完整 path。它可以返回多条 message。

父子 validator 可以组合执行：子节点 validator 校验自身或自身子树，父节点 validator 校验最终子树。每次 update 只运行目标节点子树的 validators 与其祖先 validators，执行顺序为子树→祖先；加载没有单一目标，因此运行全部 validators。错误按 validator 挂载节点的完整 path 字典序、同 path 按 message 字典序稳定聚合。

`Range` 与 `OneOf` 是 descriptor / reflect 输出的一部分，供 CLI 与设置面板机械选择控件；它们只是同一 validation 真值的只读投影，后端仍是唯一放行者。`Func` 不输出函数或伪静态约束，提交后由统一 validation 返回其 message。

update 的任一 validator 失败会拒绝整次更新，不改变内存或磁盘 Config。加载时 validator 失败不阻断启动：同一节点的全部 message 一次写入加载报告，该节点只 default 化一次，然后继续处理其余节点。

## 版本与 migration

加载流程：

```text
读原始 JSON payload + version
  ├─ version == current：null 归一 → 递归 reconcile
  ├─ version <  current：migration → null 归一 → 递归 reconcile → 备份 → 写回 current
  └─ version >  current：备份新版现场 → 找可用旧备份只读加载；无备份则拒绝启动
```

migration 是**历史 JSON 到当前 JSON 的纯、确定性、可重放映射**；不得读取环境、网络、文件或其他运行时状态。函数不接收 `version`：适用源版本已由它所属的 range 明确表达。

### Migration enum

一个节点可声明一张包含多个、不重叠 range 的表：

```rust
enum Migration {
    Default,
    Rename { from: Path },
    Func(MigrationFn),
    RenameWithFunc { from: Path, func: MigrationFn },
    // 未命中显式 range = 隐式 Current
}

type MigrationFn = fn(Value) -> Result<Value, ConfigMigrationError>;
```

| 规则 | 输入来源 | 结果 |
|---|---|---|
| `Default` | 无 | 当前节点的 default |
| `Rename` | `from` 指定的完整旧点分 path | 原样成为当前节点值 |
| `Func` | 当前节点在旧 JSON 中的同路径子树 | `func(old)` 的结果 |
| `RenameWithFunc` | `from` 指定的完整旧点分 path | `func(old)` 的结果 |
| 隐式 `Current` | 当前节点在旧 JSON 中的同路径子树 | 原样保留，随后 reconcile |

`Rename` 的 `from` 一律是**完整旧点分路径**，所以改名和跨 object 移动都没有“相对哪个父节点”的隐式约定。

```rust
#[config(migrate = [
    0..=2 => Default,
    3..=4 => Rename { from: "legacy.timeout_ms" },
    5..=5 => RenameWithFunc {
        from: "legacy.timeout_seconds",
        func: seconds_to_ms,
    },
    6..=7 => Func(normalize_timeout_v6),
    // v8 起未命中：隐式 Current
])]
pub timeout_ms: u64;
```

### 历史覆盖与父子关系

未命中规则并不表示“猜测”：它有唯一、明确的 `Current` 语义。但在当前 path 还未存在的历史版本，不能落入隐式 `Current`；该段历史必须由节点自身或某个祖先的 `Default`、`Rename`、`Func` 或 `RenameWithFunc` 显式处理。

父子节点可以都声明 migration metadata；关键约束是**显式 range 不得相交**。检查覆盖所有 enum variant，而不是只检查函数：

1. 同一节点表内的 range 不得重叠；
2. 任一祖先与后代节点的显式 range 不得相交；
3. `Config` 根也适用同一规则；
4. 字段“当前 path 在何时开始存在”是历史事实，不能由编译器从当前类型推断；每个已发布版本必须由 migration fixture 验证。

经典例子（当前为 v11）：

```rust
#[config(migrate = [
    3..=3 => Func(migrate_config_v3),
])]
struct Config {
    root: Root,
}

struct Root {
    #[config(migrate = [
        4..=7 => Default,
        // v8..=10 未命中：隐式 Current
    ])]
    leaf: Type,
}
```

| 源版本 | 生效规则 | 含义 |
|---|---|---|
| v3 | 根 `Func(migrate_config_v3)` | 根函数拥有 v3 的完整 Config 迁移 |
| v4–v7 | `root.leaf` 的 `Default` | `leaf` 在这些版本没有可保留的语义 |
| v8–v10 | `leaf` 的隐式 `Current` | 读取同路径历史值 |

该例说明：父子 metadata 可以共存；只有相同源版本的显式规则才冲突，因此无需规定父函数与子函数的执行顺序。

复杂变换通过把 `Func` 挂在足够高的节点处理：单字段挂叶子；多个兄弟字段挂其最近共同父 object；跨顶层字段挂 `Config` 根。根函数接收完整的 Config payload（不包括文件级 `version` 控制字段）。

### 失败与删除

**任意 migration 失败都统一 default 化该节点、逐项上报并继续加载。** 这包括 `Rename` 找不到输入、`Rename` 输入类型不合法、`Func` / `RenameWithFunc` 返回 `Err` 等。上报至少含目标 path、源 version 与原因。

`Delete` 不属于 `Migration` enum：被删除的旧字段已不在当前 `Config` 中，无处挂 metadata。所有映射完成后，递归 reconcile 剔除当前 schema 中不存在的旧 path 并逐项上报；`Rename` / `RenameWithFunc` 读取过的旧路径同样在此阶段清除。

## Default、null 与递归 reconcile

### Default 来源

default 是配置语义，不是根据 Rust 类型随意猜出的技术零值。

| 节点类型 | default 规则 |
|---|---|
| 叶子 | **必须**声明自身的语义 default |
| 静态 object | 可不声明；缺失或失效时递归构造其静态 child 的 defaults |
| map 字段 | **必须**声明自身的语义 default |

因此静态 object 不需要重复维护一份 object default；其默认形态由静态子节点组装。叶子不能继续向下构造，必须有 default。map 的 key 动态，map 整体缺失时也没有已知 child key 可向下构造，所以 map 自身必须定义 default。

map default 可以是空 map，也可以是系统预设；由字段产品语义决定，不能以 `HashMap::default()` 自动代替。例如 `kaomoji.system` 整体缺失可恢复系统表情映射，`providers` 整体缺失可恢复公开预设或空 map——这是字段自己的声明，不是 map 类型的特殊规则。

### `null = 缺失`

Config 对 JSON object 统一规定：

> `"key": null` 与该 key 不存在完全等价。

进入 migration / reconcile 前，递归移除所有 object 中值为 `null` 的 key；数组中的 `null` 不适用这条规则。工具写入与保存也遵守同一归一化，不持久化“显式 null”这种第二语义。如需区分显式值与缺失，必须用显式 enum 建模。

### Reconcile 逻辑链

```text
object 中 key:null → 移除 key（视为缺失）

静态 object 缺失 / 类型错误
  → 向下递归，组装每个静态 child 的结果

叶子缺失 / 类型错误
  → 该叶子的 default

map 字段缺失 / 类型错误
  → 该 map 字段自己的 default

map 已存在
  → 不把 map default 的 key merge 回来
  → 遍历现有且非 null 的 entry
  → 对 entry 的固定 value schema 递归 reconcile

未知 path
  → 递归剔除并逐项上报
```

动态的是 map 的 **key**，不是 entry 内的字段。

因此：

- map 节点 `providers` 缺失时，用 `providers` 自己的 default；
- 已存在 map 中的某个 key 缺失或为 null 时，该 key 就不存在，不创建它；
- map 已存在时，不自动把 default map 中未出现的 key 合并回来；
- 已存在 key 的 value 类型错误时，该 value object 按其固定 child defaults 直接 default 化；
- 某个 child 的病灶只回退该 child，不能因为它回退整个父 object。

## 反射与消费者投影

完整 descriptor tree 必须为**所有容器**保留可定位节点：静态 object、map，以及已有 map entry 的固定 object value 都在树中。因此 `query(path, view?)` 可以统一查询任一可见容器或叶子；本地完整 tree 例如有 `kaomoji`、`kaomoji.system.idle`、`kaomoji.system.idle.face`，而 LLM 受限投影只暴露可访问的 `kaomoji.user` 子树。本地 UI 是否把 object 本体渲染为独立节点，是另一个扁平呈现选择，不改变 descriptor tree。

`reflect()` 负责把 `Config` 的类型、doc comment、约束与当前值投影为节点；本地 CLI 和设置面板消费完整投影。例如本地完整投影可包含：

```json
{ "path": "llm.active", "type": "enum", "options": ["unconfigured", "debug", "deepseek"],
  "value": "deepseek", "desc": "当前 LLM" }
```

约定：doc comment → `desc`，`#[schemars(range(...))]` → min/max，serde default → 初始值；类型到控件的机械映射为 bool→开关、enum→单选组、int+range→滑块、int→数字框、string→文本框、map→键值对列表、嵌套 object→按 path 前缀分组。

**两个手工钩子**仍是唯一的非自动点：

1. **动态 enum options**：类型表达不了 `llm.active` 的合法值为 `unconfigured`、`debug` 与 `providers` 的 key，由唯一 `OPTIONS` 注册表提供 `path → fn(&Config) -> Vec<String>`。
2. **热/冷语义**：由运行时 diff 如实上报；具体字段分类只在其行为文档定义，未列为热字段的项默认冷更新，写盘但保持当前运行值。热更新从下一项运行操作起生效：不改变已发出的 LLM 请求或正在执行的 tool call，后续运行操作读取新值。冷更新统一在整个应用 / backend 进程重启后生效。agent 自己 update 时，热字段 tool result 以 `msg` 说明“已生效”，冷字段以 `msg` 说明“已保存，重启应用后生效”；用户经反射 Config UI 修改时，只在对应 UI 字段显示 `restartRequired` 状态，不向 agent 主动注入事件、聊天或系统消息。agent 后续按需 query 到存在待重启变更的具体值时，query result 以 `msg` 说明“已保存，重启应用后生效”。

`no_llm_visible` 在 descriptor tree 上形成另一份**LLM 受限投影**。它不是路径硬编码：任何标记节点及其后代都从 LLM 投影移除，且对直接访问统一拒绝。

当前 `llm` 整棵子树应标记 `no_llm_visible`：包括 `llm.active` 与 `llm.providers`。本地配置仍可使用本机私有 provider；这些 endpoint、model、环境变量名和 active 选择器均不向 LLM 暴露，也不可由 LLM 修改。

## `edit_config`：单工具、显式动作、渐进披露

LLM 只有一个 `edit_config` tool。行为由必填 `action` 分支表达，禁止通过缺参、空值或失败写入切换模式：

| action | 输入 | 语义 |
|---|---|---|
| `grep` | `pattern` | 用 Rust regex 在 LLM 可见节点的 path 与中文 `desc` 中定位候选 |
| `query` | 精确 `path`，可选 `view` | 读取指定节点的值、类型、说明与一层结构 |
| `update` | 精确 `path` + JSON `value` | 走统一验证、热应用、持久化管道修改配置 |

工具描述给出所有 **LLM 可见顶层 key** 的一句中文用途，不提供 root 查询分支；当前包括：

- `kaomoji`：表情状态映射；
- `set_autonomy_default_ttl_ms`：Autonomy 默认持续时间；
- `timer`：Timer 调度子树；
- `view_scale`、`badge_style`、`badge_side`：View 外观；
- `theme`、`themes`：当前主题名与主题 token 表（docs/theme.md）。

推荐步骤只保留一句：**路径未知先 `grep`，再 `query`；优先 `view=children`，必要时才对已定位的小 object 使用 `view=object`。**

### `grep`

`grep.pattern` 是匹配 path 与中文 `desc` 的 Rust regex，采用原始默认语义：默认大小写敏感、Unicode 启用；需要忽略大小写时显式使用内联 flag（如 `(?i)timer`），不做 lower-case、分词、模糊匹配或关键词扩展。

合法 regex 无匹配时返回成功空数组 `{"ok": true, "matches": []}`；只有 regex 语法非法才返回错误。候选可命中叶子和容器，返回 `path + type + desc`，**不返回当前 value**；按完整 `path` 字典序稳定排列。得到精确路径后由 `query` 读取真值。

### `query`

`query` 始终按精确 path 查询**一个节点**，统一返回 `node + children`；不是 leaf / children / object 三个 action。

- path 是叶子：`node` 带 `path / type / desc / value`，`children: []`；
- path 是容器且省略 `view` 或传 `view=children`：返回目标 `node` 与直接 `children`；叶子 child 带当前 value，容器 child 不递归携带 value；
- path 是容器且传 `view=object`：返回目标 object / map 的完整当前 JSON；仅建议对已经定位的小对象使用，且不返回 `children`；
- `view=object` 仅适用于容器。若对叶子传入，明确报错，不静默改成另一种读取形态。

只有携带更新目标完整当前值的 `query(path)` 才会留下该精确 path 的已读快照：叶子直接 `query(path)` 即完整；object / map 必须 `query(path, view="object")`。`grep` 与容器的默认 `view=children` 都只用于发现/导航，不产生快照。每个快照关联自己的 tool result message ID（现有 `tool_call_id`），不以摘要文本推断来源。

快照是否可用于更新是一个集合覆盖判断。设 `R` 为所有完整 query 产生的快照，`C` 为当前仍保留在 Context 的 message ID 集合，`W` 为快照之后的成功写入；目标 `P` 可写当且仅当存在 `r ∈ R`：

```text
r.path = P
r.toolResultMessageId ∈ C
r 来自更早的 LLM response
不存在与 P 相交、且发生在 r 之后的成功写入
```

因此同一路径可有多个快照；较早快照因写入失效后，后一次完整 query 可重新提供有效覆盖。Context compression 不逐条猜测失效，而是取 `R ∩ C`：仍在 Context 的 query result 对应快照继续有效，未被 Context 保留集合覆盖的快照自然失效并可清理。一次成功写入只污染相交路径的快照集合，不影响无关路径。

`update(path, value)` 必须存在上述有效完整快照；否则原子拒绝并要求重新 query。agent 不携带 revision 或 compare-and-swap 参数。

### 响应体积护栏

对所有 action 的**聚合结果**，以最终序列化 tool result 的 UTF-8 JSON 字节数计，最大 **1 KiB**。

- `grep` 正则过宽、`query(children)` 的 children 过多、或 `query(object)` 的对象过大时，明确拒绝；
- 不截断，不自动缩小 pattern，不自动换 view；错误要说明实际大小、上限和下一步应收窄的方向；
- 精确查询**单个叶子**是例外：无论 value 自身大小都完整直返，不能截断或改变其 view。

### `update` 与 null

`update` 复用本地 CLI / 面板的统一修改管道：写入 → null 归一 → reconcile / serde 结构验证 → 执行目标节点子树的 validators，再执行所有祖先节点的 validators → 持久化 candidate → 热应用并替换 live Config → 如实返回 `restartRequired`。无关分支不重复校验；候选 Config 的任一 validator 失败则整次 update 不生效，当前内存与磁盘 Config 均保持不变。candidate 持久化失败同样不改变 live Config；持久化必须原子完成，使磁盘只能保留旧完整文件或新完整文件。校验执行顺序为子树→祖先，错误按完整 path 字典序稳定聚合返回。

`value: null` 只允许写到叶子，按 `null = 缺失` 规范使该叶子回到自身 default。object / map 节点拒绝 `null` 更新，避免把 `null` 隐式变成删除动态 map entry。Config 不提供独立的 `add` / `delete` action。

## 统一修改入口

所有配置写入必须共享同一验证与应用管道：

| 入口 | 形态 |
|---|---|
| LLM `edit_config` | 受 `no_llm_visible` 过滤的 `grep / query / update` |
| `ambery-cli` | `list` / `get <path>` / `set <path> <value>` / `schema`；默认走 HTTP 以热生效和广播，`--offline` 直写文件兜底；零 per-field 子命令 |
| 设置面板 | 第 4 个 webview 窗口；右键托盘弹出、失焦关闭；完整 schema 的机械渲染器，底部提供“显示/隐藏”“退出” |

Server API：`GET /config/schema` 返回节点列表、`readOnly` 与 version；`POST /config {path, value}` 完成验证、热应用、persist、`config_changed` 广播，并返回 `restartRequired`；另有既存的 `GET /config` 前端运行时视图（kaomoji、viewScale 等），不在本设计中改变。三个修改入口共享这条统一管道，验证只能有一份。

统一管道负责 default/null 规范、递归 reconcile、serde 验证、validator 执行、动态 enum 校验、热应用、持久化与 `restartRequired` 上报。

### 外部文件自动载入

运行中监测 `config.json` 的外部修改并自动载入。

- **文件被移动或删除**：保持当前 live Config 不变，在反射 Config UI 显示“配置文件被移动或者删除”；不自动重建默认文件或写回，后续检测到文件重新出现时再自动重试。
- **读取、解析或候选校验失败**：保持当前 live Config 不变，在反射 Config UI 显示具体加载错误；后续文件变化自动重试，直到检测到已修复的文件。
- **读取合法且候选通过**：与一次全文 update 完全相同：migration → null 归一 → reconcile → serde 结构验证 → 全部 validators → 与 live Config diff。外部载入不能绕过 validation、原子性或热/冷生效边界；任何冷字段 pending 状态变化均按保存值与运行值的差异重算。
- **应用 diff**：明确热字段立即应用；冷字段保持当前运行值并在 UI 显示 `restartRequired` 状态；所有与实际变更路径相交的 agent 已读快照标记 dirty。

### 待重启状态

待重启状态等于保存配置值与当前运行值不同；两者重新相同即立即清除状态。

### 启动载入

加载没有单一 update target，因此运行全部 validators；所有错误按完整 path 字典序一次性汇总、写入加载报告，但不阻断启动。每个错误节点按既定失败语义 default 化，随后以修复后的 Config 启动。

## 原则

- **当前结构与字段语义共位**：Config 字段是类型、说明、default、迁移 metadata 与消费者访问 metadata 的声明源。
- **本文档范围**：本文只解释 Config 的通用机制：持久化、版本/迁移、default、null、validation、反射、访问投影与统一修改管道；由具体字段触发的业务行为和工具交替流程，分别在其行为文档中定义。
- **版本范围决定迁移**：显式 `Default / Rename / Func / RenameWithFunc` 处理偏离同路径保留的历史区间；未命中才是唯一、明确的隐式 `Current`。
- **父子规则按版本去冲突**：父子可各有 metadata；只有显式 source-version range 相交才拒绝，避免函数执行顺序与覆盖猜测。
- **失败局部 default 化**：任何 migration 或校验失败只 default 化病灶节点、逐项上报并继续；不让一个 child 病灶带倒整个 Config。
- **null 没有第二语义**：object 的 `key:null` 等价于缺失；需要三态时用显式 enum。
- **object 向下构造，叶子与 map 自带 default**：静态 object 的默认形态来自 child；map default 不因已存在 map 而合并动态 key。
- **不做特殊规则，用语义明确行为**：`edit_config` 的行为完全由 schema 的 `action` 分支表达；不以缺参、空值或失败写入偷偷切换语义。
- **渐进披露，按需查**：LLM 先 grep 定位，再 query 读取确切当前结构和值，最后 update；按需要走层级，而不是猜 path 或注入完整 schema。
- **可改优先，限制例外**：Config 默认允许 agent 修改；仅在（1）对 agent 有重大影响，或（2）不可逆且容易改坏时，标记 `no_llm_visible` 限制其访问。
- **访问投影不改真值**：`no_llm_visible` 只限制 LLM tool 的读写投影，不改变本地 Config、持久化或本地管理能力。
- **如实回报**：热应用立即生效；需要重启、迁移回退、未知 path 剔除和响应体积拒绝都必须明确返回并可审计。
- **单锁单真相**：所有 Config 入口与外部自动载入经同一把锁串行处理，只可观察完整旧状态或完整新状态；不保留入口私有草稿，消灭可见状态分叉与读写竞态。
