# LLM 配置引导与连接错误

[English](llm-setup.md) | 中文

> 本文档定义首启 LLM 配置引导与连接错误提示：未配置默认态、引导 modal（与设置面板相同的渲染方式——Config schema 节点的反射）、连通测试能力，以及 chat 如何报告 LLM 失败。

## 概念

- **未配置态**——`llm.active` 的默认值是 `"unconfigured"`。`llm.active` 处于未配置态时触发配置引导。
- **引导 modal**——未配置时从 Chat 弹出的 modal。它渲染 Config schema 节点，与设置面板（menu）相同的方式——同一 `config_nodes` 投影与机械渲染（`get_config_schema` → 节点 → 控件）。不是手写表单；字段随 schema 自动出现/消失。menu 本身不变：未配置态不改变任何 menu 行为。
- **连通测试**——后端新能力：按 active provider 构建一次 `complete` 调用，返回成功或具体失败原因。

## 原则

> **未配置是诚实的默认**——全新安装没有 LLM 配置；默认态必须如实说明。

> **引导 modal 是反射**——它渲染 schema 节点而非自定义 UI，与设置面板同一套渲染。一份渲染，无手写表单。

> **menu 不变**——引导不在 menu 里，也不改变 menu 行为；它是从 Chat 触达的 modal。

> **失败绝不静默**——LLM 不可达时发送聊天消息必须产生可见错误。现有 `llm_error` effect 已到达前端（今天只有 pet 渲染它）；chat 订阅同一通道。

> **key 不进 config**——config 只存环境变量*名*（`api_key_env`），key 本体在环境中（`std::env::var`）。引导 modal 显示变量名并指引用户设置；绝不存储 key。

## 触发模型

| 触发 | 条件 | 动作 |
|---|---|---|
| 应用启动 | `llm.active` == 未配置值 | 标记"未配置" |
| Chat 打开 | 未配置 | Chat 显示引导 modal + 横幅提示 |
| Chat 发送 | LLM 调用失败 | 消息流错误气泡（区分原因）+ 输入框上方 banner |
| banner 动作 | "打开配置" | 再次打开引导 modal（未配置 / 连接失败是同一 modal 的两种状态） |
| banner 关闭 | 用户关闭 | 仅隐藏当前 banner；下次错误再现 |

## 引导 modal

未配置时从 Chat 打开（或从 banner 的"打开配置"动作）。内容：

1. **选择 provider**——渲染 `llm.active` schema 节点（enum select：未配置 / debug / providers）。
2. **provider 字段**——渲染所选 provider 的 schema 节点（`base_url` / `model` / `api_key_env` 等）。
3. **key 状态**——每个 provider 的 `api_key_env` 是反射字段（每个 provider 都有；`ollama` 无）。modal 将其按**变量名 + 检测状态**展示，而非可编辑输入框：显示变量名及环境变量是否已设（后端检测）。默认预设变量名遵循 `AMBERY_<NAME>_API_KEY` 约定（如 `AMBERY_DEEPSEEK_API_KEY`）；key 本体绝不在此输入——用户在 shell 环境里设置。
4. **连通测试**——调用新的 `test_llm` 后端能力；结果内联显示（成功，或具体失败原因）。

完成：`llm.active` 不再是未配置值 → modal 不再自动触发。

## 连接错误（chat）

- **错误气泡**——LLM 调用失败时，消息流插入气泡。区分原因：网络不可达 / 超时 / 401 key 无效 / 400 参数错误 / 环境变量未设 / provider 缺失。带重试动作。
- **banner**——chat 输入框上方，仅在错误激活时显示。可关闭（仅隐藏当前；下次错误再现）。banner 带"打开配置"动作，重新打开引导 modal。
- **降级回复说明**——失败路径回退 DebugAgent 时，当轮仍产出回复；气泡/banner 必须注明"当前为降级回复（debug 兜底）"，让可见回复与可见错误不矛盾。

## 后端变更

- **未配置默认**——`LlmConfig::default().active` 为 `"unconfigured"`；`LlmBackend::from_config` 将未配置值视为"无 LLM"（静默回退语义，启动时在任何交互前不刷错误卡）。**不加迁移**：存量 config 文件保持其当前 `active`；未配置默认只作用于全新安装（已存在的 config 文件永不被改写为新默认）。
- **`test_llm` 能力**——新命令：读 active provider，构建一次 `OpenAiClient`，做一次 `complete` 调用，返回 `{ok, message}` 与具体失败原因。复用现有 provider 构建路径。

## 明确不在范围内

- 在 config 中存储 key（环境变量纪律不变）。
- 启动时自动检测"env 已设但 key 无效"（那是错误路径，不是引导路径）。
- 连通测试作为周期性健康检查（post-0.1.0）。
