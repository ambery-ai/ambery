# Roadmap

English | [中文](roadmap.zh.md)

> - **Scope of this document** — this document defines the release ladder toward 0.1.0: the theme each alpha carries and the acceptance that closes it. Day-to-day planning stays in development tickets, and capabilities beyond 0.1.0 belong to [post-0.1.0.md](post-0.1.0.md).

The release ladder toward 0.1.0: from the first packaged snapshot on, each alpha carries one theme and closes when its acceptance holds. Day-to-day planning stays in development tickets, not here; capabilities beyond 0.1.0 live in [post-0.1.0.md](post-0.1.0.md).

## Themes

### alpha1 · Surface rebuild

The frontend lands on the new window composition: Svelte window components over the widget tiers, theme tokens as the single style source, and rendering defects closed. Acceptance: windows and Cards behave stably on the desktop.

### alpha2 · Ambery Protocol

The protocol becomes real beyond Claude Code: external software joins as a Source Host through MCP, Ambery both consumes and exposes MCP, and a second, natively structured Source Host runs end to end. Acceptance: a session from that Source Host reaches the pet loop — interruption, Card, follow-up.

### alpha3 · Memory & Session

Memory that survives restarts: Session identity continues across days, Context carries over, compression keeps recoverable detail, history is queryable, and digests land in Memory automatically. Acceptance: the pet remembers yesterday.

### alpha4 · Endurance

Long-running reliability: hook delivery without silent loss, queue timeouts, storage growth under rotation, and aggregate statistics surfaced as a Card. Acceptance: a multi-day soak run exists and passes.

### 0.1.0 · First release

The Ambery Protocol frozen, the first cookbook edition where every documented recipe is reachable from the docs, and a first-run experience worthy of the installer.

## Beyond 0.1.0

Directions after the first release are registered in [post-0.1.0.md](post-0.1.0.md), one short statement per capability.
