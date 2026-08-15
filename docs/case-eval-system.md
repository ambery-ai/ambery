# Case Eval System

> Concept: the case runner's expression evaluation system (docs/case-runner.md §reading of the observability system / store companion).

## Principles

> **Scope of this document** — this document defines the case evaluation system: reading (observe path-type targets) / store / expressions / variables / parser / type system / checkhealth; for runner infrastructure, see `docs/case-runner.md`.

> **Introduce a variable system to control execution flow, with a minimal design** — use a variable system to express positions/parameters during execution (offsets for path-type reads, stored values); minimal design: all variables are string, with a built-in last-line variable.

> **Use a parser trait to constrain the type system, implement variable control and computation, and provide a simple expr to lower code cost** — the parser trait unifies type constraints (parse / try_parse), supporting variable evaluation and expression computation; provide a minimal expr (number | `$tail` ± N) to avoid building a complex expression engine.

> **Provide pre-parse for checkhealth preflight** — expressions can be preflighted with try_parse before the case runs; syntax errors surface during the checkhealth phase, without evaluation.

## System definition

The `lines` (file access position) of observe path-type targets and store's value (variable setting) share the same expression + variable + parser + type mechanism.

**Why it is needed** (the variable system controls execution flow):

```text
① 增量读：store 记住上次位置，后续 ($cursor, $tail] 读新增（不用每次全量）
② 参数化：store 设值，多个 step 复用（读不同窗口/偏移）
③ 免硬编码：位置/偏移用变量表达，改一处（store）多处生效
```

## Reading (observe path-type targets)

observe's path-type targets (context / effects) can carry `lines` to directly read file content (read_file fused into observe):

```text
{ "observe": [
    { "target": "agents" },
    { "target": "context", "lines": "[$tail-49, $tail]" }
  ] }
```

`lines` interval notation (inclusive/exclusive endpoints):

```text
[a, b]   闭区间（含 a、b）
(a, b]   from 开（不含 a）
[a, b)   to 开（不含 b）
(a, b)   全开（不含 a、b）
```

## Expressions

Endpoint expression: `数字 | $tail ± N` (one level of addition/subtraction, non-recursive)

```text
50            → 数字
$tail         → 变量（末行号）
$tail-49      → 相对末行偏移
($tail-N, $tail]  → 倒数 N 行（开区间计数，无负索引）
```

**Whitespace**: `lines` allows arbitrary spaces (the parser skips whitespace), so `[50, 100]`, `$tail - 50`, and `[ $tail-50 , $tail ]` are all valid.

## Variables

All variables are string:

- `$tail`: predefined (last line number, system-provided value). **Binding rule**: when evaluating lines = the last line number of the target file being read (context → context.jsonl, effects → effect.jsonl); when evaluating store = the last line number of context.jsonl (context is the case's main timeline file)
- User variables: set by the `store` step

```text
{ "store": { "<name>": { "type": "expr|var|int|str", "value": "<字符串>" } } }
```

`type` selects the parser, `value` is the input string; after evaluation it is stored via to_string (all string).

store types:

| type | parser | semantics |
|---|---|---|
| `expr` | ExprParser | expression evaluation (number \| `$name` \| `$name` ± N) → i64 |
| `var`  | VarIntParser | pure variable reference (`$name`, no offset) → i64: $tail takes the system value; user variables take their stored value (must be parseable as i64, otherwise error) |
| `int`  | IntParser | numeric literal → i64 |
| `str`  | — | string stored as-is (no parsing) |

Variable reference: `$name`, supports ± integer offset:

```text
$tail          末行号
$cursor        用户变量（store 设置的）
$cursor-49     用户变量偏移
$tail+10       预定义变量偏移
```

lines endpoints: `数字 | $name | $name ± N`.

### Usage example (steps)

A real cross-step use of variables in a case — `store` remembers the read-window start, and a later `observe` reads with variables, with real operations in between:

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

> **try_parse implementation note**: this grammar is extremely thin (number | `$name` ± N, one level of addition/subtraction), syntax validation and evaluation share the same path,
> and there are no side effects (evaluation is a pure function), so try_parse = parse then discard the result; in preflight scenarios the variable environment is constructed with
> "placeholder values whose names are known", so reference validity and syntax are checked in the same pass.

4 parsers (input uniformly string):

```text
RangeParser   区间外壳（开闭 + 逗号 + 两个端点）→ LinesRange
ExprParser    端点表达式（数字 | $name ± N）→ i64
VarIntParser  纯变量引用（$name，不带偏移）→ i64 行号
IntParser     数字字面量（store type=int）→ i64
```

RangeParser outputs `LinesRange` (the interval structure):

```text
{ lb: '[' | '(' , from: <表达式>, to: <表达式>, rb: ']' | ')' }
  lb/rb   决定端点含/不含
  from/to 经 ExprParser 求值为行号
```

## Type system

Independent domain types (not Rust types):

```text
Int     整数（行号、偏移）
Var     变量引用（$tail / $cursor）
Expr    表达式（数字 | 变量 ± 偏移）
Str     字符串（原始值）
Range   区间（两个端点 + 开闭标记）
```

**DirectToString** (an independent module, unrelated to parser): defines which types can be directly converted to string. Implemented conversions: `Int` (i64 → `"73"`), `Var` (→ `"$name"`), `Range` (LinesRange → `{lb}{from},{to}{rb}`, e.g. `"(49,73]"`) ... types not implemented are rejected when stored as variables (guaranteed at compile time by Rust generic bounds).

**Complete chain** (expression → string variable):

```text
"$tail-49"（字符串）
  → ExprParser.parse        → i64 73（Output）
  → 校验 i64: DirectToString ✓
  → to_string               → "73"
  → 存变量（string）
```

## checkhealth

pre-parse preflight (static validation, does not execute the case):

```text
① 表达式 try_parse    语法合法
② 变量引用有效        $tail 预定义；用户变量使用前已 store
③ store 类型合法       type ∈ {expr, var, int, str}
④ 类型可落             Output 实现 DirectToString
⑤ target 合法          observe 的 target 是可观测模块；路径类（context/effects）可带 lines（表达式 try_parse 见①）
```
