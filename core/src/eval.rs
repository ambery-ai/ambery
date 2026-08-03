//! case 求值引擎（docs/case-eval-system.md）：表达式 + 变量 + parser + 类型系统。
//! observe 路径类 target 的 lines 与 store 的 value 共用本机制。
//! feature "case-runner" gate。

use std::collections::HashMap;

// ── ParseError ──

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub msg: String,
}

impl ParseError {
    fn new(msg: impl Into<String>) -> Self {
        Self { msg: msg.into() }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for ParseError {}

// ── 变量环境（全 string；$tail 预定义，绑定规则见 docs §变量）──

#[derive(Debug, Clone, Default)]
pub struct VarEnv {
    /// $tail 系统值（lines 求值 = 目标文件末行号；store 求值 = context.jsonl 末行号）
    pub tail: i64,
    /// 用户变量（store step 设置，全 string）
    pub vars: HashMap<String, String>,
}

impl VarEnv {
    pub fn new(tail: i64) -> Self {
        Self { tail, vars: HashMap::new() }
    }

    /// 变量引用 → i64：$tail 取系统值；用户变量取其存值（须可解析为 i64）
    fn resolve(&self, name: &str) -> Result<i64, ParseError> {
        if name == "tail" {
            return Ok(self.tail);
        }
        match self.vars.get(name) {
            Some(s) => s
                .parse::<i64>()
                .map_err(|_| ParseError::new(format!("变量 ${name} 的存值 \"{s}\" 不是 i64"))),
            None => Err(ParseError::new(format!("未知变量 ${name}（使用前须先 store）"))),
        }
    }
}

// ── Parser trait ──

pub trait Parser<'a> {
    type Input;
    type Output;
    /// 完整解析 + 求值（返回 (输出, 剩余输入)；全消费时剩余为空串）
    fn parse(&self, input: Self::Input) -> Result<(Self::Output, Self::Input), ParseError>;
    /// 预检：语法与引用校验（薄语法下与 parse 同路径，丢弃结果；docs §parser 实现注）
    fn try_parse(&self, input: Self::Input) -> Result<(), ParseError> {
        self.parse(input).map(|_| ())
    }
}

// ── 领域类型（非 Rust 类型）：Int=i64 / Str=String 直用，Var / Range 如下 ──

/// Var：变量引用（$name）
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    pub name: String,
}

/// Range：区间（两个端点 + 开闭标记）
#[derive(Debug, Clone, PartialEq)]
pub struct LinesRange {
    pub from: i64,
    pub to: i64,
    pub from_inclusive: bool,
    pub to_inclusive: bool,
}

/// DirectToString（独立模块，与 parser 无关）：界定哪些类型能直接转 string
pub trait DirectToString {
    fn direct_to_string(&self) -> String;
}

impl DirectToString for i64 {
    fn direct_to_string(&self) -> String {
        self.to_string()
    }
}

impl DirectToString for Var {
    fn direct_to_string(&self) -> String {
        format!("${}", self.name)
    }
}

impl DirectToString for LinesRange {
    fn direct_to_string(&self) -> String {
        let lb = if self.from_inclusive { '[' } else { '(' };
        let rb = if self.to_inclusive { ']' } else { ')' };
        format!("{lb}{},{}{rb}", self.from, self.to)
    }
}

// ── 扫描辅助（parser 跳过空白）──

fn skip_ws(s: &str) -> &str {
    s.trim_start()
}

/// `$name`（$ 后跟 [A-Za-z0-9_]+）→ (变量名, 剩余)
fn scan_var_name(s: &str) -> Result<(&str, &str), ParseError> {
    let s = s
        .strip_prefix('$')
        .ok_or_else(|| ParseError::new(format!("期望变量引用（$name），得到 \"{s}\"")))?;
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if end == 0 {
        return Err(ParseError::new("变量名缺失（$ 后须跟标识符）"));
    }
    Ok((&s[..end], &s[end..]))
}

/// 数字字面量（可带前导负号）→ (值, 剩余)
fn scan_int(s: &str) -> Result<(i64, &str), ParseError> {
    let (neg, s2) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let end = s2.find(|c: char| !c.is_ascii_digit()).unwrap_or(s2.len());
    if end == 0 {
        return Err(ParseError::new(format!("期望数字，得到 \"{s}\"")));
    }
    let mut v: i64 = s2[..end]
        .parse()
        .map_err(|_| ParseError::new(format!("数字溢出：\"{}\"", &s2[..end])))?;
    if neg {
        v = -v;
    }
    Ok((v, &s2[end..]))
}

/// 无符号数字（偏移量）→ (值, 剩余)
fn scan_uint(s: &str) -> Result<(i64, &str), ParseError> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return Err(ParseError::new(format!("期望偏移数字，得到 \"{s}\"")));
    }
    let v: i64 = s[..end]
        .parse()
        .map_err(|_| ParseError::new(format!("数字溢出：\"{}\"", &s[..end])))?;
    Ok((v, &s[end..]))
}

// ── 3 个 parser（input 统一 string）──

/// ExprParser：端点表达式（`数字 | $name | $name ± N`，一层加减非递归）→ i64
pub struct ExprParser<'e> {
    pub env: &'e VarEnv,
}

impl<'a, 'e> Parser<'a> for ExprParser<'e> {
    type Input = &'a str;
    type Output = i64;

    fn parse(&self, input: Self::Input) -> Result<(i64, Self::Input), ParseError> {
        let s = skip_ws(input);
        let (base, rest) = if s.starts_with('$') {
            let (name, rest) = scan_var_name(s)?;
            (self.env.resolve(name)?, rest)
        } else {
            scan_int(s)?
        };
        // 仅变量形式可带 ± N 偏移（数字端点不支持，留作剩余由调用方判全消费）
        if s.starts_with('$') {
            let rest_ws = skip_ws(rest);
            if let Some(r) = rest_ws.strip_prefix('+') {
                let (n, rest) = scan_uint(skip_ws(r))?;
                return Ok((base + n, skip_ws(rest)));
            }
            if let Some(r) = rest_ws.strip_prefix('-') {
                let (n, rest) = scan_uint(skip_ws(r))?;
                return Ok((base - n, skip_ws(rest)));
            }
        }
        Ok((base, rest))
    }
}

/// VarIntParser：纯变量引用（`$name`，不带偏移）→ i64 行号
pub struct VarIntParser<'e> {
    pub env: &'e VarEnv,
}

impl<'a, 'e> Parser<'a> for VarIntParser<'e> {
    type Input = &'a str;
    type Output = i64;

    fn parse(&self, input: Self::Input) -> Result<(i64, Self::Input), ParseError> {
        let s = skip_ws(input);
        let (name, rest) = scan_var_name(s)?;
        Ok((self.env.resolve(name)?, skip_ws(rest)))
    }
}

/// IntParser：数字字面量 → i64
pub struct IntParser;

impl<'a> Parser<'a> for IntParser {
    type Input = &'a str;
    type Output = i64;

    fn parse(&self, input: Self::Input) -> Result<(i64, Self::Input), ParseError> {
        let (v, rest) = scan_int(skip_ws(input))?;
        Ok((v, skip_ws(rest)))
    }
}

/// RangeParser：区间外壳（开闭 + 逗号 + 两个端点）→ LinesRange
pub struct RangeParser<'e> {
    pub env: &'e VarEnv,
}

impl<'a, 'e> Parser<'a> for RangeParser<'e> {
    type Input = &'a str;
    type Output = LinesRange;

    fn parse(&self, input: Self::Input) -> Result<(LinesRange, Self::Input), ParseError> {
        let s = skip_ws(input);
        let (from_inclusive, s) = match s.chars().next() {
            Some('[') => (true, &s[1..]),
            Some('(') => (false, &s[1..]),
            _ => return Err(ParseError::new(format!("区间须以 [ 或 ( 开头，得到 \"{s}\""))),
        };
        let ep = ExprParser { env: self.env };
        let (from, rest) = ep.parse(s)?;
        let rest = skip_ws(rest);
        let rest = rest
            .strip_prefix(',')
            .ok_or_else(|| ParseError::new(format!("区间缺逗号，得到 \"{rest}\"")))?;
        let (to, rest) = ep.parse(rest)?;
        let rest = skip_ws(rest);
        let (to_inclusive, rest) = match rest.chars().next() {
            Some(']') => (true, &rest[1..]),
            Some(')') => (false, &rest[1..]),
            _ => return Err(ParseError::new(format!("区间须以 ] 或 ) 结尾，得到 \"{rest}\""))),
        };
        Ok((
            LinesRange { from, to, from_inclusive, to_inclusive },
            skip_ws(rest),
        ))
    }
}

// ── store 求值（docs §完整链路）：按 type 选 parser 求值 → DirectToString → string ──

/// 全消费校验：剩余输入须为空（否则语法错误）
fn full_consumed(rest: &str, input: &str) -> Result<(), ParseError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(ParseError::new(format!(
            "\"{input}\" 有多余内容 \"{rest}\"（语法错误）"
        )))
    }
}

/// 求值并转 string；Output: DirectToString 即「类型可落」的编译期保证
fn eval_to_string<'a, P>(parser: &P, input: &'a str) -> Result<String, ParseError>
where
    P: Parser<'a, Input = &'a str>,
    P::Output: DirectToString,
{
    let (out, rest) = parser.parse(input)?;
    full_consumed(rest, input)?;
    Ok(out.direct_to_string())
}

/// store step 的 value 求值：type ∈ {expr, var, int, str}（docs §变量）
pub fn eval_store(env: &VarEnv, ty: &str, value: &str) -> Result<String, ParseError> {
    match ty {
        "expr" => eval_to_string(&ExprParser { env }, value),
        "var" => eval_to_string(&VarIntParser { env }, value),
        "int" => eval_to_string(&IntParser, value),
        "str" => Ok(value.to_string()),
        other => Err(ParseError::new(format!(
            "未知 store 类型 \"{other}\"（合法：expr/var/int/str）"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> VarEnv {
        let mut e = VarEnv::new(122); // $tail = 122
        e.vars.insert("cursor".into(), "73".into());
        e
    }

    // ── ExprParser ──

    #[test]
    fn expr_number() {
        assert_eq!(ExprParser { env: &env() }.parse("50").unwrap(), (50, ""));
    }

    #[test]
    fn expr_tail() {
        assert_eq!(ExprParser { env: &env() }.parse("$tail").unwrap(), (122, ""));
    }

    #[test]
    fn expr_var_offset() {
        let e = env();
        assert_eq!(ExprParser { env: &e }.parse("$tail-49").unwrap(), (73, ""));
        assert_eq!(ExprParser { env: &e }.parse("$tail+10").unwrap(), (132, ""));
        assert_eq!(ExprParser { env: &e }.parse("$cursor-49").unwrap(), (24, ""));
    }

    #[test]
    fn expr_whitespace() {
        let e = env();
        assert_eq!(ExprParser { env: &e }.parse("  $tail - 50 ").unwrap(), (72, ""));
        assert_eq!(ExprParser { env: &e }.parse("50, 100").unwrap(), (50, ", 100")); // 剩余留调用方
    }

    #[test]
    fn expr_number_no_offset() {
        // 数字端点不支持 ±：剩余留给调用方判全消费
        assert_eq!(ExprParser { env: &env() }.parse("50-3").unwrap(), (50, "-3"));
    }

    #[test]
    fn expr_errors() {
        let e = env();
        assert!(ExprParser { env: &e }.parse("$nope").is_err()); // 未知变量
        assert!(ExprParser { env: &e }.parse("$tail*2").is_ok()); // "*" 留剩余
        assert!(ExprParser { env: &e }.parse("").is_err()); // 空
        assert!(ExprParser { env: &e }.parse("$").is_err()); // 变量名缺失
        assert!(ExprParser { env: &e }.parse("$tail-").is_err()); // 偏移缺数字
    }

    #[test]
    fn expr_user_var_non_i64() {
        let mut e = VarEnv::new(1);
        e.vars.insert("s".into(), "abc".into());
        let err = ExprParser { env: &e }.parse("$s").unwrap_err();
        assert!(err.msg.contains("不是 i64"));
    }

    // ── VarIntParser ──

    #[test]
    fn varint_pure_ref_only() {
        let e = env();
        assert_eq!(VarIntParser { env: &e }.parse("$tail").unwrap(), (122, ""));
        assert_eq!(VarIntParser { env: &e }.parse("$cursor").unwrap(), (73, ""));
        // 带偏移 / 数字字面量都不是纯引用
        assert_eq!(VarIntParser { env: &e }.parse("$tail-1").unwrap(), (122, "-1"));
        assert!(VarIntParser { env: &e }.parse("123").is_err());
    }

    // ── IntParser ──

    #[test]
    fn int_literal() {
        assert_eq!(IntParser.parse("50").unwrap(), (50, ""));
        assert_eq!(IntParser.parse("-3").unwrap(), (-3, ""));
        assert!(IntParser.parse("x5").is_err());
    }

    // ── RangeParser ──

    #[test]
    fn range_open_close() {
        let e = env();
        let (r, rest) = RangeParser { env: &e }.parse("[50, 100]").unwrap();
        assert_eq!(r, LinesRange { from: 50, to: 100, from_inclusive: true, to_inclusive: true });
        assert_eq!(rest, "");
        let (r, _) = RangeParser { env: &e }.parse("($tail-49, $tail]").unwrap();
        assert_eq!(r, LinesRange { from: 73, to: 122, from_inclusive: false, to_inclusive: true });
        let (r, _) = RangeParser { env: &e }.parse("[1, 5)").unwrap();
        assert!(r.from_inclusive && !r.to_inclusive);
        let (r, _) = RangeParser { env: &e }.parse("( 1 , 5 )").unwrap();
        assert!(!r.from_inclusive && !r.to_inclusive);
    }

    #[test]
    fn range_errors() {
        let e = env();
        assert!(RangeParser { env: &e }.parse("50, 100").is_err()); // 缺开括号
        assert!(RangeParser { env: &e }.parse("[50 100]").is_err()); // 缺逗号
        assert!(RangeParser { env: &e }.parse("[50, 100").is_err()); // 缺闭括号
        assert!(RangeParser { env: &e }.parse("[$nope, 5]").is_err()); // 未知变量
    }

    // ── DirectToString ──

    #[test]
    fn direct_to_string_forms() {
        assert_eq!(73i64.direct_to_string(), "73");
        assert_eq!(Var { name: "cursor".into() }.direct_to_string(), "$cursor");
        let r = LinesRange { from: 49, to: 73, from_inclusive: false, to_inclusive: true };
        assert_eq!(r.direct_to_string(), "(49,73]");
    }

    // ── 完整链路（docs §类型系统）："$tail-49" → parse → i64 → to_string → 存变量 ──

    #[test]
    fn full_chain_doc_example() {
        let e = env(); // tail = 122
        assert_eq!(eval_store(&e, "expr", "$tail-49").unwrap(), "73");
    }

    // ── store 求值 ──

    #[test]
    fn store_types() {
        let e = env();
        assert_eq!(eval_store(&e, "expr", "$tail").unwrap(), "122");
        assert_eq!(eval_store(&e, "var", "$cursor").unwrap(), "73");
        assert_eq!(eval_store(&e, "int", "42").unwrap(), "42");
        assert_eq!(eval_store(&e, "str", "任意 $ 字符串").unwrap(), "任意 $ 字符串");
        assert!(eval_store(&e, "expr", "$tail*2").is_err()); // 全消费校验
        assert!(eval_store(&e, "var", "$tail-1").is_err()); // 纯引用拒绝偏移
        assert!(eval_store(&e, "bogus", "1").is_err()); // 未知类型
    }

    // ── try_parse 预检：语法与引用在同一遍被检查 ──

    #[test]
    fn try_parse_catches_syntax_and_ref() {
        let e = env();
        assert!(ExprParser { env: &e }.try_parse("$tail-49").is_ok());
        assert!(ExprParser { env: &e }.try_parse("$unknown").is_err());
        assert!(RangeParser { env: &e }.try_parse("[1, 2]").is_ok());
        assert!(RangeParser { env: &e }.try_parse("[1, ").is_err());
    }
}
