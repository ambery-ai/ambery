# Spec

English | [中文](spec.zh.md)

> Document responsibilities / navigation: see [docs-spec.md](docs-spec.md). For concept definitions see [concepts.md](concepts.md); this file records the repository-level structure (package split and directory layout) and the host technology choices. Runtime mechanisms live in `docs/`; leaf packages carry their own spec under `packages/`.

## Package split

```
packages/
├── core/                          host crate (ambery-core + observe-derive + bins)
├── case/                          test replay engine (ambery-case)
├── terminal-lib/                  contract crate (adapter trait / envelope / composite / MapAdapter stub)
├── terminals/                     one leaf per terminal
│   ├── wt/                        C# UIA sidecar
│   ├── zellij/                    in-process CLI adapter
│   └── ghostty/…                  future leaves
├── agents/                        one leaf per agent CLI
│   ├── claude/                    hook script + filter + marker
│   └── opencode/
└── apps/                          frontend form packages
    ├── tauri/                     Tauri shell
    └── webui/                     pure-web form, same frontend code's second host
```

- Specs: each crate-bearing package carries its own spec under its directory (`packages/case/spec.md`, `packages/terminal-lib/spec.md`, `packages/apps/spec.md`, `packages/terminals/wt|zellij/spec.md`, `packages/agents/claude|opencode/spec.md`); the root file (this document) holds the structure and the host technology choices.
- Documents: `docs/` stays at the root and is not split; the access docs live in `docs/` (`docs/terminal/`, `docs/agents/`, `docs/cron.md` — see docs-spec responsibility map), not inside packages.
- Dependencies: `core` → `terminal-lib` only; `terminals/*` and `agents/*` → `terminal-lib` only; leaves never depend on each other or on core; `apps/*` → `core`; `case` → all (read-only service). The protocol (concepts §5, Ambery Protocol) is the contract shared across packages.

## Technology choices (host)

| Layer | Technology | Responsibility |
|---|---|---|
| System | Rust (`ambery-core`, single process) | Harness, Agent Loop, Memory, Timer, Perception, persistence, Tool Set execution |
| Persistence | append-only JSONL files + single-file JSON config | Crash-safe run records; full OpenAI context reconstructable from the log |

Tradeoffs:

- **JSONL over SQLite** — logs are sacred, views are ephemeral: append-only files keep every run reconstructable and greppable; SQLite enters only when query needs actually arise.
- **No local tokenizer** — token budgets follow the API's usage truth (est chars/4 only as fallback), matching what Claude Code / opencode do; a local BPE would drift from the provider's count.
- **LLM calls stay in Rust** — OpenAI-compatible Chat Completions endpoint; base_url / key come from Config; reasoning_content persists per the provider contract (docs/agent-loop.md).
- **observe-derive is a build-time companion** — the `Observe` macro lives in its own crate only because proc-macros must; it is versioned and released with core.

## Architecture decisions (host)

1. **Two process families, one protocol**: the host process is the single consumer; external software is reached only through the Ambery Protocol's contract surface (host push = hook; host read = enumerate/read; agent consumption = Tool Set). No back channels.
2. **Terminal access is consumed through the contract only**: core depends on `ambery-terminal-lib` and never on a leaf; assembly (which leaves are active) happens at the binary/config layer.
3. **Storage is always append-only**: restart replays to restore state; compression is a marker, not deletion; `context.jsonl.bad` isolates unparseable lines instead of dropping them.
4. **Config and Storage are two domains**: single-file `config.json` + identity prompt `AGENTS.md` at the config root; run data under `storage/`; paths resolved by `core/paths.rs` (`AMBERY_CONFIG_DIR` / `AMBERY_STORAGE_DIR` override).

## Fixed constraints (host)

- External processes are observed, never assumed: no lifecycle inference without evidence.
- AmberyBackend never embeds leaf knowledge (no wt/zellij/claude names in core code paths; leaves arrive via the contract).
