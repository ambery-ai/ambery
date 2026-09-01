# Contributing

English | [中文](CONTRIBUTING.zh.md)

Ambery is a desktop pet for agent models: it hooks into agent sessions, observes what the model does, and expresses itself through card windows on your desktop.

Questions, ideas, or bugs? Just open an issue or discussion — no need to ask first. The project is early-stage, and every contribution is welcome.

## Docs-first: follow the docs

The docs are the design source. Before changing behavior, read the corresponding domain doc in `docs/`, and when behavior changes, update the doc along with it (docs and code are committed separately). Terms: `concepts.md`; technology choices and structural decisions: `spec.md`. Open questions and regression records: `dev/`.

## Repository layout

```
core/             Rust core library (harness / backend / server / storage)
ambery-case/      case-runner: snapshot replay, concept observation, frontend headless host
packages/terminal-lib/      terminal access contract crate (trait / envelope / composite / test stub)
packages/terminals/wt/      Windows Terminal package: C# UIA sidecar + Rust client (Windows-only)
packages/terminals/zellij/  zellij package: in-process CLI adapter
app/              frontend vanilla TypeScript (pet / chat / cards / positioning)
app/src-tauri/    Tauri shell (static window + card windows + /hook thin server)
scripts/          development scripts
tools/            diagnostic tools
```

## Commit conventions

- **One thing per commit**: behavior, tests, and docs are committed separately.
- **English subject line** summarizing the behavior change; body explains why.
- Behavior changes come with tests: Rust unit tests in the module, frontend tests in `app/test/*.test.ts`.
- No internal project/example names in commits or comments — the repo is public.

## Code rules

- Frontend reads go through the store (`app/src/store.ts`), writes through the action layer (`app/src/tauri_runtime_actions.ts`); no scattered `invoke` calls.
- Non-readonly Tauri actions enter the effect stream (`docs/effect-reporting.md`); new `Effect` variants sync `effect_kind_payload` and the bridge dispatch.
- Storage is append-only JSONL: logs are sacred, views are ephemeral; never rewrite historical lines.
- Path resolution goes only through `core/src/paths.rs`; platform differences are gated with `cfg(windows)`, non-Windows code must not depend on UIA.

## Tests

```bash
cargo test --workspace
cargo test -p ambery-core --features case-runner
cargo run -p ambery-case -- frontend --silent   # headless frontend cases, no keys
cd app && npm ci && npx tsc --noEmit && node scripts/lint-tokens.mjs
```

CI (`.github/workflows/ci.yml`) runs this on ubuntu + macOS; no secrets, no real LLM calls.

## License

MIT. By contributing, you agree your contributions are licensed under the MIT License.
