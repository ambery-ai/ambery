# Pet Window Size Design

> This document defines the size formula and principles of the pet window.

## Scope of This Document

This document defines pet's size, scanning, and positioning contract; for the emoji pool's editing permissions, overall updates, and movement protocol, see `docs/config.md` / `docs/autonomy.md`.

## Principles

1. **Center invariant** — when the window grows or shrinks, the visual center stays pinned at the same point; after `setSize`, the top-left offset is compensated.
2. **Pure function** — window size = f(baseline, scale, face, motion); the computation does not read the current OS window size.
3. **Fixed obstacle area** — reserved by the worst case across all face/motion, so it does not jitter with state and Card/chat layout stays stable.
4. **Independent per direction** — the four directions top/bottom/left/right each take their own maximum, not bound into a single H and W.
5. **Measure only face** — `getBoundingClientRect()` measures only the current rendered width of `#face`, not `#view` (avoiding a window-constraint feedback loop); `#face` must be `flex-shrink: 0` to prevent the container from compressing it.
6. **Center stays on screen** — only at drag end is it verified that the pet center falls within some monitor's available work area; if out of bounds, it is pulled back to the nearest point of the nearest work area. Size changes always keep the center unchanged and do not participate in this boundary correction.
7. **Animation does not move the center** — the motion CSS `transform` only displaces temporarily within the reserved window space; the first and last animation frames always return to the base position, so `petCenter` does not change. Dragging, attached-window following, obstacle-area positioning, and boundary validation always use the same base center.

## CSS ↔ JS Consistency Contract

The JS formula must be equivalent to the CSS layout. The following CSS values are either read directly by JS or relied upon by its computation; **when changing CSS, the JS must be updated in sync at the token marked by the comment, otherwise the window size will be wrong**:

```css
#view {
  --view-scale: 1;
  position: fixed;
  min-width: calc(72px * var(--view-scale));      /* ← JS: minFaceW 基底 */
  height: calc(40px * var(--view-scale));          /* ← JS: baselineH 基底 */
  padding: 0 calc(22px * var(--view-scale));       /* ← JS: padLeft/Right 基底 */
  display: flex;
  align-items: center;
  justify-content: center;
}

#face {
  font-size: calc(25px * var(--view-scale));
  line-height: 1;                                  /* ← JS: height 由 line-height=1 保证 */
  white-space: nowrap;
  flex-shrink: 0;                                  /* 必须：禁止被容器压缩，确保测量为自然宽度 */
  pointer-events: none;
}
```

The extremes of the `@keyframes` animation must also match the Motion registry (see below).

## Design Constants (Base Values at viewScale=1, dpr=1)

| Constant | Value | Source | Purpose |
|---|---|---|---|
| baselineH | 40px | CSS `#view height` | face height + safety margin (25px face centered vertically, ~7.5px top and bottom); animation overflow lives at the window layer, so the container does not need to reserve space for animation |
| minFaceW | 72px | CSS `#view min-width` | Minimum width when there is no kaomoji |
| padLR | 44px | CSS `#view padding-left + padding-right` | Left/right padding (22px × 2) |
| borderPx | 2px | CSS `#view border` (1px × 2 sides, does not scale) | Stroke: separates the white capsule from the background; the window formula compensates so the edge border is not clipped by the window |
| maxFaceWidth | Design constant | Max of scanned `kaomoji.system` rendered widths + margin | Obstacle-area width cap; anything beyond is clipped and warned |

## Motion Overflow Reserve (Not a Hard-Coded Constant)

Each motion definition carries its own four-direction overflow; the engine scans all registered motions and takes the maximum in each of the four directions:

```ts
type MotionDef = {
  motion: Motion;
  overflow: { top: number; bottom: number; left: number; right: number };
  durationMs?: number; // 一次动作时的 TTL；必须与 CSS animation-duration 一致
  // ↑ overflow 必须与 CSS @keyframes 的 translateX/Y 极值一致
};

const MOTIONS: MotionDef[] = [
  { motion: "still",  overflow: { top:  0, bottom:  0, left:  0, right:  0 } },
  { motion: "bounce", overflow: { top: 18, bottom:  0, left:  0, right:  0 }, durationMs:  900 }, // ← CSS: translateY(-18px), 0.9s
  { motion: "float",  overflow: { top: 10, bottom:  0, left:  0, right:  0 }, durationMs: 4000 }, // ← CSS: translateY(-10px), 4s
  { motion: "shake",  overflow: { top:  0, bottom:  0, left:  6, right:  6 }, durationMs:  400 }, // ← CSS: translateX(±6px), 0.4s
];

// 四向各自的最大值（跑一遍 MOTIONS 即得）
const ANIM_TOP    = Math.max(...MOTIONS.map(m => m.overflow.top));
const ANIM_BOTTOM = Math.max(...MOTIONS.map(m => m.overflow.bottom));
const ANIM_LEFT   = Math.max(...MOTIONS.map(m => m.overflow.left));
const ANIM_RIGHT  = Math.max(...MOTIONS.map(m => m.overflow.right));
```

To add a new motion, just add one MotionDef; in CSS keyframes, write a matching comment marking the corresponding overflow value, and if it plays in one pass, fill in `durationMs` accordingly.

## One Formula

```
contextW = max(minFaceW, faceWidth) × scale + padLR × scale + borderPx
contextH = baselineH × scale + borderPx

w = contextW + motionLeft + motionRight
h = contextH + motionTop + motionBottom
```

- `faceWidth` = `#face.getBoundingClientRect().width` (CSS pixels)
- `scale` = `viewScale` (hot-updated; see the Config field in docs/view.md)
- `motion{Left,Right,Top,Bottom}` = the current motion's `overflow[direction]`

**Correspondence with CSS**:
`contextW` = the natural width of `#view` excluding overflow (CSS `min-width` + `padding` + flex content)
`contextH` = the natural height of `#view` excluding overflow (CSS `height`)

**Obstacle area** (registered once; updated only on scale/drag):

```
obstacleW = max(minFaceW, maxFaceWidth) × scale + padLR × scale + ANIM_LEFT + ANIM_RIGHT
obstacleH = baselineH × scale + ANIM_TOP + ANIM_BOTTOM
```

## Six Entry Points

| # | Event | Action |
|---|---|---|
| 1 | face changes | Re-measure faceWidth → compute current window size → setSize + center anchoring |
| 2 | scale changes | Recompute → setSize + center anchoring |
| 3 | motion changes | Take the current motion's four-direction overflow → recompute → setSize + center anchoring |
| 4 | drag ends | Measure center → update engine obstacle area |
| 5 | engine initial registration | Register pet's occupied area with the obstacle-area size (obstacleW/H) |
| 6 | scale changes | Update engine obstacle area in sync |

## Dependency Layers

```
Layer 0: 设计基底（baselineH / minFaceW / padLR）
    ↓
Layer 1: 测量层 faceWidth = measure(#face)     ← flex-shrink:0 保证自然宽度
    ↓
Layer 2: Scale 层（× viewScale）
    ↓
Layer 3: Motion 溢出层（+ 当前 motion 四向值，取自 MotionDef）
    ↓
Layer 4: 窗口尺寸（纯函数，四向各自独立）
    ↓
Layer 5: 中心锚定（先算新 center = old center，再反推新左上角）
    ↓
Layer 6: 障碍区 = Layer4 在所有 face × motion 笛卡尔积上的 max
         ANIM_* 由 MotionDef 扫描得出，不硬编码
```

One-way dependencies, no feedback loops.
