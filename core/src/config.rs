//! Config 域（concepts §12，docs/config.md）：类型 + load/save。
//! 子模块：reflect（声明式 UI 反射）、migrate（版本与迁移加载管线）、
//! meta（字段行为元数据注册表：validation / no_llm_visible / 冷字段）。

pub mod meta;
pub mod migrate;
pub mod reflect;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CONFIG_FILE: &str = "config.json";

/// Config（concepts §12）：持久化单文件 config.json，edit_config tool 可写
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    /// 表情领域（docs/config.md §表情池）：两个固定池；池内表情名称是动态 map key。
    /// 两池 key 全局唯一（validate_kaomoji_pools 保证不相交），无隐式优先级
    #[serde(default)]
    pub kaomoji: KaomojiConfig,
    /// Compression 输出预留默认值（#16）：触发点 = context_window − reserve，
    /// provider 未设 `compression_reserve` 时用此值
    #[serde(default = "default_compression_reserve")]
    pub compression_reserve_default: usize,
    /// set_autonomy 省略 ttlMs 时的默认值（docs/autonomy.md）
    #[serde(default = "default_ttl_ms")]
    pub set_autonomy_default_ttl_ms: u64,
    /// Filter 策略名（concepts §11/§12，docs/filter.md）
    #[serde(default = "default_filter_strategy")]
    pub filter_strategy: String,
    /// Timer 兜底扫描间隔（concepts §1a，docs/timer.md）；≤ 0 = 整体禁用
    #[serde(default = "default_timer_interval")]
    pub timer_interval_ms: i64,
    /// Timer 错峰窗口（concepts §1a「错峰分布偏移量」）
    #[serde(default = "default_timer_stagger")]
    pub timer_stagger_ms: i64,
    /// Timer 主循环粒度（docs/timer.md；interval 小于它也最多每 tick 一扫）
    #[serde(default = "default_timer_tick")]
    pub timer_tick_ms: u64,
    /// Timer 每 tick 最多扫描实例数（限流，docs/timer.md）
    #[serde(default = "default_timer_batch")]
    pub timer_batch: usize,
    /// stop hook 模式（docs/hook.md §stop 三模式）：queue_only（默认，hint 按需读）/ auto_read / message
    #[serde(default = "default_stop_hook_mode")]
    pub stop_hook_mode: String,
    /// system prompt 基座（运行时与 kaomoji 表、顶层状态拼装，concepts §12）
    pub base_prompt: String,
    /// View 缩放（concepts §3，球场圆形默认 0.5）
    #[serde(default = "default_view_scale")]
    pub view_scale: f64,
    /// 未读角标样式（concepts §3a）：number（纯数字，默认）/ bubble（气泡）
    #[serde(default = "default_badge_style")]
    pub badge_style: String,
    /// 未读角标方位：right（正右边，默认）/ left
    #[serde(default = "default_badge_side")]
    pub badge_side: String,
    /// LLM 多 profile 配置（docs/agent-loop.md §LLM 抽象）
    #[serde(default)]
    pub llm: LlmConfig,
    /// 只读降级模式（docs/config.md 降级路径）：true 时任何 save 报错。
    /// 运行时标记，不落盘（serde skip）
    #[serde(skip)]
    pub read_only: bool,
    /// 加载管线报告（迁移/reconcile/降级每个动作一行，docs/config.md「上报」）。
    /// 运行时数据，不落盘（serde skip）
    #[serde(skip)]
    pub load_report: Vec<String>,
}

/// LLM 配置 v2：多 provider profile + active 选择器（切换不丢配置）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LlmConfig {
    /// "debug" = DebugAgent（内置规则）；其他值 = providers 里的 key
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
}

impl Default for LlmConfig {
    /// 公开厂商预设（首次启动写盘后可自由增删；内部网关只进本地 config.json，不进代码）
    fn default() -> Self {
        let mut providers = std::collections::HashMap::new();
        // (name, base_url, model, key_env, context_window——模型窗口事实，#16)
        for (name, base_url, model, key_env, window) in [
            ("deepseek", "https://api.deepseek.com", "deepseek-chat", "DEEPSEEK_API_KEY", 128_000),
            ("moonshot", "https://api.moonshot.cn/v1", "kimi-k2", "MOONSHOT_API_KEY", 256_000),
            ("zhipu", "https://open.bigmodel.cn/api/paas/v4", "glm-4-flash", "ZHIPU_API_KEY", 128_000),
            ("openai", "https://api.openai.com/v1", "gpt-4o-mini", "OPENAI_API_KEY", 128_000),
            ("ollama", "http://localhost:11434/v1", "qwen3", "", 32_000),
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

fn default_compression_reserve() -> usize {
    10_000
}

fn default_ttl_ms() -> u64 {
    5000
}

fn default_filter_strategy() -> String {
    "default".into()
}

fn default_timer_interval() -> i64 {
    300_000 // 5 分钟（concepts §1a）
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KaomojiEntry {
    pub face: String,
    pub motion: String,
}

/// 表情两池（docs/config.md §表情池）：系统池 + 用户池。
/// 区别只在归属与尺寸扫描来源（系统池），不在访问权限。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KaomojiConfig {
    /// 系统池：系统状态表情；pet 窗口尺寸扫描只扫描此池（docs/pet-window-size.md）。
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

/// 系统池语义 default（map 字段必须声明自身 default，docs/config.md）
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

/// 两池校验（docs/config.md §表情池，validate_kaomoji_pools 的两个不变量）：
/// keys(system) ∩ keys(user) = ∅；{ idle, processing, notify } ⊆ keys(system) ∪ keys(user)。
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
    errors
}

impl Config {
    /// 两池并集按 key 解析（docs/autonomy.md：默认状态与 set_autonomy(key) 共用）。
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
            filter_strategy: default_filter_strategy(),
            timer_interval_ms: default_timer_interval(),
            timer_stagger_ms: default_timer_stagger(),
            timer_tick_ms: default_timer_tick(),
            timer_batch: default_timer_batch(),
            stop_hook_mode: default_stop_hook_mode(),
            base_prompt:
                "你是ペット，Terminal Overseer 的看板宠物。根据系统状态决定通知或沉默，用 tool_calls 行动。"
                    .into(),
            view_scale: default_view_scale(),
            badge_style: default_badge_style(),
            badge_side: default_badge_side(),
            llm: LlmConfig::default(),
            read_only: false,
            load_report: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 读配置：版本与迁移加载管线（docs/config.md，config/migrate.rs）；
    /// 文件不存在 → 写入默认配置（首次启动落地，用户可直接编辑）
    pub fn load_or_default(dir: &std::path::Path) -> Self {
        migrate::load(dir)
    }

    /// 持久化（注入 version 控制字段 = current）；只读降级模式 → 报错
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        if self.read_only {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "只读降级模式：config 写被禁止（docs/config.md）",
            ));
        }
        std::fs::create_dir_all(dir)?;
        let mut v = serde_json::to_value(self).map_err(std::io::Error::other)?;
        v["version"] = serde_json::Value::from(migrate::CURRENT_VERSION);
        let s = serde_json::to_string_pretty(&v).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(CONFIG_FILE), s)
    }
}
