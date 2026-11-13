# Spec

English | [中文](spec.zh.md)

> Document responsibilities / navigation: see [docs-spec.md](docs-spec.md). For concept definitions see [concepts.md](concepts.md); this file records the repository-level structure (package split and directory layout) and the host technology choices. Runtime mechanisms live in `docs/`; terminal/agent packages carry their own spec under `packages/`.

## Package split

```
ambery/
├── core/                          host crate (ambery-core + bins)
├── observe-derive/                Observe macro proc-macro crate (ambery-observe-derive)
├── packages/                      package split
│   ├── case-runner/               test replay engine (ambery-case)
│   ├── terminal-lib/              contract crate
│   ├── terminals/                 one package per terminal
│   │   ├── wt/                    C# UIA sidecar
│   │   ├── zellij/                in-process CLI adapter
│   │   └── ghostty/…              future terminal packages
│   ├── agents/                    one package per agent CLI
│   │   ├── claude/                hook script + filter + marker
│   │   └── opencode/
│   └── apps/                      frontend package
│       ├── src/                   the shared frontend
│       ├── tauri/                 Tauri shell + host layer
│       └── webui/                 pure-web form, same frontend code's second host
├── docs/                          contract documents (docs-spec.md)
├── experiments/                   throwaway prototypes
├── reports/                       research conclusions
├── scripts/                       development scripts
├── tools/                         diagnostic tools
└── Cargo.toml                     workspace manifest
```

- Specs: each crate-bearing package under `packages/` carries its own spec under its directory (`packages/case-runner/spec.md`, `packages/terminal-lib/spec.md`, `packages/apps/spec.md`, `packages/terminals/wt|zellij/spec.md`, `packages/agents/claude|opencode/spec.md`); the root file (this document) holds the structure and the host technology choices.
- Dependencies: `core` → `terminal-lib` only; `terminals/*` and `agents/*` → `terminal-lib` only; terminal/agent packages never depend on each other or on core; `apps/*` → `core`; `case` → all (read-only service). Assembly of active terminal/agent packages happens at the binary/config layer, so binaries (`apps/*`, `case`, core's own bins) may additionally depend on terminal/agent crates for wiring. The protocol (concepts §Ambery Protocol) is the contract shared across packages.

## Technology choices (host)

| Choice | Reason |
|---|---|
| Rust, one process (`ambery-core`) | one address space keeps the LLM call, tool execution and persistence on a single crash-recoverable log |
| Append-only JSONL files + single-file JSON config | crash-safe run records; the full OpenAI context is reconstructable from the log |

## Technology decisions (host)

- **JSONL over SQLite** — logs are sacred, views are ephemeral: append-only files keep every run reconstructable and greppable; SQLite enters only when query needs actually arise.
- **No local tokenizer** — token budgets follow the API's usage truth, matching what Claude Code / opencode do; a local BPE would drift from the provider's count.
- **LLM calls stay in Rust** — OpenAI-compatible Chat Completions endpoint; base_url / key come from Config; reasoning_content persists per the provider contract (docs/agent-loop.md).
- **observe-derive is a build-time companion** — the `Observe` macro lives in its own crate only because proc-macros must; it is versioned and released with core.

## Architecture decisions (host)

- **One consumer, one protocol**: the host process is the single consumer; external software reaches it only through the Ambery Protocol's contract surface. No back channels.
- **Terminal access is consumed through the contract only**: core depends on `ambery-terminal-lib` and never on a terminal package; assembly of the active terminal packages happens at the binary/config layer.
- **Storage is always append-only**: restart replays to restore state; compression is a marker, not deletion; `context.jsonl.bad` isolates unparseable lines instead of dropping them.
- **Config and storage are separate domains**: single-file `config.json` plus the identity prompt `AGENTS.md` at the config root; run data under `storage/`; paths resolved by `core/paths.rs`.

## Fixed constraints (host)

- External processes are observed, never assumed: no lifecycle inference without evidence.
- AmberyBackend never embeds terminal knowledge (no wt/zellij/claude names in core code paths; terminal packages arrive via the contract).
