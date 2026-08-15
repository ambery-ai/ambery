// Motion 注册表（非硬编码常量）：
// 每种 motion 定义时自带四向溢出与一次播放时长；尺寸引擎扫描所有已注册 motion
// 取四个方向的最大值作障碍区预留，set_autonomy(once:true) 从 durationMs 取 TTL。
//
// ⚠ CSS ↔ JS 一致性契约：
//   overflow 必须与 styles.css @keyframes 的 translateX/Y 极值一致；
//   durationMs 必须与 styles.css 的 animation-duration 一致。
//   新增 motion 只加一条 MotionDef；CSS keyframes 同步写注释标注对应值。

import type { Motion } from "./bridge";

export interface MotionOverflow {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

export interface MotionDef {
  motion: Motion;
  /** 四向溢出（CSS px，与未缩放的 @keyframes 极值一致，不随 viewScale 缩放） */
  overflow: MotionOverflow;
  /** 一次动作的持续时间（once:true 时作 TTL）；必须与 CSS animation-duration 一致 */
  durationMs?: number;
}

export const MOTIONS: MotionDef[] = [
  { motion: "still", overflow: { top: 0, bottom: 0, left: 0, right: 0 } },
  // ← CSS: translateY(-18px), 0.9s
  { motion: "bounce", overflow: { top: 18, bottom: 0, left: 0, right: 0 }, durationMs: 900 },
  // ← CSS: translateY(-10px), 4s
  { motion: "float", overflow: { top: 10, bottom: 0, left: 0, right: 0 }, durationMs: 4000 },
  // ← CSS: translateX(±6px), 0.4s
  { motion: "shake", overflow: { top: 0, bottom: 0, left: 6, right: 6 }, durationMs: 400 },
];

/** 四向各自的最大值（跑一遍 MOTIONS 即得，不硬编码） */
export const ANIM_TOP = Math.max(...MOTIONS.map((m) => m.overflow.top));
export const ANIM_BOTTOM = Math.max(...MOTIONS.map((m) => m.overflow.bottom));
export const ANIM_LEFT = Math.max(...MOTIONS.map((m) => m.overflow.left));
export const ANIM_RIGHT = Math.max(...MOTIONS.map((m) => m.overflow.right));

/** 查 motion 定义；未知 motion 回落 still（与 view.ts 无动画默认一致） */
export function motionDef(motion: Motion): MotionDef {
  return MOTIONS.find((m) => m.motion === motion) ?? MOTIONS[0];
}
