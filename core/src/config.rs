//! Config 域（concepts §12，docs/config.md）：类型 + load/save。
//! 子模块：reflect（声明式 UI 反射）、migrate（版本与迁移加载管线）。

pub mod migrate;
pub mod reflect;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CONFIG_FILE: &str = "config.json";

/// Config（concepts §12）：持久化单文件 config.json，edit_config tool 可写
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    /// 状态 key → 颜文字映射（Autonomy 默认行为表，concepts §4）
    pub kaomoji: std::collections::HashMap<String, KaomojiEntry>,
    /// Compression 触发阈值——**未知模型 fallback**（concepts §10d，#16）。
    /// 分模型标定见 `llm.providers.*.token_threshold`；生效值取 `effective_token_threshold()`
    pub token_threshold: usize,
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
    /// Compression 触发阈值（真值 token，按模型窗口 ~80% 标定，#16）。
    /// None → 全局 `token_threshold`（fallback）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_threshold: Option<usize>,
}

impl Default for LlmConfig {
    /// 公开厂商预设（首次启动写盘后可自由增删；内部网关只进本地 config.json，不进代码）
    fn default() -> Self {
        let mut providers = std::collections::HashMap::new();
        // (name, base_url, model, key_env, token_threshold preset——按模型窗口 ~80%，#16)
        for (name, base_url, model, key_env, threshold) in [
            ("deepseek", "https://api.deepseek.com", "deepseek-chat", "DEEPSEEK_API_KEY", 100_000),
            ("moonshot", "https://api.moonshot.cn/v1", "kimi-k2", "MOONSHOT_API_KEY", 200_000),
            ("zhipu", "https://open.bigmodel.cn/api/paas/v4", "glm-4-flash", "ZHIPU_API_KEY", 100_000),
            ("openai", "https://api.openai.com/v1", "gpt-4o-mini", "OPENAI_API_KEY", 100_000),
            ("ollama", "http://localhost:11434/v1", "qwen3", "", 24_000),
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
                    token_threshold: Some(threshold),
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

impl Default for Config {
    fn default() -> Self {
        let mut kaomoji = std::collections::HashMap::new();
        kaomoji.insert(
            "idle".into(),
            KaomojiEntry {
                face: "(´ω`)".into(),
                motion: "still".into(),
            },
        );
        kaomoji.insert(
            "processing".into(),
            KaomojiEntry {
                face: "(ˇωˇ」∠)_".into(),
                motion: "float".into(),
            },
        );
        kaomoji.insert(
            "notify".into(),
            KaomojiEntry {
                face: "✧*｡٩(ˊᗜˋ*)و✧*｡".into(),
                motion: "bounce".into(),
            },
        );
        Self {
            kaomoji,
            token_threshold: 8000,
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
    fn effective_threshold_resolution_order() {
        let mut cfg = Config::default(); // fallback = 8000
        assert_eq!(cfg.effective_token_threshold(), 8000); // active=debug 无 provider → fallback
        // provider 有值 → 分模型值胜
        cfg.llm.active = "moonshot".into();
        assert_eq!(cfg.effective_token_threshold(), 200_000);
        // provider 值为 None → fallback
        cfg.llm.providers.get_mut("moonshot").unwrap().token_threshold = None;
        assert_eq!(cfg.effective_token_threshold(), 8000);
        // active 未知 → fallback
        cfg.llm.active = "no-such".into();
        assert_eq!(cfg.effective_token_threshold(), 8000);
    }
}
impl Config {
    /// Compression 生效阈值（#16，唯一出口）：active provider 的分模型值，无则全局 fallback
    pub fn effective_token_threshold(&self) -> usize {
        self.llm
            .providers
            .get(&self.llm.active)
            .and_then(|p| p.token_threshold)
            .unwrap_or(self.token_threshold)
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
