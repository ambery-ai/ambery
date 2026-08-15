# Autonomy Design

> Concept definition: see concepts.md §4. This document fixes the expression model, the default mapping table, and override semantics.

## Scope

This document defines expression states, default derivation, override semantics, and kaomoji pool resolution; for general Config validation and persistence mechanisms, see `docs/config.md`.

## Expression model

pet's external appearance = `Expression { face: string, motion: Motion }`, `Motion = still | float | bounce | shake`.

- `face`: kaomoji (emoticon), rendered inside the View.
- `motion`: the View's motion pattern, implemented with CSS animation; the two runtime modes (Tauri/browser) are identical.

## Config fields

| field | effective | agent access | behavior |
|---|---|---|---|
| `kaomoji.system` | hot | visible, modifiable; by default do not modify | from the next runtime operation, participates in the two-pool union resolution of the default state and `set_autonomy(key)`; immediately re-scans the system pool and recomputes pet size and the fixed obstacle area; falls back to the default state when the current override or state key no longer exists |
| `kaomoji.user` | hot | visible, modifiable | from the next runtime operation, participates in the two-pool union resolution of the default state and `set_autonomy(key)`; falls back to the default state when the current override or state key no longer exists |
| `set_autonomy_default_ttl_ms` | hot | visible, modifiable | the next time `set_autonomy` omits `ttlMs`, the new default value is used (the frontend reads the runtime projection) |

## Two-path control

1. **Default behavior (without the LLM)**: outputs the current expression and motion according to the kaomoji mapping table. Rules (priority from high to low):

   | condition | state key | default face | default motion |
   |---|---|---|---|
   | has pending notifications | `notify` | `✧*｡٩(ˊᗜˋ*)و✧*｡` | bounce |
   | any instance Processing | `processing` | `(ˇωˇ」∠)_` | float |
   | otherwise (all Idle / no instances) | `idle` | `(´ω`)` | still |

   The mapping table is stored in Config's two pools `kaomoji.system` and `kaomoji.user`; it is uniquely resolved by key in the union of the two pools. `idle` / `processing` / `notify` must exist in the union; both the system default derivation and `set_autonomy(key)` resolve against it. Both pools can be managed by the agent via `query(view=object) → update(完整 map)`; by default do not modify the system pool, and it is the source for size scanning. Cross-pool validation is in docs/config.md. The state keys match the concepts §4 examples: Processing → `(ˇωˇ」∠)_` + slow floating, notification → `✧*｡٩(ˊᗜˋ*)و✧*｡` + bouncing.

2. **pet-initiated override**: the `set_autonomy` tool call.

## set_autonomy semantics

```
set_autonomy(key?: string, motion?: Motion, ttlMs?: number, once?: boolean)
```

- `key` may be a state key name (`idle`/`processing`/`notify`/custom key): it resolves in the union of the two pools to the mapping-table entry itself (only face is resolved; motion is not carried along).
- Only the fields actually passed are overridden; the rest keep their default output.
- When `ttlMs` is omitted, it defaults to 60000ms; after TTL expiry, output falls back to the default.
- With `once: true`, the duration is taken automatically from `MotionDef.durationMs`; like motion's four-direction overflow, it belongs to the animation registry and must stay in sync with CSS `animation-duration`. The animation CSS still loops; after TTL expiry it falls back to the default state, thereby converging into a one-shot action.
- `once: true` together with an explicit `ttlMs` is rejected outright, to avoid two conflicting sets of duration semantics.
- All parameters omitted (or `ttlMs: 0`) → immediately clear the override and fall back to the default.
- During an override, instance state changes do not interrupt the override (pet's expression takes precedence); after TTL expiry it falls back to the default.

## Relationship to the LLM side

Autonomy is its own engine, independent of AmberyBackend. The state format `[face: key, motion: key]`, about 6-7 tokens, is appended to the end of each request. It persists in Context (one record per turn), not in Queue.

Note: the LLM perceives Code CLI instance state changes through Context diff events (see docs/harness.md); the two have different data sources and are mutually independent — do not confuse them.
