# Case Eval System

English | [中文](case-eval-system.zh.md)

> - **Scope of this document** — concept: the case runner's expression evaluation system (§reading of the observability system / store companion). This document defines reading (observe path-type targets) / store / expressions / variables / parser / type system / checkhealth; the runner infrastructure belongs to [case-runner.md](case-runner.md).
> - **Introduce a variable system to control execution flow, with a minimal design** — use a variable system to express positions/parameters during execution (offsets for path-type reads, stored values); minimal design: all variables are string, with a built-in last-line variable.
> - **Use a parser trait to constrain the type system, implement variable control and computation, and provide a simple expr to lower code cost** — the parser trait unifies type constraints (parse / try_parse), supporting variable evaluation and expression computation; provide a minimal expr (number | `$tail` ± N) to avoid building a complex expression engine.
> - **Provide pre-parse for checkhealth preflight** — expressions can be preflighted with try_parse before the case runs; syntax errors surface during the checkhealth phase, without evaluation.

## System definition

The `lines` (file access position) of observe path-type targets and store's value (variable setting) share the same expression + variable + parser + type mechanism.

**Why it is needed** (the variable system controls execution flow):

```text
① incremental read: store remembers the previous position, and later reads take ($cursor, $tail] (no full read each time)
② parameterization: store sets a value that several steps reuse (reading different windows / offsets)
③ no hard-coding: positions / offsets are expressed as variables; change one place (store) and every use follows
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
[a, b]   closed interval (includes a and b)
(a, b]   from open (excludes a)
[a, b)   to open (excludes b)
(a, b)   fully open (excludes a and b)
```

## Expressions

Endpoint expression: `number | $tail ± N` (one level of addition/subtraction, non-recursive)

```text
50            → number
$tail         → variable (last line number)
$tail-49      → offset relative to the last line
($tail-N, $tail]  → the last N lines (open-interval counting, no negative index)
```

**Whitespace**: `lines` allows arbitrary spaces (the parser skips whitespace), so `[50, 100]`, `$tail - 50`, and `[ $tail-50 , $tail ]` are all valid.

## Variables

All variables are string:

- `$tail`: predefined (last line number, system-provided value). **Binding rule**: when evaluating lines = the last line number of the target file being read (context → context.jsonl, effects → effect.jsonl); when evaluating store = the last line number of context.jsonl (context is the case's main timeline file)
- User variables: set by the `store` step

```text
{ "store": { "<name>": { "type": "expr|var|int|str", "value": "<string>" } } }
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
$tail          last line number
$cursor        user variable (set by store)
$cursor-49     user variable offset
$tail+10       predefined variable offset
```

lines endpoints: `number | $name | $name ± N`.

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
① load        replay the snapshot
② observe     read agents + the last 50 lines of context (using $tail directly, without storing first)
③ store       cursor = $tail (remember the last line read this time)
④ timer_scan  a real operation (context may gain content)
⑤ observe     read ($cursor, $tail] (from the previous position to the new end = the newly added part) — variable reads carry context_diff's incremental semantics
```

## parser

```rust
pub trait Parser<'a> {
    type Input;
    type Output;
    /// full parse + evaluation (returns (output, remaining input); the remainder is an empty string when fully consumed)
    fn parse(&self, input: Self::Input) -> Result<(Self::Output, Self::Input), ParseError>;
    /// preflight: validate syntax and references only, discarding the result
    fn try_parse(&self, input: Self::Input) -> Result<(), ParseError>;
}
```

> **try_parse implementation note**: this grammar is extremely thin (number | `$name` ± N, one level of addition/subtraction), syntax validation and evaluation share the same path,
> and there are no side effects (evaluation is a pure function), so try_parse = parse then discard the result; in preflight scenarios the variable environment is constructed with
> "placeholder values whose names are known", so reference validity and syntax are checked in the same pass.

4 parsers (input uniformly string):

```text
RangeParser   interval shell (open/closed + comma + two endpoints) → LinesRange
ExprParser    endpoint expression (number | $name ± N) → i64
VarIntParser  pure variable reference ($name, no offset) → i64 line number
IntParser     numeric literal (store type=int) → i64
```

RangeParser outputs `LinesRange` (the interval structure):

```text
{ lb: '[' | '(' , from: <expression>, to: <expression>, rb: ']' | ')' }
  lb/rb   decide whether each endpoint is included
  from/to are evaluated to line numbers by ExprParser
```

## Type system

Independent domain types (not Rust types):

```text
Int     integer (line number, offset)
Var     variable reference ($tail / $cursor)
Expr    expression (number | variable ± offset)
Str     string (raw value)
Range   interval (two endpoints + open/closed markers)
```

**DirectToString** (an independent module, unrelated to parser): defines which types can be directly converted to string. Implemented conversions: `Int` (i64 → `"73"`), `Var` (→ `"$name"`), `Range` (LinesRange → `{lb}{from},{to}{rb}`, e.g. `"(49,73]"`) ... types not implemented are rejected when stored as variables (guaranteed at compile time by Rust generic bounds).

**Complete chain** (expression → string variable):

```text
"$tail-49" (string)
  → ExprParser.parse        → i64 73 (Output)
  → validate i64: DirectToString ✓
  → to_string               → "73"
  → stored as a variable (string)
```

## checkhealth

pre-parse preflight (static validation, does not execute the case):

```text
① expression try_parse   syntax valid
② variable references valid  $tail predefined; user variables stored before use
③ store type valid       type ∈ {expr, var, int, str}
④ type storable          Output implements DirectToString
⑤ target valid           observe targets are observable modules; path-type (context/effects) may carry lines (expression try_parse, see ①)
```
