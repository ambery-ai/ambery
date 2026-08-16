# Case Eval System

[English](case-eval-system.md) | 中文

> 概念：case runner 的表达式求值系统（docs/case-runner.md §可观测体系的读取 / store 配套）。

## 原则

> **本文档范围**——本文定义 case 的求值系统：读取（observe 路径类 target）/ store / 表达式 / 变量 / parser / 类型系统 / checkhealth；runner 基础设施见 `docs/case-runner.md`。

> **引入变量系统控制执行流，极简设计**——用变量系统表达执行中的位置/参数（路径类读取的偏移、存储的值）；极简设计：变量统一 string、内置末行变量。

> **用 parser trait 约束类型系统，实现变量控制与计算，提供简单 expr 降低代码成本**——parser trait 统一类型约束（parse / try_parse），支撑变量求值与表达式计算；提供极简 expr（数字 | `$tail` ± N），避免自造复杂表达式引擎。

> **提供 pre parse 实现 checkhealth 预检**——表达式在 case 跑前可 try_parse 预检，语法错误在 checkhealth 阶段暴露，不求值。

## 系统定义

observe 路径类 target 的 `lines`（文件访问位置）与 store 的 value（变量设置）共用同一套 表达式 + 变量 + parser + 类型 机制。

**为什么需要**（变量系统控制执行流）：

```text
① 增量读：store 记住上次位置，后续 ($cursor, $tail] 读新增（不用每次全量）
② 参数化：store 设值，多个 step 复用（读不同窗口/偏移）
③ 免硬编码：位置/偏移用变量表达，改一处（store）多处生效
```

## 读取（observe 的路径类 target）

observe 的路径类 target（context / effects）可带 `lines` 直接读取文件内容（read_file 融合进 observe）：

```text
{ "observe": [
    { "target": "agents" },
    { "target": "context", "lines": "[$tail-49, $tail]" }
  ] }
```

`lines` 开闭区间（含/不含端点）：

```text
[a, b]   闭区间（含 a、b）
(a, b]   from 开（不含 a）
[a, b)   to 开（不含 b）
(a, b)   全开（不含 a、b）
```

## 表达式

端点表达式：`数字 | $tail ± N`（一层加减，非递归）

```text
50            → 数字
$tail         → 变量（末行号）
$tail-49      → 相对末行偏移
($tail-N, $tail]  → 倒数 N 行（开区间计数，无负索引）
```

**空白**：`lines` 支持任意空格（parser 跳过空白），如 `[50, 100]`、`$tail - 50`、`[ $tail-50 , $tail ]` 均有效。

## 变量

变量全 string：

- `$tail`：预定义（末行号，系统提供值）。**绑定规则**：lines 求值时 = 被读目标文件的末行号（context → context.jsonl，effects → effect.jsonl）；store 求值时 = context.jsonl 末行号（context 是 case 的主时间轴文件）
- 用户变量：`store` step 设置

```text
{ "store": { "<name>": { "type": "expr|var|int|str", "value": "<字符串>" } } }
```

`type` 选 parser，`value` 为输入字符串；求值后经 to_string 存变量（全 string）。

store 的 type：

| type | parser | 语义 |
|---|---|---|
| `expr` | ExprParser | 表达式求值（数字 \| `$name` \| `$name` ± N）→ i64 |
| `var`  | VarIntParser | 纯变量引用（`$name`，不带偏移）→ i64：$tail 取系统值；用户变量取其存值（须可解析为 i64，否则报错） |
| `int`  | IntParser | 数字字面量 → i64 |
| `str`  | — | 字符串直存（不做解析） |

变量引用：`$name`，支持 ± 整数偏移：

```text
$tail          末行号
$cursor        用户变量（store 设置的）
$cursor-49     用户变量偏移
$tail+10       预定义变量偏移
```

lines 端点：`数字 | $name | $name ± N`。

### 运用示例（steps）

一个 case 里变量跨 step 的真实运用——`store` 记住读取窗口起点，后续 `observe` 用变量读取，中间夹真实操作：

```json
"steps": [
  { "load": {} },
  { "observe": [
      { "target": "agents" },
      { "target": "context", "lines": "($tail-50, $tail]" }
  ] },
  { "store": { "cursor": { "type": "expr", "value": "$tail" } } },
  { "timer_scan": {} },
  { "observe": [
      { "target": "context", "lines": "($cursor, $tail]" }
  ] }
]
```

```text
① load        重放快照
② observe     读 agents + context 末尾 50 行（直接用 $tail，不先 store）
③ store       cursor = $tail（记住本次读取的末行位置）
④ timer_scan  真实操作（context 可能新增内容）
⑤ observe     读 ($cursor, $tail]（上次位置之后到新末尾 = 新增部分）——变量读承担 context_diff 的增量语义
```

## parser

```rust
pub trait Parser<'a> {
    type Input;
    type Output;
    /// 完整解析 + 求值（返回 (输出, 剩余输入)；全消费时剩余为空串）
    fn parse(&self, input: Self::Input) -> Result<(Self::Output, Self::Input), ParseError>;
    /// 预检：只验证语法与引用，丢弃结果
    fn try_parse(&self, input: Self::Input) -> Result<(), ParseError>;
}
```

> **try_parse 实现注**：本语法极薄（数字 | `$name` ± N，一层加减），语法校验与求值同路径、
> 无侧效应（求值是纯函数），故 try_parse = parse 后丢弃结果；预检场景变量环境用
> 「名字已知的占位值」构造，从而引用有效性与语法在同一遍被检查。

4 个 parser（input 统一 string）：

```text
RangeParser   区间外壳（开闭 + 逗号 + 两个端点）→ LinesRange
ExprParser    端点表达式（数字 | $name ± N）→ i64
VarIntParser  纯变量引用（$name，不带偏移）→ i64 行号
IntParser     数字字面量（store type=int）→ i64
```

RangeParser 输出 `LinesRange`（区间结构）：

```text
{ lb: '[' | '(' , from: <表达式>, to: <表达式>, rb: ']' | ')' }
  lb/rb   决定端点含/不含
  from/to 经 ExprParser 求值为行号
```

## 类型系统

独立领域类型（非 Rust 类型）：

```text
Int     整数（行号、偏移）
Var     变量引用（$tail / $cursor）
Expr    表达式（数字 | 变量 ± 偏移）
Str     字符串（原始值）
Range   区间（两个端点 + 开闭标记）
```

**DirectToString**（独立模块，与 parser 无关）：界定哪些类型能直接转 string。已实现可转：`Int`（i64 → `"73"`）、`Var`（→ `"$name"`）、`Range`（LinesRange → `{lb}{from},{to}{rb}`，如 `"(49,73]"`）……未实现的类型存变量时拒绝（Rust 泛型约束编译期保证）。

**完整链路**（表达式 → string 变量）：

```text
"$tail-49"（字符串）
  → ExprParser.parse        → i64 73（Output）
  → 校验 i64: DirectToString ✓
  → to_string               → "73"
  → 存变量（string）
```

## checkhealth

pre parse 预检（静态校验，不执行 case）：

```text
① 表达式 try_parse    语法合法
② 变量引用有效        $tail 预定义；用户变量使用前已 store
③ store 类型合法       type ∈ {expr, var, int, str}
④ 类型可落             Output 实现 DirectToString
⑤ target 合法          observe 的 target 是可观测模块；路径类（context/effects）可带 lines（表达式 try_parse 见①）
```
