# LLM 配置引导与连接错误

[English](llm-setup.md) | 中文

> 本文档定义首启 LLM 配置引导：未配置默认态、引导 modal（与设置面板相同的渲染方式——Config schema 节点的反射）、连通测试能力。错误呈现遵循 [errors.md](errors.md)。

## 概念

- **未配置态**——`llm.active` 的默认值是 `"unconfigured"`。`llm.active` 处于未配置态时触发配置引导。
- **引导 modal**——未配置时从 Chat 弹出的 modal。它渲染 Config schema 节点，与设置面板（menu）相同的方式——同一 `config_nodes` 投影与机械渲染（`get_config_schema` → 节点 → 控件）。不是手写表单；字段随 schema 自动出现/消失。menu 本身不变：未配置态不改变任何 menu 行为。
- **连通测试**——后端新能力：按 active provider 构建一次 `complete` 调用，返回成功或具体失败原因。
- **应用级 env 层**——provider 凭据的存储位。`~/.config/ambery/env`（0600）存 `KEY=value` 行；应用解析环境变量时**先 env 文件、后进程环境**（文件覆盖系统）。key 本体永不进 `config.json`。

## 原则

> **未配置是诚实的默认**——全新安装没有 LLM 配置；默认态必须如实说明。

> **引导 modal 是反射**——它渲染 schema 节点而非自定义 UI，与设置面板同一套渲染。一份渲染，无手写表单。

> **menu 不变**——引导不在 menu 里，也不改变 menu 行为；它是从 Chat 触达的 modal。

> **失败绝不静默**——动作失败（LLM 调用、key 写入）用户必须可见；引导流绝不隐藏失败。

> **key 不进 config**——`config.json` 只存环境变量*名*（`api_key_env`）；key 本体在应用级 env 层或进程环境中。引导 modal 可以*输入* key：写入 env 文件。`config.json` 永不包含 key 值。

## key 存储模型（应用级 env 层）

env 文件 `~/.config/ambery/env` 是**应用级环境变量层**：

- 格式：每行 `KEY=value`；允许空行与 `#` 注释。
- 权限：`0600`——用户秘密文件。
- 解析顺序：**env 文件 → 进程环境**（先命中者胜）。文件*覆盖*系统，不是第二个命名空间。
- 变量名：统一 `AMBERY_<PROVIDER>_API_KEY`（如 `AMBERY_DEEPSEEK_API_KEY`）。UI 写入某 provider 的 key 时，若其 `api_key_env` 不同（旧名如 `DEEPSEEK_API_KEY`）或为空，写操作同时把 `api_key_env` 归一为统一名（只动 config 字段，绝不写 key 值）。这是隐式单向迁移，无独立迁移步骤。
- 读取路径：`LlmBackend::from_config` 通过应用级层解析 `api_key_env`（先 env 文件，后 `std::env::var`）。无 `api_key_env` 的 provider（本地端点如 ollama/brain）无需 key。

为什么是这个形态：从 Finder/Dock 启动的 GUI 应用**不继承 shell profile 的 export**（环境由 launchd 提供），只靠 shell 设 key 对安装版不可用。env 文件给 key 一个与 shell 无关的家，同时保持 `config.json` 零 key。

## 触发模型

| 触发 | 条件 | 动作 |
|---|---|---|
| 应用启动 | `llm.active` == 未配置值 | 标记"未配置" |
| Chat 打开 | 未配置 | Chat 显示引导 modal + 横幅提示 |
| Chat 发送 | LLM 调用失败 | 按 [errors.md](errors.md) 出错误通知；banner 的 `setup` action 打开本 modal |
| banner 动作 | `setup` action | 打开引导 modal（未配置 / 连接失败共用同一 modal） |

## 引导 modal

未配置时从 Chat 打开（或从 banner 的"打开配置"动作）。内容：

1. **选择 provider**——渲染 `llm.active` schema 节点（enum select：未配置 / debug / providers）。
2. **provider 字段**——渲染所选 provider 的 schema 节点（`base_url` / `model` / `api_key_env` 等）。
3. **key 输入**——每个 provider 一个密码输入框。本地端点（无 `api_key_env`，如 ollama/brain）显示"无需 key"。状态：
   - **未设置**——env 文件与进程环境都无该 key：输入框警示样式，占位"请输入 API key"。
   - **已设置**——任一来源有 key：占位 `••••••••（已设置，留空则不改动）` + 小字提示"已设置（来源：env 文件 / 环境变量）"，另有**清除**按钮（从 env 文件删除该 key）。
   - **保存中**——写入期间禁用。
   - 判定走与读取相同的解析链（env 文件 → 进程环境），本地即时完成——不依赖 `test_llm` 往返。
4. **保存语义**——留空提交 = 不改动（保留现有 key）；填写提交 = upsert 进 env 文件（统一 `AMBERY_<PROVIDER>_API_KEY` + `api_key_env` 归一）；清除 = 从 env 文件删除。写失败内联报错（绝不静默）。保存/清除后 modal 立即刷新 未设置/已设置 状态，并**自动重跑 `test_llm`**。
5. **连通测试**——调用新的 `test_llm` 后端能力；结果内联显示（成功，或具体失败原因）。

UI 区分两种失败形态：**未设置**（本地存在性检查——输入框警示）vs **已设置但连不上**（连通测试 / chat 错误——错误气泡 + banner）。同一组件同时服务引导 modal 与 menu 设置面板（单一渲染源）。

完成：`llm.active` 不再是未配置值 → modal 不再自动触发。

## 连接错误

错误呈现（气泡 / banner）遵循 [errors.md](errors.md) 的模型。本文档与它的唯一连接点：banner 的 `setup` action 打开上文引导 modal。

## 后端变更

- **未配置默认**——`LlmConfig::default().active` 为 `"unconfigured"`；`LlmBackend::from_config` 将未配置值视为无 provider——配置引导（modal + banner，action `setup`）按 [errors.md](errors.md) 呈现它。**不加迁移**：存量 config 文件保持其当前 `active`；未配置默认只作用于全新安装（已存在的 config 文件永不被改写为新默认）。
- **`test_llm` 能力**——新命令：读 active provider，构建一次 `OpenAiClient`，做一次 `complete` 调用，返回 `{ok, message}` 与具体失败原因。复用现有 provider 构建路径。
- **`set_api_key(provider, Option<key>)`**——`Some` upsert 进 env 文件（统一名 + `api_key_env` 归一），`None` 清除。双通道暴露（Tauri command + HTTP route）；core 函数可单测直调。
- **`get_api_key_status(provider)`**——经 env 文件优先解析链的存在性检查；返回 未设置/已设置 + 来源。本地即时。

## 明确不在范围内

- 在 config 中存储 key（env 文件纪律保持 `config.json` 零 key）。
- 启动时自动检测"env 已设但 key 无效"（那是错误路径，不是引导路径）。
- 连通测试作为周期性健康检查（post-0.1.0）。
- OS 钥匙串集成（post-0.1.0；0600 env 文件是 0.1.0 的答案）。
