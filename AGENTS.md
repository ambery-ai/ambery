# AGENTS.md

Ambery is a Tauri multi-window desktop pet (pet / chat / menu / shelf / card) plus a Rust core. Read the matching doc under `docs/` before changing anything.

Keep internal material private; exclude it with `.gitignore`.

## Docs-driven development

- Develop document by document: land new code or a new capability in the matching document first (contracts in `docs/`, concepts in `concepts.md`, technology and structural decisions in `spec.md`); when documents conflict, remove the conflict before implementing.
- Record principles and decisions settled in discussion in the matching document.

## Commit messages

- Concise: a one-sentence subject is enough; no long body.
- Write commit messages in English (open-source norm: English subject, friendly to international collaboration and tooling).
- This is an open-source project: never put sensitive information in the project or in commit messages.

## Run and build

- The shell must go through the tauri CLI: `npx tauri dev` (dev with hot reload, connecting vite on 3000) or `npx tauri build` (production, embeds dist and produces an installer).
- A bare `cargo build` stays in dev mode without vite and fails at runtime with "127.0.0.1 refused to connect"; use it for compile checks only.
- CI (push/PR, three-platform gate): Rust tests (default + case-runner), frontend tsc / token guard / bilingual pairing, frontend vitest, shell cargo check. Locally, run at least `cargo test --workspace` after changes.
- Release: a `v*` tag or workflow_dispatch triggers `npx tauri build` on three platforms and uploads the installers to the Release.

## Debug

Pick the tool by the domain it covers:

- `cargo test --workspace` → core logic (Rust tests).
- `cargo run -p ambery-case -- <case.case>` → behavior reproduction (two-stage .case replay; `--health` validates / `export` builds a case from storage).
- `cargo run -p ambery-case -- frontend --silent` → frontend behavior (headless, embeds core + vitest).
- `cargo run -p ambery-case -- serve --brain-addr <url>` → end-to-end path (with the LLM stand-in `python3 scripts/debug_brain.py` and the frontend `npm run dev`, observe in a browser).
- tauri CLI (`npx tauri dev` / `build`) → shell layer (windows, build form).
- `tools/locate.ps1` → locate every Ambery window's position / size / visibility (table output).

## Storage JSONL output files

Default directory `~/.config/ambery/storage/` (`AMBERY_STORAGE_DIR` overrides it); these are app runtime artifacts used for debugging and replay. Each file's contract lives in [docs/storage.md](docs/storage.md) (one home per fact).

## User Goals

- UI interactions must not use native browser dialogs (alert / prompt / confirm): express errors and input through in-app UI elements (inline hints / inline forms), never system dialogs.
- (.gitignore) Comments are all in English.
