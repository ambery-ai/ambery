# `0.1.0` Follow-up Capabilities

> This document only records directions after `0.1.0`; it does not define the current implementation, protocol, permission flow, asset format, rendering interface, or configuration structure.

## Global Wake-up Hotkey (Explicitly Cut)

0.1.0 **does not implement** a global wake-up hotkey. An earlier design once listed it as a v2 candidate but never delivered it; it is formally cut here: it involves cross-platform global listening, conflicts with tray show/hide semantics, and preemption decisions against the user's existing hotkeys, so it is not part of alpha0's minimal credible surface. When restarting this capability, first determine the trigger key, conflict handling, and interaction with tray/blur-close, and only then land the implementation.

## Codex Skin

Support for a Codex-style pet skin is considered for later. It belongs to the character appearance and related design system capabilities, not the delivery scope of the current theme / palette facility.

The current theme is responsible for the application-wide palette and common style modifications. The specific design of the Codex skin will be converged separately later; do not change the boundaries of the `0.1.0` theme model in order to support it early.

A formal design must independently determine the skin's identity, assets and motion expression, rendering boundaries, configuration entry point, and its relationship with the existing pet name, theme, and Harness copy.

## Temporary User effort Adjustment and Chat Shortcut Bar

The user manually shifts the effort of the current `user_chat` (temporarily raising/lowering the thinking budget), plus a chat shortcut bar as the UI entry for that adjustment. These are independent user capabilities above effort classification and keyword matching; a formal design must independently determine the granularity of manual shifting, its effective scope, and the form of the shortcut bar.

## Tool Call Batching and Concurrency (Observation-Driven)

Specific tool call optimizations — merging multiple calls into one response, having a single call carry multiple purposes, and executing independent calls concurrently — must be judged by real runtime data. First observe the actual tool call distribution produced by pet over the long term, then decide whether and how to optimize; do not optimize for its own sake.

Confirmed principle: tool calls have two syntactic registers — multiple independent calls express **process semantics** (issued one by one, observed one by one, each later step depending on the previous result), while a single call carrying batch parameters expresses **set semantics** (one set-level decision, no internal branching, all succeed or all fail). Batching is not a performance optimization; it is the syntactic primitive by which the LLM expresses "this is one holistic decision". Harness faithfully executes whichever form the LLM chooses and never implicitly converts between the two forms. Progressive disclosure constrains "how parameters are discovered"; batching constrains "how known parameters are carried" — the two are orthogonal, not in conflict.

A formal design must independently determine: failure semantics of batch update (atomic rejection vs per-item independence), the snapshot mechanism (per-path independent snapshot vs batch query), whether the read-only tool group (`fetch_terminal` / `read_memory` / `edit_config` grep/query branches) executes concurrently, and the comparison experiment plan once observation shows a signal.

## Jump to the Target Instance's Desktop

The user-visible "jump": from a Card/notification, switch with one click to the virtual desktop where the target Code CLI instance resides. It consumes Platform Primitives' `switch_vd` (docs/platform-primitives.md); a formal design must independently determine the jump entry (quick_jump event chain or an independent gesture), target location (Terminal Adapter locate), and switch-confirmation semantics.

## v2 Prune Mode (Summary Replacement)

After a hook fires, use the configured model to generate a summary; keep only the summary in Queue to replace the original content, greatly compressing the Context. The full original Terminal Content is still archived in terminal-content.jsonl (the normalized full text is computed on demand from the original digest), and can be restored on demand through the query interface. Reference pi-context-prune: detect tool call completion → model generates summary → replace original output → original data is kept in the session index and can be restored at any time via context_tree_query.

## Autonomy Position State

The current Autonomy top-level state is facial expression and motion; position state (changing pet's screen position according to the state key) is a later capability. A formal design must determine the position expression, motion path, and obstacle-area interaction.

## OpenCode Filter Rules

OpenCode Filter's noise list, block splitting, and line-wrap merging parameters are to be converged after real samples (currently supporting the `"claude"` and `"opencode"` kinds; the opencode rules are a skeleton).

## Multi-Monitor Validation

Window layout, off-screen behavior, and center anchoring in multi-monitor scenarios need validation in a real multi-monitor environment.

## Default Value Calibration

Defaults such as Compression thresholds and Timer intervals are to be calibrated against real usage data.
