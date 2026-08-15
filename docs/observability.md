# Observability (Observability Foundation)

> Concept: every concept module is observable, and coverage is guaranteed by **compile-time enforcement** (docs/case-runner.md §Observability System).
> This document defines the mechanism: trait / derive / coverage assertions / skip declarations; for the observe output shape and the evaluation system, see
> `docs/case-runner.md` / `docs/case-eval-system.md`.

## Principles

> **Every module observable, enforced at compile time** — when adding a concept module, its observability must be declared, otherwise compilation fails; coverage does not rely on manual spot checks (effects once slipped through because of this).

> **Wholly mounted in an optional compilation unit** — the observability mechanism is enabled only under the observation build configuration; production builds are unaffected.

> **Minimal mechanism** — the derive mechanism only performs coverage assertions and does not generate assembly; assembly involving derived items remains hand-written (derived items do not map one-to-one to modules, so generating them would distort).

## Problem

**Add a field to Harness and it either implements Observable, or explicitly declares a skip with a reason; otherwise E0277** — coverage is enforced at compile time, not by manual spot checks.

## Mechanism

### trait Observable (Module Projection)

```rust
/// 可观测模块（core/src/observe.rs，cfg case-runner）
pub trait Observable {
    /// 快照投影类型（值语义，observe step 直接消费）
    type Snapshot;
    fn observe(&self) -> Self::Snapshot;
}
```

Each concept module implements its own projection: Queue → `Vec<QueueInput>`, Context → `Vec<MessageSnapshot>`, EventBuffer → `Vec<String>`, `Vec<AgentEntry>` → `Vec<AgentSnapshot>`, `Option<Usage>` → `Option<Usage>`, Memory → `Vec<MemoryNoteSnapshot>`, CronScheduler → `Vec<CronSnapshot>`.

> filtered_content **does not go through Observable** — the normalized full text has no module field (not persisted; computed on the spot by digesting the terminal-content.jsonl original text, docs/storage.md §filtered_content),
> and it belongs with panorama / answer / est_delta as derived items hand-assembled by case.rs.

### derive Observe (Aggregate Coverage Assertion)

Applies to **Harness** (the aggregate, not a module) and generates a coverage-assertion method:

```rust
impl Harness {
    fn __observe_coverage(&self) {
        fn require<T: ::ambery_core::observe::Observable>(_: &T) {}
        require(&self.queue);      // 每个非 skip 字段一行
        require(&self.context);
        // ...
    }
}
```

- A field type that does not implement `Observable` → **E0277**, with the error location pointing at the derive site (that field).
- Explicit skip: `#[observe(skip = "reason")]` — the reason is a required string, visible in review and searchable by grep.
- The derive macro name `Observe` (aggregate assertion) and the trait name `Observable` (module projection) are deliberately distinct.
- The macro expansion refers to `::ambery_core::observe::Observable`; inside core, the `extern crate self as ambery_core;` alias makes that path resolve both inside and outside the crate (the same technique serde uses).

### Harness Field Coverage Table

| Field | Disposition | Reason |
|------|------|------|
| queue | Observable | Concept §10c |
| context | Observable | Concept §10b |
| event_buffer | Observable | Concept §10d |
| agents | Observable | Concept §9 |
| last_usage | Observable | usage ground truth |
| last_head | skip | Assembly trail (head diff audit), not a concept module |
| last_usage_msg_len | skip | Base for est delta derivation (derived data, observed via context_est_delta computed on the spot) |
| last_usage_ts | skip | ts anchor of the usage row (derived data, observed synchronously with last_usage via the usage item) |
| memory | Observable | §10f persistent understanding buffer; observe gives an index summary (name / description / count), without expanding the body by default |
| cron | Observable | §10g persistent schedules and delayed dispatch; observe gives the schedule projection (id / schedule / message / next_due), excluding the sleep waiter |
| cards | Observable | Card registry (components.md §Card File); observe gives a registry projection (id/typ/title/created/user_closed/layout summary), without expanding the component |
| store | skip | JSONL persistence handle (mechanism, not a concept) |
| config_dir | skip | Path (mechanism, not a concept) |

> skip is not a loophole in the principle: it turns “unobserved” into an explicit declaration. Every concept module must implement
> Observable; the concrete observe output may be determined later, but that does not bypass the coverage constraint. skip is only for mechanism fields.

## Acceptance

- The `observe.rs` module documentation contains a `compile_fail,E0277` doctest: derive a struct with a field that does not implement Observable → compilation fails with error code exactly E0277 (zero dependencies, built into cargo test).
- Positive case: `case::observe()` output matches the projection contract field by field.

## Explicitly Out of Scope

- **Do not generate CaseObserve assembly**: panorama (derived from agents) / answer (derived from context) / context_est_delta (derived from usage landing points) do not map one-to-one to modules; hand-written assembly keeps them readable.
- **Do not expand Memory body or sleep waiter**: Memory observe gives only the index summary, to avoid spreading the full long-term understanding into case output; Cron observe gives only the persisted schedule projection, and the sleep waiter is an in-process, non-persistent mechanism. Both must implement Observable; the concrete fields follow the summary contract here.
- **The exhaustive-match projection for effects is not part of this foundation**: it belongs to the Effect action-stream module (docs/storage.md
  §effect.jsonl) and will plug in through the same Observable mechanism when implemented.
