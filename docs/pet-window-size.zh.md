# Pet Window Size 设计

> 本文档定 pet 窗口的尺寸公式与原则。

## 本文档范围

本文定义 pet 的尺寸、扫描与定位契约；表情池的编辑权限、整体更新与移动协议见 `docs/config.md` / `docs/autonomy.md`。

## 原则

1. **中心不变** — 窗口扩大缩小，视觉中心钉在同一点，`setSize` 后补偿左上角偏移
2. **纯函数** — 窗口尺寸 = f(baseline, scale, face, motion)，输入计算不读当前 OS 窗口大小
3. **障碍区固定** — 按所有 face/motion 的最坏情况预留，不随状态抖动，card/chat 布局稳定
4. **单向独立** — 上下左右四个方向各自取最大值，不绑定成单一的 H 和 W
5. **测量只测 face** — `getBoundingClientRect()` 只测 `#face` 当前渲染宽度，不测 `#view`（避免窗口约束闭环）；`#face` 必须 `flex-shrink: 0` 防止被容器压缩
6. **中心不离屏** — 仅在拖拽结束时校验 pet 中心必须落在某个显示器可用工作区内；若越界，拉回最近工作区的最近点。尺寸变化始终保持中心不变，不参与此边界修正。
7. **动画不改中心** — motion 的 CSS `transform` 仅在预留的窗口空间内暂时位移；动画首尾帧必回到基准位置，因此不改变 `petCenter`。拖拽、附属窗口跟随、障碍区定位与边界校验始终使用同一个基准中心。

## CSS ↔ JS 一致性契约

JS 公式必须与 CSS 布局等效。以下 CSS 值被 JS 直接读取或计算依赖，**修改 CSS 必须在注释标注的 token 处同步更新，否则窗口尺寸会错**：

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

动画 `@keyframes` 的极值也必须与 Motion 注册表一致（见下）。

## 设计常量（viewScale=1, dpr=1 时基底）

| 常量 | 值 | 来源 | 用途 |
|---|---|---|---|
| baselineH | 40px | CSS `#view height` | face 高度 + 安全边距（25px face 垂直居中，上下各 ~7.5px）；动画溢出在窗口层，容器无需为动画预留空间 |
| minFaceW | 72px | CSS `#view min-width` | 无 kaomoji 时的最小宽度 |
| padLR | 44px | CSS `#view padding-left + padding-right` | 左右内边距（22px × 2） |
| borderPx | 2px | CSS `#view border`（1px × 2 边，不随 scale） | 描边：白胶囊与背景区分；窗口公式补偿，防边缘 border 被窗口裁 |
| maxFaceWidth | 设计常量 | 扫描 `kaomoji.system` 渲染宽度取 max + 余量 | 障碍区宽度上限，超出 clip 并告警 |

## Motion 溢出预留（非硬编码常量）

每种 motion 定义时自带四向溢出，引擎扫描所有已注册 motion 取四个方向的最大值：

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

新增 motion 只加一条 MotionDef；CSS keyframes 同步写注释标注对应 overflow 值，若可一次播放则同步填写 `durationMs`。

## 一个公式

```
contextW = max(minFaceW, faceWidth) × scale + padLR × scale + borderPx
contextH = baselineH × scale + borderPx

w = contextW + motionLeft + motionRight
h = contextH + motionTop + motionBottom
```

- `faceWidth` = `#face.getBoundingClientRect().width`（CSS 像素）
- `scale` = `viewScale`（热更新；见 docs/view.md Config 字段）
- `motion{Left,Right,Top,Bottom}` = 当前 motion 的 `overflow[方向]`

**与 CSS 的对应关系**：
`contextW` = `#view` 不含 overflow 的自然宽度（CSS `min-width` + `padding` + flex content）
`contextH` = `#view` 不含 overflow 的自然高度（CSS `height`）

**障碍区**（一次注册，只随 scale/拖拽更新）：

```
obstacleW = max(minFaceW, maxFaceWidth) × scale + padLR × scale + ANIM_LEFT + ANIM_RIGHT
obstacleH = baselineH × scale + ANIM_TOP + ANIM_BOTTOM
```

## 六个入口

| # | 事件 | 动作 |
|---|---|---|
| 1 | face 变 | 重测 faceWidth → 算当前窗口尺寸 → setSize + 中心锚定 |
| 2 | scale 变 | 重算 → setSize + 中心锚定 |
| 3 | motion 变 | 取当前 motion 四向 overflow → 重算 → setSize + 中心锚定 |
| 4 | drag 结束 | 测 center → 更新引擎障碍区 |
| 5 | engine 初始注册 | 用障碍区尺寸（obstacleW/H）注册 pet 占区 |
| 6 | scale 变 | 同步更新引擎障碍区 |

## 依赖层次

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

单向依赖，无反馈回路。
