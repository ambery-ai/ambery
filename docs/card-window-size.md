# Card Window Size

English | [中文](card-window-size.zh.md)

> - **Scope of this document** — This document defines how a Card window's size is determined: its single source, its projection into the Card file, the triggers that recompute it, and the path from projection to window creation. Coordinate units and display-scale conversion belong to [window-follow.md](window-follow.md); the Card file field list belongs to [components.md](components.md); pet window sizing belongs to [pet-window-size.md](pet-window-size.md).

## Definition

A Card window renders exactly one Card ([multi-window.md](multi-window.md)). Its size is the physical size of that OS window — not the size of the Card's content box: the Card's `min-width` / `max-width` are style caps inside the window, while the window is the box the user sees wrapping the Card.

## Single source

The window size is produced by a size computation whose inputs are the Card content, the font identity in effect, and the layout constants of the Card's rendering (font size, line height, padding, chrome). The computation is carried by `@chenglou/pretext`: `prepareWithSegments()` segments a block once and measures every segment on a canvas, caching the widths by (font, segment); `measureNaturalWidth()` gives the block's intrinsic width (no wrapping, hard breaks still split); `layout()` walks the segment advances and returns the line count for a given width and line height, with `height = lineCount × lineHeight`. Block heights plus the chrome constants give the window size.

The computation reads no geometry of the Card's rendered layout — no `getBoundingClientRect`, `offsetWidth`, `offsetHeight`, `scrollHeight` of the Card or its descendants — so its result does not depend on whether a window is currently showing the Card, or at what size. The only DOM access is pretext's emoji-width correction: for text that may contain emoji it compares the canvas-measured emoji width with a hidden inline-block span's width, once per font, and caches the difference. It never reads the Card.

Rendering consumes the size and never produces it. There is exactly one producer, so a Card window has one size definition instead of a computed one and a measured one.

## Input sources

| Input | Home |
|---|---|
| Card content | the Card file's `component` ([docs/components.md](components.md) §Card File (.card.json)) |
| Font identity | the Config font dimension (`ui.font`) — the resolved font family the user chose |
| Layout constants | the rendering contract: the Card's font size, weight, letter spacing, line height, padding, chrome and width bounds as declared by the Card styles and the component builder |

`layoutVersion` is the identity of the last two — the font identity plus the layout constants. A change in either invalidates every projection: every living Card is recomputed and its `layoutVersion` rewritten.

## Font identity

The Card font is a user choice, so the size computation assumes no font stack. Two constraints follow:

- **The font must be named.** A generic family (`sans-serif`, `system-ui`, `-apple-system`, `ui-sans-serif`) resolves to a different face per platform and per OS version, so a size computed with one is not reproducible even on a single machine. Such a family may render a Card, but it does not serve as a size authority.
- **The font must be loaded before measuring.** Canvas measurement silently substitutes a fallback face for a font that is not loaded, which would produce a size that does not match what is rendered. The computation therefore waits until the font is available, and a font that cannot be loaded is surfaced through the error channel ([docs/errors.md](errors.md)) instead of being measured as a fallback.

| Font class | Size authority | Reproducible across machines |
|---|---|---|
| Bundled with the app | yes | yes |
| Named system font present on the platform | yes | no — the same Card can size differently on another platform |
| Remote font, generic family, or a user-provided file that fails to load | no | no |

The measured metrics belong to the resolved face, not to the family list: the list is resolved before measuring and the first family present wins. A list whose leading family is absent silently measures with the next one, so one configuration can yield two different sizes on two machines ([docs/concrete-insight.md](concrete-insight.md) §Card Size Derivation).

Cross-machine reproducibility is a property, not a per-Card warning: a Card rendered in a font the app does not bundle can size differently on another machine, and that is expected. The settings surface where the font is chosen states this once; individual Cards do not warn.

## Content blocks

The Card content is measured block by block, because a Card body is a sequence of markdown blocks with different layout rules:

| Block | Height contribution |
|---|---|
| Heading, paragraph, list item, blockquote | line count × line height + block spacing |
| Fenced code block | line count × code line height; the block scrolls horizontally instead of wrapping |
| Table | per row, the tallest wrapped cell; column widths are the max-content widths of their cells, clamped to the Card's width cap |
| Horizontal rule | one fixed height |

Inline content (emphasis, strong, inline code, links) changes a line's width but not the block model. The Card's chrome — header, padding and border — is added once on top of the block total.

A Card body is assembled from a small, closed set of block types. Each type has one layout and one height rule and is computed on its own; a block does not mix layout kinds. Every block's height is derivable from its content and the layout constants — never from an element whose size the computation cannot predict. The rendering inside a block may use flex, grid or absolute positioning as long as that type's rule models the geometry. A block whose size comes from media declares that size — width with height, or width with aspect ratio — so the computation derives it without waiting for the load.

The Card body is capped at a fixed height — the same layout constant as its width cap — and content beyond the cap scrolls inside the Card. The cap does not depend on the display: a display-dependent cap would make the projection display-dependent, which this contract forbids.

Width is bounded, not derived freely: the Card's width is the widest of its blocks and its header, clamped between a minimum and a maximum width layout constant. Content narrower than the minimum still yields the minimum width; content wider than the maximum wraps at the maximum. The header title is a single non-wrapping line clipped with an ellipsis, so a long title never widens the Card past the maximum and always contributes exactly one line of height. There is no minimum height: a Card is as tall as its chrome plus its blocks.

## Projection

The computed size is projected into the Card file so that other components read it without running the computation: `_meta.layout.size` holds `[width, height]` in CSS pixels, and `_meta.layout.layoutVersion` holds the identity of the inputs that determine size (layout constants plus font identity) — a projection whose value differs from the current one is stale. The field list and file contract belong to [components.md](components.md) §Card File (.card.json); this document owns what the size field means.

Core is the only writer of the Card file; the frontend produces the size and requests the write through the Card write path.

`size` is derived state, not a user choice: unlike `_meta.layout.offset` and `_meta.layout.manual`, no user action edits it, and a user drag changes position only.

## Units and display scale

The projected size is stored in CSS pixels and is independent of display scale: the scale factor is not an input to the size computation, never appears in `layoutVersion`, and a Card moved to a display with a different scale factor keeps its projection unchanged.

Conversion to physical pixels happens once, at the boundary into the engine and the OS ([window-follow.md](window-follow.md) §Coordinate unit contract). The physical size is derived from the target display's scale factor — the display the Card window is placed on, not the Card window's own initial `devicePixelRatio` — and rounded up, so a fractional scale never yields a window smaller than its content. Placement and sizing are computed together and applied once the window is on its target display, so the physical size is never derived from the display the window happened to start on.

## Recompute triggers

| Input change | Recompute |
|---|---|
| Card content updated by the agent (same id) | yes |
| todobox item added by the user | yes |
| todobox item checked or unchecked | no — `done` changes, but no line count or width does |
| Card chrome text (UI language switch) | yes — button labels participate in width |
| font identity or layout constants | yes, for every Card |
| theme switch (colors, radii) | no — a theme is a purely visual change |

Recomputation has a single entry: every trigger above reaches the same function, and the resulting size is written in the same Card file write as the content that produced it, so a projection never pairs new content with a stale size. At startup every living Card's projection is recomputed; a projection whose `layoutVersion` already matches the current inputs is not rewritten.

## Window creation

A Card window is created at the size carried by the Card file's projection, read before the window is created. The Card window never resizes itself: it calls no `setSize`, and no post-show measurement loop exists. The create / reuse / close decision stays with the shell ([case-runner.md](case-runner.md) §Window decisions lifted up); this document owns only the size the window is created with.

## Relationship to pet sizing

Pet size is a closed formula over face width, scale and motion ([pet-window-size.md](pet-window-size.md)): the pet's content is fixed, so the formula is the whole contract. A Card's content is arbitrary, so its size is computed from that content. Both have a single source, and neither is measured from a live window.

## Verification

The size computation is rendering-side logic, so the case-runner does not exercise it: that harness observes frontend logic — the store, the action layer and window wiring ([case-runner.md](case-runner.md) §Observation boundary). This contract is verified against real rendering instead: a Card's derived size is compared with the size the same Card occupies when rendered, per type and across the content shapes that stress the block model. The comparison runs in the engine the Card renders in — the line walk's fit tolerance and a few break behaviours are calibrated per engine — so the tolerance is the engine's own rather than one number for every platform.
