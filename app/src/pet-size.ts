// Pet 窗口尺寸公式（docs/pet-window-size.md）：纯函数 + 设计常量。
// 窗口尺寸 = f(baseline, scale, face, motion)，输入计算不读当前 OS 窗口大小。
// ⚠ CSS ↔ JS 一致性契约：以下常量与 styles.css #view/#face 的标注 token 一一对应，
//   改 CSS 必须同步改这里，否则窗口尺寸会错。

import { ANIM_BOTTOM, ANIM_LEFT, ANIM_RIGHT, ANIM_TOP, type MotionOverflow } from "./motions";

/** CSS `#view height` 基底：face 高度 + 安全边距（25px face 垂直居中，上下各 ~7.5px）。
 *  动画溢出在窗口层（MotionDef overflow），容器无需为动画预留空间，故可保持紧凑 */
export const BASELINE_H = 40;
/** CSS `#view min-width` 基底：无 kaomoji 时的最小宽度 */
export const MIN_FACE_W = 72;
/** CSS `#view padding` 左右合计基底（22px × 2） */
export const PAD_LR = 44;
/** maxFaceWidth 余量（设计常量）：系统池扫描取 max 后加的防 clip 边距 */
export const MAX_FACE_MARGIN = 4;
/** #view 描边总宽（CSS border 1px × 2 边，不随 scale）：窗口公式补偿，防边缘 border 被窗口裁（#17 增补） */
export const BORDER_PX = 2;

export interface PetSize {
  w: number;
  h: number;
}

/** 内容区尺寸（不含 motion 溢出）：
 *  contextW = max(minFaceW, faceW) × scale + padLR × scale + 描边；contextH = baselineH × scale + 描边 */
export function contextSize(faceW: number, scale: number): PetSize {
  return {
    w: Math.max(MIN_FACE_W, faceW) * scale + PAD_LR * scale + BORDER_PX,
    h: BASELINE_H * scale + BORDER_PX,
  };
}

/** 窗口尺寸（§一个公式）：内容区 + 当前 motion 四向溢出（四向各自独立，不绑定成单一 H/W） */
export function windowSize(faceW: number, scale: number, overflow: MotionOverflow): PetSize {
  const c = contextSize(faceW, scale);
  return {
    w: c.w + overflow.left + overflow.right,
    h: c.h + overflow.top + overflow.bottom,
  };
}

/** 障碍区（一次注册，只随 scale/系统池扫描更新）：
 *  内容区最坏情况（maxFaceWidth）+ 所有已注册 motion 的四向最大溢出（MotionDef 扫描，不硬编码） */
export function obstacleSize(maxFaceW: number, scale: number): PetSize {
  return {
    w: Math.max(MIN_FACE_W, maxFaceW) * scale + PAD_LR * scale + BORDER_PX + ANIM_LEFT + ANIM_RIGHT,
    h: BASELINE_H * scale + BORDER_PX + ANIM_TOP + ANIM_BOTTOM,
  };
}
