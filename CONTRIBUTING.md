# Contributing to Ambery

English | [中文](CONTRIBUTING.zh.md)

Ambery is an Agent desktop pet: hook-driven reading of Claude Code sessions; pet observes, reports, and manages card components.
This file gives the contribution entry points; for design and concept terminology see `concepts.md`, for architecture decisions see `spec.md`, and for documentation standards see `docs-spec.md`.

## Read these first

- `concepts.md`: concept model and glossary — use it for commit messages, comments, and test names.
- `spec.md`: technology choices and architecture decisions.
- `docs/`: detailed design split by domain; before changing behavior, find the corresponding domain doc first.
- `dev/`: development process records (undecided issues, regression records).

## Repository layout

```
core/             Rust core library (Harness / AmberyBackend / server / storage)
ambery-case/      case-runner: Storage snapshot replay, concept observation, frontend headless case host
app/              frontend vanilla TS (pet / chat / components / positioning)
app/src-tauri/    Tauri shell (static window + multi-window cards + /hook thin server)
sidecar/          Windows UIA sidecar (C#, Windows-only build/package)
scripts/          development scripts (hook installation, debug brain)
tools/            diagnostic tools such as window positioning
```

## Environment

- Rust stable (the workspace excludes the Tauri shell; run `cargo` at the repo root)
- Node 24 + npm (frontend; `app/package-lock.json` is the only lockfile)
- Tauri shell built separately: `cd app/src-tauri && cargo check` (mac has `macOSPrivateApi` enabled to support transparent windows)
- Windows UIA sidecar: .NET 9 SDK; published form self-contained win-x64, see `docs/sidecar.md §Packaging`

## Test commands

```bash
# Rust workspace (default feature = release form)
cargo test --workspace

# case-runner feature (observation/replay/frontend headless injection surface)
cargo test -p ambery-core --features case-runner

# frontend headless case (embedded core + vitest; full-chain mock/keyless)
cargo run -p ambery-case -- frontend --silent

# type and design token guards
cd app
npm ci
npx tsc --noEmit
node scripts/lint-tokens.mjs
```

CI definition is in `.github/workflows/ci.yml`: ubuntu + macos dual-platform runs the above matrix, no secrets, no real LLM calls.

## Commit conventions

- **One thing per commit**: behavior fixes, test adaptation, and doc updates are committed separately; do not mix them.
- Commit messages use Chinese (the repo's current convention); the first line summarizes the behavior, and the body gives the reason and impact.
- Commit messages and comments must not contain any internal project/example names; both code and docs must be directly public.
- Every behavior change comes with tests: core unit tests go in the corresponding module, and frontend behavior goes into `app/test/*.test.ts`.
- Docs change together with behavior: when behavior changes, update the corresponding `docs/` design docs in sync, otherwise the docs fall behind.

## Code rules

- Frontend reads go through the store (`app/src/store.ts`), and writes go through the action layer (`app/src/tauri_runtime_actions.ts`); main logic does not scatter `invoke` calls.
- Non-readonly Tauri runtime actions must enter the effect stream (`docs/effect-reporting.md`).
- New `Effect` variants must sync `effect_kind_payload` and the frontend bridge dispatch (the exhaustive match reminds at compile time).
- Storage is append-only JSONL: logs are sacred, views are volatile; never rewrite historical lines in place.
- Path resolution goes only through `core/src/paths.rs`; platform differences are gated with `cfg(windows)`, and non-Windows must not depend on UIA.

## Windows-specific boundaries

- The UIA sidecar is only compiled and only packaged on Windows; mac/Linux are the hook-driven core experience, not a downgraded version.
- Changes involving sidecar, Tauri shell window behavior, and install-hooks can only cover compilation and the protocol layer on mac/CI;
  real-device verification items are recorded in `dev/issues.md`; do not claim that unverified Windows behavior has passed.
