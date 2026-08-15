# DebugAgent — 纯 Mock 与 HTTP brain

> 本文档定义 debug 规则。

## 定位（设计决定）

DebugAgent 是**纯 mock，零逻辑**。它不做任何判断——不解析消息、不设阈值、不挑实例。
每次 `complete()` 把全量 Context 交给**外部注入的决策源**，原样返回决策源给的 `LlmOutput`。

判断逻辑不属于 mock——它要么属于真实 LLM，要么属于调试者本人。

## 三种决策源

| 来源 | 构造 | 用途 |
|---|---|---|
| 沉默 | `DebugAgent::silent()`（Default） | OpenAi 失败降级兜底；不需要反应的测试 |
| 脚本闭包 | `DebugAgent::new(move \|msgs\| …)` | 测试：按调用序弹出预定返回（ambery.rs 测试的 `scripted()` 辅助函数） |
| HTTP brain | `debug_brain.py`（OpenAI 兼容 `/chat/completions`） | 人/外部脚本当 LLM，手动驱动真实 Harness 链路（case-runner debug 模式 `--brain-addr` 连） |

`LlmBackend::from_config` 的 debug 分支默认沉默；case-runner 检测到
Debug 变体时按 debug 模式参数换成 HTTP brain 或沉默。Tauri 内嵌 core 无控制台，
保持沉默兜底（诚实降级：不再假装会判断）。

## scripts/debug_brain.py — HTTP LLM 替换示例

本地 OpenAI 兼容 `/chat/completions` HTTP 服务器，是「LLM 替换」的最小示例：
决策源逻辑内置在脚本里，case-runner 以 debug 模式 `--brain-addr` 连它当 LLM 用
（docs/case-runner.md §用例）。

```bash
python scripts/debug_brain.py --port 47777   # 起 HTTP 服务（--port 可选，默认 47777）
```

内置最小阈值决策源：请求内容含「完成，Context 已更新（N 字）」且 N ≥ 80 → 回通知 tool；
否则沉默。实现为 OpenAI 兼容响应，case-runner 走 OpenAiClient 通用路径调用。
