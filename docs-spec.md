# Docs Spec (docs directory constraints)

> This document constrains the responsibility boundaries and content admission of `docs/*.md` and `concepts.md`. It governs only these two; `spec.md` (tech stack) and `reports/` (research) each have their own governance and are outside this document's scope. Before touching any `docs/*.md` or `concepts.md` change, you must read this file first.

## Responsibility map

Each `docs/*.md` has one responsibility. The grouping is reading organization, not boundary enforcement — the real boundary is each line's "one-sentence responsibility".

### Core runtime

- `harness.md` — Harness data model, injection rules, trigger model, and JSONL storage format
- `agent-loop.md` — LLM abstraction, Tool Set protocol, and mock hook contract
- `autonomy.md` — expression model, default mapping, and override semantics of expression Autonomy
- `filter.md` — terminal text filtering strategy and structure-understanding data types
- `debug-agent.md` — DebugAgent pure mock and debug CLI
- `toolset.md` — parameter schemas of the nine function definitions pet can call
- `agent-assistance.md` — capability boundary of the Agent work supervision and collaboration assistant
- `capability-evaluation-project.md` — the system for decomposing capabilities into repeatable evaluation projects
- `effort.md` — effort thinking budget: domain-layer unified tiers and provider translation

### Storage and configuration

- `storage.md` — Storage directory layout, file semantics, record format, and lifecycle
- `config.md` — config.json + AGENTS.md model, migration, reconcile, unified modification pipeline
- `memory.md` — Memory Workspace directory model, read_memory / write_memory contracts

### Scheduling

- `timer.md` — Timer fallback scan scheduling, stagger algorithm, and scan action application points
- `cron.md` — Cron task model, persistence format, and the three scheduling tool contracts

### Terminal access

- `terminal-adapter.md` — terminal access abstraction: locate/read/forget interfaces, per-terminal implementations, and config fields
- `sidecar.md` — WtAdapter process protocol (the independent process of the wt terminal adapter: stdio JSONL, command set, and lifecycle)

### Cross-platform

- `platform-primitives.md` — abstraction group of platform-specific capabilities (OS-layer capabilities such as virtual desktop switching): interfaces and per-platform implementations

### Communication and protocols

- `hook.md` — real Claude Code hook contract: event layering, marker location, startup scan, installation
- `core-server.md` — embedded thin HTTP server: binds only 127.0.0.1 to carry external hook access
- `streaming.md` — LLM reply streaming incremental push contract
- `effect-reporting.md` — Tauri runtime action effect reporting: action layer, channel, kind/payload

### Frontend and windows

- `view.md` — View physical implementation, interaction details, and Config fields
- `chat-panel.md` — Chat Panel summon/close, layout, and message rendering rules
- `components.md` — Component invocation protocol, lifecycle events, direction geometry
- `multi-window.md` — multi-window solution design
- `tauri-shell.md` — Tauri shell form and cross-platform UIA boundary
- `window-positioning.md` — window direction layout engine
- `window-follow.md` — window follow coordinate system, responsibility layering, and state semantics
- `pet-window-size.md` — pet window size formula and principles
- `theme.md` — theme/color table and its Config facility

### Internationalization

- `i18n.md` — UI and Harness, two independent language preferences

### Processing flows and overviews

- `processing-flow.md` — main processing flow ASCII diagram (what log to write at each step)
- `module-storage-flow.md` — code module layering + processing flow diagram
- `concrete-insight.md` — real data + diagrams demonstrating the concept chain

### Evaluation tools

- `case-runner.md` — Storage snapshot regression and concept observation infrastructure
- `case-eval-system.md` — case expression evaluation system
- `observability.md` — observability base: compile-time enforcement that all concept modules are observable
- `tools.md` — development tool collection: tools/ directory script tools and core standalone bin tools (locate / run-vite / ambery-activity)

### Roadmap

- `post-0.1.0.md` — post-0.1.0 capability roadmap: one short statement per future capability

## General principles

The following content is **forbidden** in ordinary `docs/*.md`; each has its own dedicated carrier:

- **Version info** — docs do not write any version number or version range (e.g. "X belongs before/after 0.1.0"). Version boundaries are defined by unified release planning; a single capability document does not temporarily assign its own version attribution.
- **Status markers** — docs do not write volatile status (current contract / to-be-landed / undecided, etc.). Superseded historical plans are deleted or marked historical in the original text, not expressed as maintained status fields.
- **No internal issue references** — docs do not reference internal issue numbers (#N, issue-xxx, issues #N): issue tracking belongs to `dev/issues.md`; docs state only the current state and contracts, and do not use internal issue numbers as evidence or anchors; external upstream references (such as upstream issue / discussion numbers) are allowed.
- **Research and argumentation process** — belongs in `reports/`; docs record only converged conclusions; the ins and outs of research are not design contracts.
- **Single-session process reviews** (grill reviews, etc.) — belong in `drafts/`, not design contracts.
- **To-be-landed / undecided implementation lists** — execution items go to development tickets (the current round of fixes), and future items go to `docs/post-0.1.0.md` (roadmap); do not deposit them in docs pretending to be an implementation basis, and do not spread undecided items publicly in `dev/issues.md`.
- **Future capabilities** — new capabilities after 0.1.0 are uniformly written into `docs/post-0.1.0.md`, only as a roadmap, one short statement per item; a separate document is split off when formal design starts.

The above general principles also constrain `concepts.md` (the domain concept document).

## Concept document spec (concepts.md)

`concepts.md` is the first-class document of domain concepts, constrained by the above general principles. A concept entry consists of:

- **Positioning** — what the concept is, clear in one sentence.
- **Boundaries and relationships** — the boundaries with related concepts, who uses it, and its place in the whole, stated directly.
- **Instantiable** — a concept should be able to land as an instantiable type (trait / enum / struct), not an abstract noun.
- Implementation details, protocols, config fields, and command sets belong to the corresponding `docs/*.md`; **concepts do not reference design documents** — a concept entry is a self-contained definition and does not point to `docs/*.md`; the reference direction is docs → concepts (design documents may reference concepts, concepts do not reference design documents).

The boundary between concept and design: concepts.md answers "what is it and where are the boundaries", and docs/*.md answers "how to implement and what the interface looks like". Concept changes likewise require reading this file first.

**Example requirements**: examples live in the concept document; each example analyzes one complete process and marks what each concept represents in it; every concept is covered by at least one example; one example covers as many concepts as possible and at least three; the number of examples is not equal to the number of concepts (one example covers many concepts, so there can be fewer examples than concepts).
