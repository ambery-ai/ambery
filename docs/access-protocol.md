# Access Protocol (Ambery Protocol)

English | [中文](access-protocol.zh.md)

> Concept definitions: see concepts.md §5 (Ambery Protocol) and its subconcepts. This document defines pet's outward-facing roles and event contract — how external software becomes a Source, how events flow in and out, and how actions are graded. Its implementation documents are registered in the docs-spec responsibility map; this document does not enumerate them.

## Positioning

The middle communication layer between Ambery and external software is defined by this contract alone. Hook scripts, the sidecar stdio protocol, and the terminal adapter interface are realizations beneath the contract — their documents describe mechanisms; none of them defines access semantics on its own.

## Two roles

- **MCP client (pull)**: pet consumes data from external source plugins — a source plugin is an MCP server that offers sources or actions to Ambery.
- **MCP server (push)**: pet exposes a push entrance to agents and host software — the outside world actively tells pet "something happened".

The difference from plain MCP: pet has an attention core — observation is not a passive pipe. The Watch Schedule actively schedules the observation rhythm, Digest mandatorily processes the update stream, and things worth telling enter the Agent Loop through the Queue to proactively notify the user.

## Event shape

```text
Source   = { kind, source_id, title, focused } + extras
Content  = text | position | progress
```

- `kind`: machine identifier of the source's kind (e.g. `wt` / `zellij` / `book`).
- `source_id`: the source's stable identity — updates from the same source re-enter under the same ID (Context Slot semantics: continuity is carried by the ID, attention is driven by the update stream).
- `title` / `focused`: normalized core fields, guaranteed by the host side.
- `extras`: dynamic extension, keys prefixed with `<kind>_`, missing keys degrade gracefully; an extras field proven useful across multiple source kinds is promoted to a normalized field.
- `Content` is multi-form: body text / position (book page, playback point) / progress (percentage, episode number).

## Tool surface

| Tool | Direction | Semantics |
|---|---|---|
| `list_sources` | pull | Enumerate the sources under a host; failure = no observation, strictly distinct from "succeeded but empty" |
| `read_source` | pull | Tri-state read of one source: `Content` (evidence of life) / `Gone` (confirmed absent) / `Error` (no observation, belief unchanged) |
| `notify` | push | A host or its Hook pushes "something happened" — lifecycle events, bookmark changes, tab events |
| `report_progress` | push | Structured position / progress report (the position / progress carriers of Content) |
| `act` | pull | Perform an action on a source, authorized by the three act classes |

## Three act classes

Actions are classified by side-effect scope; permission ascends by class, and capability negotiation grants by class:

1. **observe** — side-effect-free observation: read a screen, read a page, check progress.
2. **mutate-source** — change the source's own state: pause playback, turn a page, seek back.
3. **mutate-user-env** — change the user's environment: switch a tab, switch a virtual desktop, adjust a window; ties into Platform Primitives (the platform capabilities of Ambery's own surfaces share the same class semantics).

## Processes and regulation

Source plugins run as separate processes: language freedom and crash isolation — one plugin's crash does not infect the host. The subscribe-vs-poll tradeoff, scan staggering, backpressure, and notification coalescing all concentrate in a single regulation core inside Ambery's observation loop (the Watch Schedule plans, the Timer executes). This contract defines the surface, not the rhythm policy.

## Delivery semantics

- **Idempotent**: delivering the same event twice creates no new fact.
- **Deduplicated**: multiple arrivals of the same event merge into one.
- **Ordered**: updates under the same `source_id` are delivered in the order they occurred.

Queue and Event Buffer are Harness organs and not part of this protocol — the protocol only promises delivery semantics; which organ buffers internally is an implementation detail.

## Transport

- **stdio**: the standard carrier for local plugins.
- **Streamable HTTP**: the carrier for remote sources (likewise for the push side, where Ambery acts as an MCP server).

Lifecycle negotiation, capability-change notification, and backoff reconnection all reuse MCP's established machinery; no self-invented transport mechanisms.
