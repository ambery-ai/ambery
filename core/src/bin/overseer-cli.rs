//! overseer-cli（docs/config.md「修改入口」）：Config 的声明式 CLI。
//! 零 per-field 代码——list/get/set/schema 四命令全是 schema 节点的薄渲染。
//! 默认走 HTTP（热生效 + 广播）；--offline 直写文件兜底（server 未运行时使用）。

use clap::{Parser, Subcommand};
use overseer_core::config::reflect;
use overseer_core::Config;
use serde_json::Value;

const DEFAULT_BASE: &str = "http://127.0.0.1:47600";

#[derive(Parser)]
#[command(name = "overseer-cli", about = "Terminal Overseer 配置 CLI（docs/config.md）")]
struct Cli {
    /// 直写 config.json（server 未运行时；无热生效/广播）
    #[arg(long, global = true)]
    offline: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 列出所有配置节点：path = 当前值 [类型] 说明
    List,
    /// 原始 schema 节点 JSON（CLI/面板的调试视图）
    Schema,
    /// 读单个值
    Get { path: String },
    /// 改单个值（value 先按 JSON 解析，失败按字符串）
    Set { path: String, value: String },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

async fn run(cli: Cli) -> Result<String, String> {
    if cli.offline {
        run_offline(cli.cmd)
    } else {
        run_online(cli.cmd).await
    }
}

// ---------- online（HTTP 薄客户端，与托盘面板同一数据源） ----------

fn base() -> String {
    std::env::var("OVERSEER_ADDR").unwrap_or_else(|_| DEFAULT_BASE.into())
}

async fn fetch_schema(client: &reqwest::Client) -> Result<Value, String> {
    let url = format!("{}/config/schema", base());
    let resp: Value = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("连不上 server（{e}）——server 未运行时用 --offline"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if resp["readOnly"].as_bool() == Some(true) {
        eprintln!("warning: server 处于只读降级模式，set 会被拒绝");
    }
    Ok(resp)
}

async fn run_online(cmd: Cmd) -> Result<String, String> {
    let client = reqwest::Client::new();
    match cmd {
        Cmd::Schema => {
            let s = fetch_schema(&client).await?;
            Ok(serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?)
        }
        Cmd::List => {
            let s = fetch_schema(&client).await?;
            Ok(format_nodes(s["nodes"].as_array().ok_or("nodes 缺失")?))
        }
        Cmd::Get { path } => {
            let s = fetch_schema(&client).await?;
            let nodes = s["nodes"].as_array().ok_or("nodes 缺失")?;
            let n = nodes
                .iter()
                .find(|n| n["path"].as_str() == Some(&path))
                .ok_or_else(|| format!("无此节点: {path}"))?;
            Ok(serde_json::to_string_pretty(&n["value"]).map_err(|e| e.to_string())?)
        }
        Cmd::Set { path, value } => {
            let body = serde_json::json!({ "path": path, "value": parse_value(&value) });
            let resp: Value = client
                .post(format!("{}/config", base()))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("连不上 server（{e}）"))?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            if resp["ok"].as_bool() == Some(true) {
                let mut out = format!("ok: {path} 已更新（热生效）");
                if let Some(rr) = resp["restartRequired"].as_array() {
                    if !rr.is_empty() {
                        out.push_str(&format!("\nwarning: {rr:?} 需重启生效"));
                    }
                }
                Ok(out)
            } else {
                Err(format!("被拒绝: {}", resp["error"].as_str().unwrap_or("?")))
            }
        }
    }
}

// ---------- offline（直写文件兜底） ----------

fn run_offline(cmd: Cmd) -> Result<String, String> {
    let dir = overseer_core::paths::config_root();
    let cfg = Config::load_or_default(&dir);
    match cmd {
        Cmd::Schema => {
            let nodes = reflect::config_nodes(&cfg);
            Ok(serde_json::to_string_pretty(&nodes).map_err(|e| e.to_string())?)
        }
        Cmd::List => {
            let v = serde_json::to_value(reflect::config_nodes(&cfg)).map_err(|e| e.to_string())?;
            Ok(format_nodes(v.as_array().ok_or("nodes 序列化异常")?))
        }
        Cmd::Get { path } => {
            let v = serde_json::to_value(&cfg).map_err(|e| e.to_string())?;
            let mut cur = &v;
            for seg in path.split('.') {
                cur = cur
                    .get(seg)
                    .ok_or_else(|| format!("无此路径: {path}（止于 {seg}）"))?;
            }
            Ok(serde_json::to_string_pretty(cur).map_err(|e| e.to_string())?)
        }
        Cmd::Set { path, value } => {
            let value = parse_value(&value);
            let mut v = serde_json::to_value(&cfg).map_err(|e| e.to_string())?;
            reflect::set_by_path(&mut v, &path, value.clone())?;
            let new: Config =
                serde_json::from_value(v).map_err(|e| format!("验证失败: {e}"))?;
            // 统一 validation（docs/config.md §统一修改入口：验证只能有一份——
            // offline 直写同样跑 meta validators，原子拒绝）
            let verrs = overseer_core::config::meta::validate_for_update(
                &serde_json::to_value(&new).map_err(|e| e.to_string())?,
                &path,
            );
            if !verrs.is_empty() {
                return Err(format!(
                    "验证失败: {}",
                    verrs.iter().map(|(p, m)| format!("{p}: {m}")).collect::<Vec<_>>().join("；")
                ));
            }
            if let (Some(opts), Value::String(s)) = (reflect::valid_options(&new, &path), &value) {
                if !opts.contains(s) {
                    return Err(format!("{path}: '{s}' 不在合法选项 {opts:?} 中"));
                }
            }
            new.save(&dir).map_err(|e| format!("写入失败: {e}"))?;
            Ok(format!("ok: {path} 已写入 {}（server 重启/热加载后生效）", dir.join("config.json").display()))
        }
    }
}

// ---------- 共用 ----------

/// value 解析：先 JSON（5000/true/{...}/[...]/"..."），失败按裸字符串
fn parse_value(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

fn format_nodes(nodes: &[Value]) -> String {
    let mut out = String::new();
    for n in nodes {
        let path = n["path"].as_str().unwrap_or("?");
        let kind = n["type"]["kind"].as_str().unwrap_or("?");
        let value = serde_json::to_string(&n["value"]).unwrap_or_default();
        let value = if value.chars().count() > 60 {
            format!("{}…", value.chars().take(60).collect::<String>())
        } else {
            value
        };
        let desc = n["desc"].as_str().unwrap_or("");
        out.push_str(&format!("{path} = {value}  [{kind}]  {desc}\n"));
    }
    out
}
