# Terminal Adapter Design

English | [中文](terminal-adapter.zh.md)

Terminal access abstraction: a layered model — L1 (hook + lookup) → M1 → L2 (comprehensive query pipeline) → M2 → L3 (visible & adjustable) → M3 → agenttool. Multi-terminal compatibility = each terminal provides L1 (transport/lookup); the query strategy is independent as L2.

> See `concepts.md` §14 (Terminal Adapter) for the concept positioning.

## Layered model

```
agenttool — agent obtains M3
  ▲ M3 = query result (instance status + hit + optional content + parameters)
L3 · visible & adjustable
  ▲ M2 (final hit result)
L2 · comprehensive query (pipeline, composable, user-rewritable)
  ├─ stage 1 · conclusive-condition filtering ──► M2
  ├─ stage 2 · ambiguity scoring          ──► M2
  └─ … user-insertable stages             ──► M2
  ▲ M1 = { tab attributes, hook records }
L1 · hook + lookup itself
```

### L1 · transport & enumeration (terminal-carrier language only)

The minimal per-terminal contract — the protocol asks for as little as possible, and every term is terminal-carrier language, never Code CLI:

- `enumerate() -> Vec<TabInfo>` — traverse the terminal's tabs/panes, returning carrier attributes (id / title / cwd / command / focused / …); this is the M1 tab-attributes part, the basis of discovery (startup scan, reconciliation).
- `read(tab_id) -> ReadOutcome` — read one carrier's text by its opaque id:
  - `Content(text)` — read succeeds → observation: alive;
  - `Gone` — positively confirmed absent (the reader can verify the id no longer exists) → observation: dead (strong evidence);
  - `Error` — transient failure → no observation, retry; **never** treated as death.

L1 does **not** know instances: no `locate(instance)`, no marker matching, no sid8 / project / status. Those live above — the hook-records part of M1 comes from the hook event channel, and the Code CLI ↔ tab **join** is the L2 pipeline's job (matching by marker / attributes, ambiguity scoring).

### M1 · contract (pure data)

`{ tab attributes, hook records }`. The query's input data, fully preserved (tab: id / title / cwd / command / focused / …; hook: sid8 / project / status / …), nothing dropped. Tab attributes are produced by L1's `enumerate`; hook records arrive via the hook event channel of the backend.

### L2 · comprehensive query (pipeline, composable, user-rewritable)

A composable pipeline with multiple stages (conclusive-condition filtering → ambiguity scoring → …). Users can insert / rewrite their own stages (a seam, toward user plugins). **Each stage boundary produces one M2**.

### M2 · match result

`Hit / Ambiguous (candidates) / Not-found`. Hit = Found(tab); ambiguous and not-found are failure paths (error channel), not normal results.

### L3 · visible & adjustable

Result presentation + parameter adjustment (on ambiguity the user sees the candidates and corrects the match). Produces **M3**.

### M3 · query result

Instance status + match result + optional content + adjustable parameters.

### agenttool

The tool through which the agent obtains M3.

## Implementation contract (Rust)

Layer names (L1/M1/…) are model vocabulary; code identifiers use semantic names.

```rust
pub enum ReadOutcome {
    Content(String),  // evidence: alive
    Gone,             // evidence: dead (positively confirmed by the adapter)
    Error(String),    // no observation; belief unchanged, never fatal
}

pub trait TerminalAdapter: Send + Sync {
    fn locate(&self, inst: &str) -> Option<TabRef>;
    fn read(&self, tab: &TabRef) -> ReadOutcome;
    fn unlocate(&self, inst: &str);
}
```

`locate`/`unlocate` take instance names (marker matching) — per the layered model that identity judgment belongs to L2; the trait carries it today (it moves up with the query pipeline, marker included). `read` takes only the carrier `TabRef` and is pure L1.

`Gone` is returned only after positive confirmation inside the adapter; consumers never recheck:

| adapter | how Gone is confirmed |
|---|---|
| WtAdapter | `read_tab` fails → `list_tabs` succeeds and the marker tab is absent; any other failure → `Error` |
| ZellijAdapter | `dump-screen` fails → `list-panes` shows no such pane id (stable ids, positional) |
| MapAdapter | tab unknown to the adapter, or its content entry removed |
| Composite | routes to the producing adapter and passes its outcome through; no route = `Error` |

Consumers: the fallback timer scan reads **by the already-located tab** (the instance record carries the TabRef; re-locating is only a fallback for never-located instances) — `Content` → change detection, `Gone` → mark the instance closed, `Error` → record only, never close. The agent-facing `fetch_terminal` path collapses `Gone`/`Error` to a plain "unreadable" result (the tri-state is internal semantics).

## Principles

- **Plugin-ability (seam)** — each layer boundary is a provider/consumer contract (a seam), so the adapter is pluggable: users add new terminal types and query stages without touching the core.
- **Responsibilities are not code-cli-specific** — the adapter abstraction serves reading and enumeration for the loop; future terminals may serve purposes beyond a code CLI, so the abstraction must not become code-cli-specialized.

## Implementations (L1 transport / lookup providers)

| adapter | form | access | platform |
|---|---|---|---|
| **WtAdapter** | standalone C# process | stdio JSONL calls into C#; UIA (CASCADIA/TermControl) locate + read | Windows |
| **ZellijAdapter** | in-process (Rust calls CLI directly) | `zellij action` commands (list-panes / dump-screen / …) | cross-platform |
| **MapAdapter** | in-process (built into core) | shared map (the terminal/terminal_gone scenario source for case-runner) | cross-platform |
| **Composite** | in-process (built into core) | multi-adapter dispatch | cross-platform |

WtAdapter keeps its standalone process form — UIA reading depends on .NET assemblies, and Rust cannot directly consume the UIA TextPattern, hence a separate exe. ZellijAdapter only needs to call the CLI, executes natively in Rust, and has no separate process.

The locate query strategy lives in **L2**, not hardcoded in the adapter: `find_pane`'s `title.contains(instance)` assumes the zellij title carries the marker (`project·sid8`); observed real pane titles are `◐ ambery` / `✳ agent-team` (spinner + project name, no sid8) → locate mismatch. The query strategy moves to the L2 pipeline.

## Config fields

One boolean switch per adapter:

```text
terminal.adapter_wt: bool      // enable the wt adapter
terminal.adapter_zellij: bool  // enable the zellij adapter
// adapters not listed default to false; all false = no terminal access, Hook-driven core experience still works
```

WtAdapter uses the conventional path (env `AMBERY_SIDECAR` > repo conventional path); ZellijAdapter uses the default session.
