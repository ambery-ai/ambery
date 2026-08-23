# LLM Setup & Connection Errors

English | [中文](llm-setup.zh.md)

> This document defines the first-run LLM setup guide: the unconfigured default state, the setup modal (rendered like the settings panel — a reflection of Config schema nodes), and the connection test capability. Error presentation follows [errors.md](errors.md).

## Concepts

- **Unconfigured state** — the default value of `llm.active` is `"unconfigured"`. The setup guide triggers when `llm.active` is in the unconfigured state.
- **Setup modal** — a modal shown from Chat when unconfigured. It renders Config schema nodes the same way the settings panel (menu) does — the same `config_nodes` projection and mechanical rendering (`get_config_schema` → nodes → controls). It is not a hand-written form; fields appear and disappear with the schema. The menu itself is unchanged: an unconfigured state does not alter any menu behavior.
- **Connection test** — a backend capability (new) that builds the active provider once and makes one `complete` call, returning success or the concrete failure reason.
- **App-level env layer** — the key store for provider credentials. `~/.config/ambery/env` (0600) holds `KEY=value` lines; the app reads env *file first, then process environment* (the file overrides the system). Keys never live in `config.json`.

## Principles

> **Unconfigured is the honest default** — a fresh install has no LLM configured; the default state must say so.

> **The setup modal is a reflection** — it renders schema nodes, not custom UI, using the same rendering as the settings panel. One rendering, no hand-written form.

> **The menu is unchanged** — the setup guide does not live inside the menu and does not alter menu behavior; it is a modal reached from Chat.

> **Failures are never silent** — when an action fails (an LLM call, a key write), the user sees it; the setup flow never hides a failure.

> **Key stays out of config** — `config.json` stores only the environment-variable *name* (`api_key_env`); the key itself lives in the app-level env layer or the process environment. The setup modal can *enter* a key: writing it to the env file. `config.json` never contains a key value.

## Key storage model (app-level env layer)

The env file `~/.config/ambery/env` is an **app-level environment-variable layer**:

- Format: `KEY=value` per line; blank lines and `#` comments allowed.
- Permission: `0600` — the file is user-secret.
- Resolution order for a key: **env file → process environment** (first hit wins). The file *overrides* the system, it is not a second namespace.
- Variable naming: unified `AMBERY_<PROVIDER>_API_KEY` (e.g. `AMBERY_DEEPSEEK_API_KEY`). When the UI writes a key for a provider whose `api_key_env` differs (legacy name like `DEEPSEEK_API_KEY`) or is empty, the write also normalizes `api_key_env` to the unified name (config field only — never a key value). This is an implicit one-way migration; no separate migration step.
- Read path: `LlmBackend::from_config` resolves `api_key_env` through the app-level layer (env file first, then `std::env::var`). Providers with no `api_key_env` (local endpoints like ollama/brain) need no key.

Why this shape: a GUI app launched from Finder/Dock does **not** inherit shell-profile exports (launchd provides the environment), so shell-only key setup breaks for the installed app. The env file gives a shell-independent home for keys while keeping `config.json` key-free.

## Trigger model

| Trigger | Condition | Action |
|---|---|---|
| App startup | `llm.active` == unconfigured value | Mark state "unconfigured" |
| Chat opens | unconfigured | Chat shows the setup modal + a banner hint |
| Chat send | LLM call fails | Error notification per [errors.md](errors.md); the banner's `setup` action opens this modal |
| Banner action | `setup` action | Opens the setup modal (unconfigured / connection-failed share the modal) |

## Setup modal

Opened from Chat when unconfigured (or from a banner's "open config" action). Content:

1. **Provider selection** — renders the `llm.active` schema node (enum select: unconfigured / debug / providers).
2. **Provider fields** — renders the selected provider's schema nodes (`base_url` / `model` / `api_key_env` etc.).
3. **Key input** — each provider gets a password input for its key. Local endpoints (no `api_key_env`, e.g. ollama/brain) show "no key needed" instead. States:
   - **Unset** — the env file and process environment both lack the key: the input shows a warning style with placeholder "enter API key".
   - **Set** — either source has the key: placeholder `•••••••• (set — leave empty to keep)` plus a small "set (source: env file / environment)" hint, and a **clear** button that removes the key from the env file.
   - **Saving** — disabled while writing.
   - Presence is judged by the same resolution chain as reads (env file → process env), locally and instantly — independent of `test_llm` round-trips.
4. **Save semantics** — empty submit = no change (keep existing key); filled submit = upsert into the env file (unified `AMBERY_<PROVIDER>_API_KEY` + `api_key_env` normalization); clear = remove from the env file. Write failures surface as an inline error (never silent). After a save/clear the modal immediately refreshes the set/unset state and **auto-reruns `test_llm`**.
5. **Connection test** — a button calling the `test_llm` backend capability; result shown inline (success, or the concrete failure reason).

The UI distinguishes two failure flavors: **unset** (local presence check — input warning) vs **set but unreachable** (connection test / chat error — error bubble + banner). Same UI component serves both the setup modal and the menu settings panel (single rendering source).

Completion: `llm.active` is no longer the unconfigured value → the modal no longer auto-triggers.

## Connection errors

Error presentation (bubble / banner) follows the model in [errors.md](errors.md). This guide's only connection to it: the banner's `setup` action opens the setup modal above.

## Backend changes

- **Unconfigured default** — `LlmConfig::default().active` is `"unconfigured"`; `LlmBackend::from_config` treats the unconfigured value as no provider — the setup guide (modal + banner, action `setup`) surfaces it per [errors.md](errors.md). **No migration**: existing config files keep their current `active`; the unconfigured default applies to fresh installs only (a config file that already exists is never rewritten to the new default).
- **`test_llm` capability** — a new command that reads the active provider, builds `OpenAiClient` once, makes one `complete` call, returns `{ok, message}` with the concrete failure reason. Reuses the existing provider construction path.
- **`set_api_key(provider, Option<key>)`** — `Some` upserts the key into the env file (unified name, normalizing `api_key_env`), `None` clears it. Exposed over both channels (Tauri command + HTTP route); the core function is unit-testable directly.
- **`get_api_key_status(provider)`** — presence check through the env-file-first resolution chain; returns set/unset + source. Local and instant.

## Explicitly out of scope

- Storing keys in config (the env-file discipline keeps `config.json` key-free).
- Auto-detect "env set but key invalid" at startup (that is the error path, not the setup path).
- Connection test as a periodic health check (post-0.1.0).
- OS keychain integration (post-0.1.0; the 0600 env file is the 0.1.0 answer).
