# DebugAgent — Pure Mock and HTTP Brain

English | [中文](debug-agent.zh.md)

> This document defines the debug rules.

## Positioning (Design Decision)

DebugAgent is a **pure mock, zero logic**. It makes no judgments — it does not parse messages, set thresholds, or pick instances.
Every `complete()` hands the full Context to an **externally injected decision source** and returns the `LlmOutput` that source gives, unchanged.

Judgment logic does not belong in the mock — it belongs either to the real LLM or to the debugger.

## Three Decision Sources

| Source | Construction | Purpose |
|---|---|---|
| Silent | `DebugAgent::silent()` (Default) | OpenAi failure fallback; tests that need no reaction |
| Script closure | `DebugAgent::new(move \|msgs\| …)` | Tests: pop predetermined returns by call order (the `scripted()` helper in ambery.rs tests) |
| HTTP brain | `debug_brain.py` (OpenAI-compatible `/chat/completions`) | A human/external script acts as the LLM to manually drive the real Harness chain (case-runner debug mode connects via `--brain-addr`) |

The debug branch of `LlmBackend::from_config` defaults to silent; when case-runner detects the
Debug variant it swaps in the HTTP brain or silence according to the debug-mode arguments. The Tauri-embedded core has no console,
so it keeps the silent fallback (honest degradation: it no longer pretends to judge).

## scripts/debug_brain.py — HTTP LLM Replacement Example

A local OpenAI-compatible `/chat/completions` HTTP server, and a minimal example of an "LLM replacement":
the decision-source logic is built into the script, and case-runner connects to it as the LLM in debug mode via `--brain-addr`
(docs/case-runner.md §Use cases).

```bash
python scripts/debug_brain.py --port 47777   # 起 HTTP 服务（--port 可选，默认 47777）
```

Built-in minimal threshold decision source: if the request content contains the completion notice pattern `完成，Context 已更新（N 字）` and N ≥ 80, it replies with the notify tool;
otherwise it stays silent. Implemented as an OpenAI-compatible response, case-runner calls it through the common OpenAiClient path.
