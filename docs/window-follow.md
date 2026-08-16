# Window Follow

English | [中文](window-follow.zh.md)

> For the layout algorithm (optimal direction), see docs/window-positioning.md. This document defines **coordinate system selection,
> responsibility layering, and follow/restore state semantics** — how the windows should move when pet moves, is dragged, or is hidden.

## Coordinate system: pet-relative

Inside the engine, **pet is fixed at (0,0)**; all windows store only the "offset relative to pet center (dx, dy)".

- The offset is an **invariant** of pet movement: pet movement changes only `petCenter`; the whole occupied set does not move.
  There is no translateAll, no snapshot, no stale state.
- Conversion in and out of the engine is unified at two points: returning screen coordinates `= petCenter + offset`;
  drag write-back `offset = screen center − petCenter`.
- Memory = the user's override intent for layout (relative offset), not the layout itself. Windows that have never been dragged have no memory;
  auto layout recomputes them.

## Responsibility layering

| Layer | Owns what | Key items |
|---|---|---|
| engine | **Pure position**: layout computation, offset storage | `occupied[]` (offset/size/manual), `place()`, `updateCenter()`, `restorePositions()` |
| Each window | **Visibility + user intent** | Private `userClosed` boolean |
| pet.ts | **Orchestration** | System-hide broadcast, restore broadcast (computes the new center) |

## State semantics

| State bit | When set | Effect |
|---|---|---|
| `manual` | User drag release (outerPosition writes back the offset) | `place()` does not rearrange and keeps the offset; pet movement follows with this offset as the basis |
| `userClosed` | User × / toggle close | The restore broadcast does **not** restore it (A semantics); toggle open resets it |

**Three kinds of hide**: the user actively closes (intent, sets userClosed), pet dragging temporarily hides (system hide, flags untouched),
tray group-hide (system hide, same as above). The latter two share the restore path: pet returns → broadcast the new center →
each window checks its own userClosed to decide whether to show.

#### ⟡ Consistency analysis

Except for pet, Chat and Card are uniformly Managed Surfaces (unified show / hide / restore semantics). The persistent truth of a Managed Surface falls into only four groups: identity, content, Surface intent (lifecycle and display choice), and spatial layout (direction and auto / manual offset); OS window handles, creating / closing, and temporary system hiding are all runtime and do not enter Surface. The positioning occupied region is not an implicit side effect of `hide()`; it is computed on demand from persistent intent: user hide releases the occupied region but keeps layout memory, temporary system hide keeps the occupied region, and dismiss ends the Surface and forgets its layout. Window kinds must be distinguished only by content source and lifecycle policy; they must not each invent their own hide / close / placement semantics. Cards Shelf is not a Managed Surface: it is a pet-anchored transient management popup (same class as Menu — closes on focus loss, closes with the pet-drag group hide, does not enter the engine occupied region, has no persistent spatial layout or display choice), and its persistent truth is not in a window but in the `.card.json` collection it manages.

## Restore: computed now, no snapshot

`restorePositions(petCenter)` = `occupied.map(o => petCenter + o.offset)`, computed at call time.
**No snapshot mechanism (hideAll/restoreAll)** — once offsets are the persistent truth, positions can be derived directly at any moment.

## Monitor geometry (monitor cache table)

The logical rectangles of all displays are **cached as a table and read once** (topology changes infrequently; no per-query reads):

```ts
monitors: [{ x, y, width, height, scaleFactor }]  // logical pixels (size/scaleFactor)
monitorOf(petCenter) → rect of the containing screen  // the only path for off-screen checks / height cap
```

- **Source**: Tauri uses `availableMonitors()` (all screens incl. scaleFactor); browser = viewport (`window.innerWidth/Height` — the card DOM's world is the viewport; using window.screen would misjudge off-viewport positions as visible)
- **Cache timing**: once at startup
- **Self-healing refresh**: pet center not inside any cached rect (screen change / topology change / hotplug) → automatically re-read, always converges
- **Unified consumption path**: `adapter.getScreenHeight()` is the only access point; its Tauri implementation reads this table internally — branches do not each call `window.screen` / `currentMonitor()` (see the "Adapter compliance" principle section)

## Coordinate unit contract

**engine world = environment frame** (Tauri = physical pixels, browser = CSS logical pixels) — the engine does not care about units internally,
only requiring full-pipeline consistency within one environment. Conversion is allowed only at two points:

| Point | Direction | Rule |
|---|---|---|
| Measurement entry (DOM → engine) | CSS → physical | DOM measurements (getBoundingClientRect / offsetWidth) **must be ×dpr before entering engine** (Tauri environment); browser (dpr=1) is naturally consistent |
| Adapter exit (engine → OS) | physical → OS | engine output goes directly to `setPosition/setSize` (PhysicalPosition/Size), with no second conversion |

Already observed: card-window measures `offsetWidth × dpr` ✓, `outerPosition` (physical) write-back ✓,
the monitors table stores raw physical rectangles ✓. **Forbidden**: DOM/CSS values (not ×dpr) written into the engine's
occupied/offset (the root cause of offset drift on mixed-DPI screens).

Logical-pixel needs (style caps such as cap = screen height × 0.5) do not belong to the engine world — they go through
the separate `adapter.getScreenHeight()` (logical pixels) channel, and are not mixed into engine coordinates.

## Off-screen and overlap

**Don't overlap others > fully visible**: prefer letting a card go partially off-screen; never do an after-the-fact clamp that "pulls it back on screen" —
an after-the-fact clamp looks only at the viewport, not at obstacles, and would pull the engine-computed position onto other windows (the overlap culprit:
positioning in chat.ts and component-manager.ts must not do an after-the-fact clamp).

**Fully missing** (the card rectangle has zero intersection with its screen) is not allowed; the fallback = **full 16-direction ring retry**
(zero algorithm-layer changes; computeCDSegments / ternarySearch are viewport-unaware):

```
place(dir) fully off-screen → retry along the 16-direction ring ±1, ±2 … ±8, stopping at the first "not fully off-screen"
(±1~±3 cover only the same hemisphere — when pet is against the bottom edge, the usable directions are in the northern hemisphere, so they are never tried);
all fail → accept the original-direction result (off-screen but honest, not overlapping others, no position correction).
```

**Partially visible is enough**: the retry acceptance criterion is "not fully off-screen", not fully inside the screen;
a direction change is triggered only by fully missing.

**Viewport units**: engine world = physical pixels (petCenter, window sizes, and setPosition are all physical);
the browser's "screen" = the browser viewport (the card DOM lives in the viewport, not the OS screen; using window.screen
would misjudge off-viewport positions as visible). Multi-screen semantics currently take the screen containing pet.

## Drag write-back

```
mousedown (except ×) → startDragging (OS modal, edges clamped)
onMoved debounced → outerPosition (the real position after OS clamping)
→ offset = (outerPos + size/2) − petCenter → updateCenter(id, offset) + manual=true
```

## Principle: single semantic source; branches only translate

**The same semantic may be implemented in only one place; branch paths (Tauri / browser / new shell) only translate events to
the unified API.** Better to merge branches into a shared implementation than to implement each separately — two implementations of the same semantic inevitably rot
and fork.

Corollaries:
- When adding a new shell branch, the first thing is to find the unified API for the semantic, not to rewrite it in the new shell
- When discovering "changed here but not there", the correct action is to merge into the single source, not to patch up the copy

## Principle: Adapter compliance — branches write no logic; shared logic sinks down

**Capabilities shared across windows / branches sink to a single point (Adapter layer / engine / shared module);
branch windows only consume the interface and do not each implement it.** A branch should contain only the thin event wiring specific to "this window";
geometry (screen rectangles, coordinate conversion), state (monitor cache, visibility semantics) — writing one copy of these shared things
in a branch is planting a fork landmine.

Concrete instances in this project:
- Screen geometry (monitor cache table, logical pixel conversion) = single point (Adapter/shared module); pet/card/chat
  all take it from there, and do not each call `window.screen` / `currentMonitor()`
- Window operations (size/position/show/hide) = WindowAdapter dual-mode single point; branches do not touch Tauri APIs directly
- Visibility semantics (intentClose/systemHide/systemRestore) = ChatPanel single point (see the section above)
- When scoping a new capability, first ask "which single point does it go in", not "which branch do I write it in"

## Two paths unified

Restore/hide semantics are **single-sourced** in ChatPanel's unified API; branches only translate events:

| API | Semantics | Tauri (chat-window) | browser (DOM ChatPanel) |
|---|---|---|---|
| `intentClose()` | User intent close: userClosed=true + hide | × / toggle close | × button |
| `intentOpen()` | User intent open: userClosed=false + show | toggle open | (right-click summons) |
| `systemHide()` | System hide: hide only, userClosed untouched | `chat:hide` event | view:drag-start |
| `systemRestore(center)` | System restore: reposition + show only if userClosed=false | `chat:show` event | view:moved |

Never implement restore rules separately in branches.
