//! Config 域：类型 + load/save。
//! 子模块：reflect（声明式 UI 反射）、migrate（版本与迁移加载管线）、
//! meta（字段行为元数据注册表：validation / no_llm_visible / 冷字段）。

pub mod meta;
pub mod migrate;
pub mod reflect;
pub mod theme;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CONFIG_FILE: &str = "config.json";

/// Config：持久化单文件 config.json，edit_config tool 可写
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    /// 表情领域：两个固定池；池内表情名称是动态 map key。
    /// 两池 key 全局唯一（validate_kaomoji_pools 保证不相交），无隐式优先级
    #[serde(default)]
    pub kaomoji: KaomojiConfig,
    /// Compression 输出预留默认值（#16）：触发点 = context_window − reserve，
    /// provider 未设 `compression_reserve` 时用此值
    #[serde(default = "default_compression_reserve")]
    pub compression_reserve_default: usize,
    /// set_autonomy 省略 ttlMs 时的默认值
    #[serde(default = "default_ttl_ms")]
    pub set_autonomy_default_ttl_ms: u64,
    // Filter 按实例 hook kind 选择：本结构不设全局策略字段
    /// Timer 兜底扫描调度（全部冷字段，重启生效）
    #[serde(default)]
    pub timer: TimerConfig,
    /// Terminal Adapter 开关：
    /// 每 adapter 一个布尔；全 false = 无终端访问（Hook 驱动核心体验仍可用）。冷字段（装配期生效）
    #[serde(default)]
    pub terminal: TerminalConfig,
    /// stop hook 模式：queue_only（默认，hint 按需读）/ auto_read / message
    #[serde(default = "default_stop_hook_mode")]
    pub stop_hook_mode: String,
    /// 一次 LLM response 最多执行的 tool call 数（冷字段）
    #[serde(default = "default_max_tool_calls_in_one_response")]
    pub max_tool_calls_in_one_response: usize,
    /// 一条已放行输入处理期间累计最多执行的 tool call 数（冷字段）
    #[serde(default = "default_max_tool_calls_per_turn")]
    pub max_tool_calls_per_turn: usize,
    /// system prompt 基座（运行时与 kaomoji 表、顶层状态拼装）
    pub base_prompt: String,
    /// View 缩放（球场圆形默认 0.5）
    #[serde(default = "default_view_scale")]
    pub view_scale: f64,
    /// 未读角标样式：number（纯数字，默认）/ bubble（气泡）
    #[serde(default = "default_badge_style")]
    pub badge_style: String,
    /// 未读角标方位：right（正右边，默认）/ left
    #[serde(default = "default_badge_side")]
    pub badge_side: String,
    /// 当前主题名：合法值为 themes 的 key（动态 enum，OPTIONS 注册表校验）
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 主题表：主题名 → token 覆写表（token 名去 `--ov-` 前缀 → CSS 值）；
    /// 未覆写的 token 回落 styles.css :root 内置默认。内置 "dark" = 全空覆写（= 当前默认视觉）
    #[serde(default = "default_themes")]
    pub themes: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// UI 语言：zh / en。首次初始化跟随受支持的系统语言，不支持回退项目默认；
    /// 用户显式选择后系统语言不再覆盖
    #[serde(default = "default_ui_language")]
    pub ui_language: String,
    /// Harness 内部语言：zh / en。首次初始化取项目明确默认语言（不随系统
    /// 语言），切换从下一次新的 LLM 交互起生效，不改写既有 Context/历史
    #[serde(default = "default_harness_language")]
    pub harness_language: String,
    /// pet 名称：稳定身份值。首次初始化写入正式默认名（Ambery，
    /// 不按语言区分）；此后与语言独立，不自动改名、不参与翻译。
    /// 不标 no_llm_visible：本地用户与 LLM 经各自 Config 入口读写
    #[serde(default = "default_pet_name")]
    pub name: String,
    /// Compression 保留目标：压缩后保留的原始 message
    /// 条数目标；切口按完整 turn 边界收口（不拆 tool 序列）。冷字段，重启后生效
    #[serde(default = "default_keep_recent")]
    pub context_compression_keep_recent_messages: usize,
    /// LLM 多 profile 配置
    #[serde(default)]
    pub llm: LlmConfig,
    /// Effort 档位配置：按 Queue 来源的档位映射（未列出来源用
    /// default）+ user_chat 关键词改写表
    #[serde(default)]
    pub effort: EffortConfig,
    /// 只读降级模式：true 时任何 save 报错。
    /// 运行时标记，不落盘（serde skip）
    #[serde(skip)]
    pub read_only: bool,
    /// 加载管线报告（迁移/reconcile/降级每个动作一行「上报」）。
    /// 运行时数据，不落盘（serde skip）
    #[serde(skip)]
    pub load_report: Vec<String>,
}

/// Timer 调度子树：兜底扫描参数；全部冷字段（TimerWheel/主循环
/// 启动时构建，不在运行中重建）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TimerConfig {
    /// 每实例兜底扫描间隔；≤ 0 = 整体禁用（真实 hook 接入初期只留 hook 驱动）
    #[serde(default = "default_timer_interval")]
    pub interval_ms: i64,
    /// 错峰窗口：多实例到期时间在窗口内打散（「错峰分布偏移量」）
    #[serde(default = "default_timer_stagger")]
    pub stagger_ms: i64,
    /// 主循环粒度：每 tick 醒一次取到期实例（interval 小于它也最多每 tick 一扫）
    #[serde(default = "default_timer_tick")]
    pub tick_ms: u64,
    /// 每 tick 最多扫描实例数（限流）
    #[serde(default = "default_timer_batch")]
    pub batch: usize,
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self {
            interval_ms: default_timer_interval(),
            stagger_ms: default_timer_stagger(),
            tick_ms: default_timer_tick(),
            batch: default_timer_batch(),
        }
    }
}

/// Terminal Adapter 配置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalConfig {
    /// 启用 wt 适配器（WtAdapter，独立 C# sidecar 进程）。默认 true——保持既有
    /// sidecar 自动发现行为（用户裁决）；「未列出的 adapter 默认 false」指未入 schema 者
    #[serde(default = "default_adapter_wt")]
    pub adapter_wt: bool,
    /// 启用 zellij 适配器（ZellijAdapter，Rust 直调 zellij CLI）。默认 false
    #[serde(default)]
    pub adapter_zellij: bool,
}

fn default_adapter_wt() -> bool {
    true
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            adapter_wt: default_adapter_wt(),
            adapter_zellij: false,
        }
    }
}

/// LLM 配置 v2：多 provider profile + active 选择器（切换不丢配置）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LlmConfig {
    /// "debug" = DebugAgent（纯 mock 零逻辑，沉默/脚本闭包决策源）；其他值 = providers 里的 key
    pub active: String,
    #[serde(default)]
    pub providers: std::collections::HashMap<String, LlmProvider>,
}

/// 一个 OpenAI 兼容端点 profile；key 本体只在环境变量里，这里只存变量名
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LlmProvider {
    pub base_url: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// 模型上下文窗口（真值 token，事实非策略，#16）：ds-v4-flash 1M、sonnet 200K、gpt 400K。
    /// Compression 触发 = 真值+增量 > context_window − reserve；**None = 不压缩**（显式不猜）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<usize>,
    /// 给输出预留的空间（触发点 = window − reserve）。None → 10_000
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_reserve: Option<usize>,
    /// effort wire 方言声明：该端点的 thinking 参数形态——
    /// "openai" = 顶层 reasoning_effort；"deepseek" = thinking.reasoning_effort。
    /// None = 未声明/不支持：effort 忽略不发送（就近归并 + 告警，绝不塞陌生参数）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_wire: Option<String>,
}

/// Effort 配置：三个直接值预置；
/// 未显式列出的来源一律使用 default
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EffortConfig {
    /// user_chat 档位（缺省 low：用户此刻盯着 pet 等回复）。
    /// 可选叶不落盘（缺省即未设，reconcile 不报噪音）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_chat: Option<crate::llm::Effort>,
    /// hook_stop_content 档位（缺省 high：有实质内容需要仔细读、判断）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_stop_content: Option<crate::llm::Effort>,
    /// 其余来源档位（缺省 medium）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<crate::llm::Effort>,
    /// 匹配关键词表：user_chat 消息命中关键词 → 本次 effort 临时改写为对应档位；
    /// 多命中取最长关键词（确定性）
    #[serde(default = "default_effort_keywords")]
    pub keywords: std::collections::HashMap<String, crate::llm::Effort>,
}

fn default_effort_keywords() -> std::collections::HashMap<String, crate::llm::Effort> {
    // 文档示例语义
    std::collections::HashMap::from([
        ("仔细想想".to_string(), crate::llm::Effort::High),
        ("快点".to_string(), crate::llm::Effort::Low),
    ])
}

impl Default for EffortConfig {
    fn default() -> Self {
        Self {
            user_chat: None,
            hook_stop_content: None,
            default: None,
            keywords: default_effort_keywords(),
        }
    }
}

impl Default for LlmConfig {
    /// 公开厂商预设（首次启动写盘后可自由增删；内部网关只进本地 config.json，不进代码）
    fn default() -> Self {
        let mut providers = std::collections::HashMap::new();
        // (name, base_url, model, key_env, context_window——模型窗口事实，#16, effort_wire——
        // 方言只给已确认值；未确认的留 None = 不发送+告警)
        for (name, base_url, model, key_env, window, effort_wire) in [
            ("deepseek", "https://api.deepseek.com", "deepseek-chat", "DEEPSEEK_API_KEY", 128_000, Some("deepseek")),
            ("moonshot", "https://api.moonshot.cn/v1", "kimi-k2", "MOONSHOT_API_KEY", 256_000, None),
            ("zhipu", "https://open.bigmodel.cn/api/paas/v4", "glm-4-flash", "ZHIPU_API_KEY", 128_000, None),
            ("openai", "https://api.openai.com/v1", "gpt-4o-mini", "OPENAI_API_KEY", 128_000, Some("openai")),
            ("ollama", "http://localhost:11434/v1", "qwen3", "", 32_000, None),
        ] {
            providers.insert(
                name.to_string(),
                LlmProvider {
                    base_url: base_url.into(),
                    model: model.into(),
                    api_key_env: if key_env.is_empty() {
                        None
                    } else {
                        Some(key_env.into())
                    },
                    temperature: Some(0.3),
                    context_window: Some(window),
                    compression_reserve: None,
                    effort_wire: effort_wire.map(String::from),
                },
            );
        }
        Self {
            active: "debug".into(),
            providers,
        }
    }
}

fn default_view_scale() -> f64 {
    1.0
}

fn default_badge_style() -> String {
    "number".into()
}

fn default_badge_side() -> String {
    "right".into()
}

/// 默认主题名：内置深色（空覆写 = styles.css :root 值即 dark 主题）
fn default_theme() -> String {
    "dark".into()
}

/// 主题表语义 default（map 字段必须声明自身 default）：仅内置 dark
fn default_themes() -> std::collections::HashMap<String, std::collections::HashMap<String, String>> {
    let mut m = std::collections::HashMap::new();
    m.insert("dark".into(), std::collections::HashMap::new());
    m
}

/// 0.1.0 支持的语言集合
pub const SUPPORTED_LANGUAGES: &[&str] = &["zh", "en"];
/// 项目明确的默认语言（系统语言不受支持时的回退；Harness 首启默认）
pub const PROJECT_DEFAULT_LANGUAGE: &str = "zh";

/// UI 语言 default：跟随受支持的系统语言；不受支持回退项目默认。
/// 只在首次初始化（或旧配置缺字段的 reconcile）求值一次，之后是稳定用户偏好
fn default_ui_language() -> String {
    let locale = sys_locale::get_locale().unwrap_or_default();
    let lang = locale
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if SUPPORTED_LANGUAGES.contains(&lang.as_str()) {
        lang
    } else {
        PROJECT_DEFAULT_LANGUAGE.into()
    }
}

/// Harness 语言 default：项目明确默认，不随系统语言自动改变
fn default_harness_language() -> String {
    PROJECT_DEFAULT_LANGUAGE.into()
}

/// pet 正式默认名：Ambery——不按语言区分
pub fn default_pet_name() -> String {
    "Ambery".into()
}

/// Compression 保留目标默认：24
fn default_keep_recent() -> usize {
    24
}

/// pet 名称校验：非空、去空白后 ≤ 64 字符
pub fn validate_pet_name(name: &str) -> Vec<String> {
    let t = name.trim();
    if t.is_empty() {
        return vec!["pet 名称不能为空".into()];
    }
    if t.chars().count() > 64 {
        return vec!["pet 名称过长（≤ 64 字符）".into()];
    }
    vec![]
}

/// 主题 token 校验：token 名去 --ov- 前缀后须 `^[a-z][a-z0-9-]*$`；
/// CSS 值拒绝结构字符（`;{}<>` 与引号外注入面），空值拒绝。
/// 主题名本身走动态 map key grammar（valid_dynamic_key）。返回 message 列表（空 = 通过）
pub fn validate_theme_table(
    themes: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
) -> Vec<String> {
    let mut errors = Vec::new();
    for (name, table) in themes {
        if !valid_dynamic_key(name) {
            errors.push(format!("主题名不符合 path grammar（小写字母开头，仅小写字母/数字/_/-）：{name}"));
        }
        for (token, value) in table {
            let mut chars = token.chars();
            let head_ok = matches!(chars.next(), Some(c) if c.is_ascii_lowercase());
            if !head_ok || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                errors.push(format!("主题 {name} 的 token 名须 ^[a-z][a-z0-9-]*$：{token}"));
            }
            if value.is_empty() || value.contains([';', '{', '}', '<', '>']) {
                errors.push(format!("主题 {name} 的 token {token} 值含非法字符或为空"));
            }
        }
    }
    errors.sort();
    errors
}

fn default_compression_reserve() -> usize {
    10_000
}

fn default_ttl_ms() -> u64 {
    60_000 // ttlMs 省略时默认 60000ms
}

fn default_timer_interval() -> i64 {
    300_000 // 5 分钟
}

fn default_timer_stagger() -> i64 {
    30_000
}

fn default_timer_tick() -> u64 {
    60_000
}

fn default_timer_batch() -> usize {
    2
}

fn default_stop_hook_mode() -> String {
    "queue_only".into()
}

fn default_max_tool_calls_in_one_response() -> usize {
    10
}

fn default_max_tool_calls_per_turn() -> usize {
    50
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KaomojiEntry {
    pub face: String,
    pub motion: String,
}

/// 表情两池：系统池 + 用户池。
/// 区别只在归属与尺寸扫描来源（系统池），不在访问权限。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KaomojiConfig {
    /// 系统池：系统状态表情；pet 窗口尺寸扫描只扫描此池。
    /// 默认不要修改
    #[serde(default = "default_system_pool")]
    pub system: std::collections::HashMap<String, KaomojiEntry>,
    /// 用户池：用户自定义表情；初始可为空
    #[serde(default)]
    pub user: std::collections::HashMap<String, KaomojiEntry>,
}

impl Default for KaomojiConfig {
    fn default() -> Self {
        Self {
            system: default_system_pool(),
            user: std::collections::HashMap::new(),
        }
    }
}

/// 系统池语义 default（map 字段必须声明自身 default）
fn default_system_pool() -> std::collections::HashMap<String, KaomojiEntry> {
    let mut system = std::collections::HashMap::new();
    system.insert(
        "idle".into(),
        KaomojiEntry {
            face: "(´ω`)".into(),
            motion: "still".into(),
        },
    );
    system.insert(
        "processing".into(),
        KaomojiEntry {
            face: "(ˇωˇ」∠)_".into(),
            motion: "float".into(),
        },
    );
    system.insert(
        "notify".into(),
        KaomojiEntry {
            face: "✧*｡٩(ˊᗜˋ*)و✧*｡".into(),
            motion: "bounce".into(),
        },
    );
    system
}

/// 动态 map key 的运行时 grammar 检查：
/// `^[a-z][a-z0-9_-]*$`——加载 reconcile、两池校验与写入管道共用同一份
pub fn valid_dynamic_key(k: &str) -> bool {
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// 两池校验（validate_kaomoji_pools 的两个不变量 +
/// 动态 key grammar）：keys(system) ∩ keys(user) = ∅；{ idle, processing, notify }
/// ⊆ keys(system) ∪ keys(user)；池内 key 全部符合 path grammar。
/// 返回 message 列表（空 = 通过）；path 前缀由调用方补。
pub fn validate_kaomoji_pools(pools: &KaomojiConfig) -> Vec<String> {
    let mut errors = Vec::new();
    let mut dup: Vec<_> = pools
        .system
        .keys()
        .filter(|k| pools.user.contains_key(*k))
        .cloned()
        .collect();
    dup.sort();
    if !dup.is_empty() {
        errors.push(format!(
            "两池 key 必须全局唯一，system 与 user 重复：{}",
            dup.join(", ")
        ));
    }
    let missing: Vec<_> = ["idle", "processing", "notify"]
        .iter()
        .filter(|k| !pools.system.contains_key(**k) && !pools.user.contains_key(**k))
        .collect();
    if !missing.is_empty() {
        errors.push(format!(
            "基础状态 key 必须存在于两池并集，缺：{}",
            missing
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut bad: Vec<_> = pools
        .system
        .keys()
        .chain(pools.user.keys())
        .filter(|k| !valid_dynamic_key(k))
        .cloned()
        .collect();
    bad.sort();
    if !bad.is_empty() {
        errors.push(format!(
            "表情 key 不符合 path grammar（小写字母开头，仅小写字母/数字/_/-）：{}",
            bad.join(", ")
        ));
    }
    errors
}

impl Config {
    /// 两池并集按 key 解析（默认状态与 set_autonomy(key) 共用）。
    /// 校验保证不相交，顺序无歧义；约定 system 先查（确定性）。
    pub fn kaomoji_resolve(&self, key: &str) -> Option<&KaomojiEntry> {
        self.kaomoji.system.get(key).or_else(|| self.kaomoji.user.get(key))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kaomoji: KaomojiConfig::default(),
            compression_reserve_default: default_compression_reserve(),
            set_autonomy_default_ttl_ms: default_ttl_ms(),
            timer: TimerConfig::default(),
            terminal: TerminalConfig::default(),
            stop_hook_mode: default_stop_hook_mode(),
            max_tool_calls_in_one_response: default_max_tool_calls_in_one_response(),
            max_tool_calls_per_turn: default_max_tool_calls_per_turn(),
            // {name} 占位：拼装 system prompt 时替换为当前 pet 名称（改名不回写历史/已生成内容，但请求头拼装跟当前名）
            base_prompt:
                "你是 {name}，Ambery 的看板宠物。根据系统状态决定通知或沉默，用 tool_calls 行动。"
                    .into(),
            view_scale: default_view_scale(),
            badge_style: default_badge_style(),
            badge_side: default_badge_side(),
            theme: default_theme(),
            themes: default_themes(),
            ui_language: default_ui_language(),
            harness_language: default_harness_language(),
            name: default_pet_name(),
            context_compression_keep_recent_messages: default_keep_recent(),
            llm: LlmConfig::default(),
            effort: EffortConfig::default(),
            read_only: false,
            load_report: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_adapter_defaults_and_compat() {
        let cfg = Config::default();
        assert!(cfg.terminal.adapter_wt, "wt 默认启用（保持既有自动发现行为）");
        assert!(!cfg.terminal.adapter_zellij, "zellij 默认关闭");
        // 旧 config.json 无 terminal 段 → serde default 补齐（无需迁移）
        let mut v = serde_json::to_value(&cfg).unwrap();
        v.as_object_mut().unwrap().remove("terminal");
        let cfg2: Config = serde_json::from_value(v).unwrap();
        assert_eq!(cfg2.terminal, cfg.terminal);
    }

    #[test]
    fn effective_limit_window_minus_reserve() {
        let mut cfg = Config::default();
        // active=debug 无 provider → None = 不压缩
        assert_eq!(cfg.effective_compression_limit(), None);
        // moonshot preset: 256K − 默认 reserve 10K
        cfg.llm.active = "moonshot".into();
        assert_eq!(cfg.effective_compression_limit(), Some(246_000));
        // provider 覆盖 reserve
        cfg.llm.providers.get_mut("moonshot").unwrap().compression_reserve = Some(6_000);
        assert_eq!(cfg.effective_compression_limit(), Some(250_000));
        // window 缺省 → None
        cfg.llm.providers.get_mut("moonshot").unwrap().context_window = None;
        assert_eq!(cfg.effective_compression_limit(), None);
        // window < reserve → 饱和 0（永远触发）
        cfg.llm.providers.get_mut("moonshot").unwrap().context_window = Some(100);
        assert_eq!(cfg.effective_compression_limit(), Some(0));
    }
}
impl Config {
    /// Compression 触发上限（#16，唯一出口）：active provider 的 context_window − reserve
    /// （reserve = provider 覆盖值，缺省用 compression_reserve_default）。
    /// **None = 不压缩**（无窗口事实时显式不猜）
    pub fn effective_compression_limit(&self) -> Option<usize> {
        let p = self.llm.providers.get(&self.llm.active)?;
        let reserve = p
            .compression_reserve
            .unwrap_or(self.compression_reserve_default);
        p.context_window.map(|w| w.saturating_sub(reserve))
    }

    /// 读配置：版本与迁移加载管线（config/migrate.rs）；
    /// 文件不存在 → 写入默认配置（首次启动落地，用户可直接编辑）
    pub fn load_or_default(dir: &std::path::Path) -> Self {
        migrate::load(dir)
    }

    /// 持久化（注入 version 控制字段 = current）；只读降级模式 → 报错。
    /// 原子写（tmp + rename）：磁盘只保留旧完整文件或新完整文件（§update 与统一管道），外部自动载入不会读到半截文件
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        if self.read_only {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "只读降级模式：config 写被禁止",
            ));
        }
        std::fs::create_dir_all(dir)?;
        let mut v = serde_json::to_value(self).map_err(std::io::Error::other)?;
        v["version"] = serde_json::Value::from(migrate::CURRENT_VERSION);
        let s = serde_json::to_string_pretty(&v).map_err(std::io::Error::other)?;
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(&tmp, dir.join(CONFIG_FILE))?;
        Ok(())
    }
}
