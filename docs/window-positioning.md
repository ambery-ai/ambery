# Window Positioning

## Overview

The engine computes the unique optimal direction relative to pet for all non-pet windows (ChatPanel, Component Cards). Core principles:

- **Closest**: as close to pet as possible
- **No overlap**: the new window does not overlap pet or any existing window
- **Optimal**: subject to the first two, the direction is as close as possible to the desired direction

The engine accepts a preferred direction (one of the 16 directions, see below) and returns the world coordinate of the new window center.

---

## Coordinate system and modeling

### Symbols

| Symbol | Meaning |
|---|---|
| `A` | pet center point (fixed) |
| `B` | new window center (to be found) |
| `m⃗` | unit vector of the preferred direction |
| `θ` | `∠(m⃗, AB⃗)`, the angle between the two vectors, range `[0, π]` |
| `|AB|` | Euclidean distance from pet center to new window center |

### Value function

```
V(B) = α · θ² + β · |AB|    (α, β > 0)
```

- `α ≫ β` (direction priority is overwhelmingly higher than distance); exact values are tuned by the implementation
- debug mode can override `α` and `β` via DevTools or an endpoint
- `gap`: minimum spacing between windows, default 12px, configurable
- the minimum point = the globally unique optimum

---

## Monotonicity: unique minimum

Constrain B to a segment `CD` (a straight segment of length `l`), with parameter `t ∈ [0, l]` giving B's position. Assume A is not on line CD.

Derivation gives (see appendix):

```
f'(t) = ψ'(t) · F(t)
F(t) = 2α(ψ(t) − c) + (β/d) · (t−a)·√((t−a)²+d²)
F'(t) = 2α·ψ'(t) + (β/d) · (2(t−a)²+d²)/√((t−a)²+d²)  > 0
```

**Conclusion**: `F(t)` is strictly increasing → `f'(t)` has at most one zero → `f(t)` is unimodal → each edge line has a unique minimum (an interior stationary point or an endpoint).

---

## Edge decomposition

The engine decomposes the usable space around pet into **edge segments (CD segments)**, forming the search domain.

### Forbidden-region expansion

Let the new window size be `W_new × H_new`. `gap` is the minimum spacing (default 12px).

Each existing window (including pet) takes its BBox as the base and is **expanded outward on all four sides**:

```
outerX = gap + W_new / 2
outerY = gap + H_new / 2
```

The expanded rectangle is the forbidden region — the new window **center** must not fall inside it.

### Union outer boundary and CD segment extraction

1. Collect the forbidden-region rectangles of all existing windows (including pet)
2. The union of all forbidden-region rectangles = the obstacle polygon
3. Extract the **outer boundary** of the obstacles
4. Each **convex edge segment** on the outer boundary = one legal CD segment

When the new window center lies on a CD segment, the window edge just touches the nearest obstacle at exactly `gap`, with no overlap.

---

## Ternary search

On each CD segment, f(t) is unimodal → ternary search converges to the local minimum.

```
function ternarySearch(CD, A, mPref, α, β, tol):
    l, r = 0, |CD|
    while r - l > tol:
        m1 = l + (r-l)/3
        m2 = r - (r-l)/3
        if V(m1) < V(m2): r = m2
        else:             l = m1
    return V((l+r)/2)
```

The full search domain = all CD segments. Local minimum on each CD segment → the global min is the minimum across all segments. The corresponding `B` coordinate is the unique optimal direction.

---

## Engine interface

```typescript
interface PositioningEngine {
  /** Register a window (size), return the optimal position; "auto" computes from the screen's largest remaining-space direction */
  place(window: WindowSpec, preferred: Direction | "auto"): Point;

  /** dismiss: forget the occupied region and layout memory together */
  remove(windowId: string): void;

  /** User hide: release the occupied region but keep layout memory (re-open place restores the original position) */
  release(windowId: string): void;

  /** Drag end writes the real position back as the new follow basis (manual flag) */
  updateCenter(windowId: string, center: Point): Point | null;

  /** Restore coordinates: compute petCenter + offset now (no snapshot) */
  restorePositions(petCenter: Point): { id: string; center: Point }[];
}

interface WindowSpec {
  id: string;
  width: number;   // declared window size
  height: number;
  gap?: number;     // custom spacing, default 12 (constant DEFAULT_GAP, no global config field)
}
```

### Hide/restore

- pet moves (drag start): the whole layer system-hides — occupied regions stay in place, no snapshot;
- pet move ends: compute `pet_new + offset` to restore (restorePositions) — offset is pet-relative and naturally translates with pet; no record-replay needed;
- Drag write-back: on release, `updateCenter` converts the OS real position into the new follow basis (manual flag); subsequent pet movement follows from the dragged position.

### Direction enum

16 direction names in English, 0 = top (north), increasing clockwise by 22.5°:

```
enum Direction {
  n, nne, ne, ene,     // 0-3
  e, ese, se, sse,     // 4-7
  s, ssw, sw, wsw,     // 8-11
  w, wnw, nw, nnw,     // 12-15
}
```

| Value | Name | Angle | Description |
|---|---|---|---|
| 0 | `n` | 0° | directly above |
| 4 | `e` | 90° | directly right |
| 7 | `sse` | 157.5° | ChatPanel default |
| 8 | `s` | 180° | directly below |
| 12 | `w` | 270° | directly left |

### ChatPanel default

ChatPanel's preferred direction is fixed to **`sse`** (docs/chat-panel.md §Layout; 0 = top, clockwise). The layout is slightly offset to match visual habit.

### gap configuration

Minimum spacing between windows (px): `WindowSpec.gap` can override per window; the global default = the `DEFAULT_GAP` constant 12 (positioning/engine.ts; currently no Config field).

---

## Algorithm flow

```
Input: petCenter, newWindow{size, preferred}, occupiedWindows[]

1. Compute forbidden regions: expand the BBox of pet + occupiedWindows outward on all four sides
2. Take the union outer boundary of forbidden regions → CD segment list
3. for each CD segment:
     ternarySearch(CD, petCenter, preferredAngle, α, β)
4. Take the global minimum among all local minima → B coordinate
5. Return B
```

Time complexity: O(k · log(1/ε)), where k = number of CD segments (≤ 2 × occupiedCount).

---

## Appendix: monotonicity derivation of the value function

Place segment CD on the x-axis, `C = (0,0)`, `D = (l,0)`. `B(t) = (t, 0)`.

`A = (a, d)`, `d > 0` (A is not on CD).

```
|AB| = d(t) = √((t−a)² + d²)
ψ(t) = atan2(−d, t−a)    // direction angle of AB, monotonically increasing (ψ'(t) > 0)
θ(t) = |ψ(t) − c|         // angle from the preferred direction angle c
f(t) = α·θ² + β·d(t)
```

```
f'(t) = 2αθ(t)θ'(t) + β·d'(t)
      = ψ'(t) [2α(ψ(t)−c) + β·d'(t)/ψ'(t)]
```

From `d'(t) = (t−a)/d(t)` and `ψ'(t) = d/d(t)²`, we get `d'(t)/ψ'(t) = (t−a)·d(t)/d`.

Let `F(t) = 2α(ψ(t)−c) + (β/d)·(t−a)·d(t)`.

`F'(t) = 2α·ψ'(t) + (β/d)·(2(t−a)²+d²)/d(t)` > 0 (all terms positive).

→ `F(t)` is strictly increasing → `f'(t)` has at most one zero → `f(t)` is unimodal → unique minimum (an interior stationary point or an endpoint).
