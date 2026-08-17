# LLM Setup & Connection Errors

English | [中文](llm-setup.zh.md)

> This document defines the first-run LLM setup guide and connection-error reporting: the unconfigured default state, the setup modal (rendered like the settings panel — a reflection of Config schema nodes), the connection test capability, and how chat reports LLM failures.

## Concepts

- **Unconfigured state** — the default value of `llm.active` is `"unconfigured"`. The setup guide triggers when `llm.active` is in the unconfigured state.
- **Setup modal** — a modal shown from Chat when unconfigured. It renders Config schema nodes the same way the settings panel (menu) does — the same `config_nodes` projection and mechanical rendering (`get_config_schema` → nodes → controls). It is not a hand-written form; fields appear and disappear with the schema. The menu itself is unchanged: an unconfigured state does not alter any menu behavior.
- **Connection test** — a backend capability (new) that builds the active provider once and makes one `complete` call, returning success or the concrete failure reason.

## Principles

> **Unconfigured is the honest default** — a fresh install has no LLM configured; the default state must say so.

> **The setup modal is a reflection** — it renders schema nodes, not custom UI, using the same rendering as the settings panel. One rendering, no hand-written form.

> **The menu is unchanged** — the setup guide does not live inside the menu and does not alter menu behavior; it is a modal reached from Chat.

> **Failures are never silent** — sending a chat message while the LLM cannot be reached must produce a visible error. The existing `llm_error` effect already reaches the frontend (only pet renders it today); chat subscribes to the same channel.

> **Key stays out of config** — only the environment-variable *name* (`api_key_env`) is stored in config; the key itself lives in the environment (`std::env::var`). The setup modal shows the variable name and guides the user to set it; it never stores the key.

## Trigger model

| Trigger | Condition | Action |
|---|---|---|
| App startup | `llm.active` == unconfigured value | Mark state "unconfigured" |
| Chat opens | unconfigured | Chat shows the setup modal + a banner hint |
| Chat send | LLM call fails | Error bubble in the message stream (reason-specific) + banner above the input |
| Banner action | "open config" | Opens the setup modal again (unconfigured / connection-failed are two states of the same modal) |
| Banner dismiss | user closes | Hides the current banner only; reappears on the next error |

## Setup modal

Opened from Chat when unconfigured (or from a banner's "open config" action). Content:

1. **Provider selection** — renders the `llm.active` schema node (enum select: unconfigured / debug / providers).
2. **Provider fields** — renders the selected provider's schema nodes (`base_url` / `model` / `api_key_env` etc.).
3. **Key status** — each provider's `api_key_env` is a reflected field (present on every provider; `ollama` has none). The modal shows it as a **variable name + detection status**, not an editable input: display the variable name and whether the environment variable is set (backend check). Default preset variable names follow the `AMBERY_<NAME>_API_KEY` convention (e.g. `AMBERY_DEEPSEEK_API_KEY`); the key itself is never entered here — the user sets it in the shell environment.
4. **Connection test** — a button calling the new `test_llm` backend capability; result shown inline (success, or the concrete failure reason).

Completion: `llm.active` is no longer the unconfigured value → the modal no longer auto-triggers.

## Connection errors (chat)

- **Error bubble** — on LLM call failure, a bubble is inserted into the message stream. It distinguishes the reason: network unreachable / timeout / 401 invalid key / 400 bad request / env var unset / provider missing. It includes a retry action.
- **Banner** — above the chat input, shown only while errors are active. Dismissible (hides the current banner only; the next error re-shows it). The banner carries an "open config" action that reopens the setup modal.
- **Degraded-reply note** — when the failure path falls back to DebugAgent, the round still produces a reply; the bubble/banner must note "current reply is degraded (debug fallback)" so the visible reply does not contradict the visible error.

## Backend changes

- **Unconfigured default** — `LlmConfig::default().active` is `"unconfigured"`; `LlmBackend::from_config` treats the unconfigured value as "no LLM" (silent fallback semantics, no error card spam at startup before any interaction). **No migration**: existing config files keep their current `active`; the unconfigured default applies to fresh installs only (a config file that already exists is never rewritten to the new default).
- **`test_llm` capability** — a new command that reads the active provider, builds `OpenAiClient` once, makes one `complete` call, returns `{ok, message}` with the concrete failure reason. Reuses the existing provider construction path.

## Explicitly out of scope

- Storing keys in config (the environment-variable discipline is unchanged).
- Auto-detect "env set but key invalid" at startup (that is the error path, not the setup path).
- Connection test as a periodic health check (post-0.1.0).
