# Docs Spec (docs directory constraints)

English | [中文](docs-spec.zh.md)

> This document constrains the responsibility boundaries and content admission of `docs/*.md` and `concepts.md`. It governs only these two; `reports/` (research) has its own governance and is outside this document's scope. Before touching any `docs/*.md` or `concepts.md` change, you must read this file first.

## Responsibility map

Each `docs/*.md` has one responsibility. The grouping is reading organization, not boundary enforcement — the real boundary is each line's "one-sentence responsibility".

### Core runtime

- `harness.md` — Harness data model, injection rules, trigger model, and JSONL storage format
- `agent-loop.md` — LLM abstraction, Tool Set protocol, and mock hook contract
- `llm-setup.md` — first-run LLM setup guide (unconfigured default, setup modal, key input, connection test)
- `autonomy.md` — expression model, default mapping, and override semantics of expression Autonomy
- `effort.md` — effort thinking budget: domain-layer unified tiers and provider translation
- `toolset.md` — parameter schemas of the nine function definitions pet can call
- `cron.md` — Cron task model, persistence format, and the three scheduling tool contracts
- `memory.md` — Memory Workspace directory model, read_memory / write_memory contracts
- `streaming.md` — LLM reply streaming incremental push contract

### Storage and configuration

- `storage.md` — Storage directory layout, file semantics, record format, and lifecycle
- `config.md` — config.json + AGENTS.md model, migration, reconcile, unified modification pipeline

### Access protocol

- `access-protocol.md` — Ambery Protocol contract: how external software becomes a Source, and how events flow in and out

#### Terminal

- `docs/terminal/terminal-adapter.md` — terminal access abstraction: locate/read/unlocate interfaces, per-terminal implementations, config fields, and the layered access model (L1–L3 / M1–M3)
- `docs/terminal/wt/sidecar.md` — WtAdapter process protocol (the independent process of the wt terminal adapter: stdio JSONL, command set, and lifecycle)
- `docs/terminal/timer.md` — fallback patrol scan scheduling, stagger algorithm, and scan action application points

#### Agent

- `docs/agents/claude/hook.md` — real Claude Code hook contract: event layering, marker location, startup scan, installation
- `docs/agents/filter.md` — terminal text filtering strategy and structure-understanding data types

#### Host side

- `core-server.md` — embedded thin HTTP server: binds only 127.0.0.1 to carry external hook access

### Cross-platform

- `platform-primitives.md` — Ambery's own platform capability layer (window positioning/focus/desktop handling of its own surfaces): interfaces and per-platform implementations

### Runtime reporting and errors

- `effect-reporting.md` — Tauri runtime action effect reporting: action layer, channel, kind/payload
- `errors.md` — error presentation model: errors as notifications, bubble/banner outlets, record–presentation separation

### Frontend and windows

- `view.md` — View physical implementation, interaction details, and Config fields
- `chat-panel.md` — Chat Panel summon/close, layout, and message rendering rules
- `components.md` — Component invocation protocol, lifecycle events, direction geometry
- `multi-window.md` — multi-window solution design
- `tauri-shell.md` — Tauri shell form and cross-platform UIA boundary
- `window-positioning.md` — window direction layout engine
- `window-follow.md` — window follow coordinate system, responsibility layering, and state semantics
- `pet-window-size.md` — pet window size formula and principles
- `card-window-size.md` — Card window size: single source, projection into the Card file, and window creation
- `theme.md` — theme/color table and its Config facility
- `ui-composition.md` — UI code layering and composition: host / Surface / widget layers, widget tiers, style composition, component principles

### Internationalization

- `i18n.md` — UI and Harness, two independent language preferences

### Processing flows and overviews

- `processing-flow.md` — main processing flow ASCII diagram (what log to write at each step)
- `module-storage-flow.md` — code module layering + processing flow diagram
- `concrete-insight.md` — real data + diagrams demonstrating the concept chain

### Evaluation tools

- `debug-agent.md` — DebugAgent pure mock and debug CLI
- `case-runner.md` — Storage snapshot regression and concept observation infrastructure
- `case-eval-system.md` — case expression evaluation system
- `observability.md` — observability base: compile-time enforcement that all concept modules are observable
- `tools.md` — development tool collection: tools/ directory script tools and core standalone bin tools (locate / run-vite / ambery-activity)
- `kitchen-sink.md` — the rendering-layer verification surface: the page, what it renders, its controls, and its sync guarantees

### Capabilities and benchmark

- `agent-assistance.md` — capability boundary of the Agent work supervision and collaboration assistant
- `capability-evaluation-project.md` — the system for decomposing capabilities into repeatable evaluation projects

### Roadmap

- `roadmap.md` — release ladder toward 0.1.0: theme and acceptance per stage; the single home of version boundaries
- `post-0.1.0.md` — post-0.1.0 capability roadmap: one short statement per future capability

## Document distribution

Where contract documents live in the repository:

- `docs/` is the single home for contract / mechanism / protocol documents; every such document is registered in the responsibility map above. `packages/*/` holds only `spec.md` pairs (technology choices, dependency boundaries, tradeoffs) and code — never contract documents.
- `docs/` stays at the root and is not split: the access docs live in `docs/` (`docs/terminal/`, `docs/agents/`, `docs/cron.md`), not inside packages.
- A parent contract document does not register the documents under its group: which documents belong to a group is stated once in the responsibility map, not by enumerating them in the parent document's intro.
- References use the repository-root path form (`docs/terminal/wt/sidecar.md`); within the same folder, bare filenames are allowed.

## General principles

The following content is **forbidden** in ordinary `docs/*.md`; each has its own dedicated carrier:

- **Version info** — docs do not write any version number or version range (e.g. "X belongs before/after 0.1.0"). Version boundaries are defined by the release ladder (`docs/roadmap.md`) and nowhere else; a single capability document does not temporarily assign its own version attribution.
- **Status markers** — docs do not write volatile status (current contract / to-be-landed / undecided, etc.). Superseded historical plans are deleted or marked historical in the original text, not expressed as maintained status fields.
- **No internal issue references** — docs do not reference internal issue numbers (#N, issue-xxx, issues #N): docs state only the current state and contracts, and do not use internal issue numbers as evidence or anchors; external upstream references (such as upstream issue / discussion numbers) are allowed.
- **Research and argumentation process** — belongs in `reports/`; docs record only converged conclusions; the ins and outs of research are not design contracts.
- **Single-session process reviews** (grill reviews, etc.) — belong in `drafts/`, not design contracts.
- **To-be-landed / undecided implementation lists** — execution items go to development tickets (the current round of fixes), and future items go to `docs/post-0.1.0.md` (roadmap); do not deposit them in docs pretending to be an implementation basis.
- **Future capabilities** — new capabilities after 0.1.0 are uniformly written into `docs/post-0.1.0.md`, only as a roadmap, one short statement per item; a separate document is split off when formal design starts.

The above general principles also constrain `concepts.md` (the domain concept document).

## Principles section

A contract document states its standing rules in a `## Principles` section, placed after the document's scope note and before the mechanism sections. The section is not mandatory: a document whose rules are already carried by its own data model, field list, interface or diagram — a format, a coordinate contract, a flow diagram — omits it, and an absent section is not a gap.

- One principle per blockquote paragraph, in the form `> **<name>** — <rule>`; a blank line separates principles.
- The first principle is the document's scope: what this document defines, and which document owns what it leaves out.
- A principle states a standing rule or a boundary, never a mechanism step; a principle that needs detail points at the section or document that owns it.
- A principle is named by its rule, never by a number, a version, or a status.

## Concept document spec (concepts.md)

`concepts.md` is the first-class document of domain concepts, constrained by the above general principles. A concept entry consists of:

- **Positioning** — what the concept is, clear in one sentence.
- **Boundaries and relationships** — the boundaries with related concepts, who uses it, and its place in the whole, stated directly.
- **Instantiable** — a concept should be able to land as an instantiable type (trait / enum / struct), not an abstract noun.
- Implementation details, protocols, config fields, and command sets belong to the corresponding `docs/*.md`; **concepts do not reference design documents** — a concept entry is a self-contained definition and does not point to `docs/*.md`; the reference direction is docs → concepts (design documents may reference concepts, concepts do not reference design documents).

The boundary between concept and design: concepts.md answers "what is it and where are the boundaries", and docs/*.md answers "how to implement and what the interface looks like". Concept changes likewise require reading this file first.

**Example requirements**: examples live in the concept document; each example analyzes one complete process and marks what each concept represents in it; every concept is covered by at least one example; one example covers as many concepts as possible and at least three; the number of examples is not equal to the number of concepts (one example covers many concepts, so there can be fewer examples than concepts).
