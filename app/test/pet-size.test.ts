// pet 窗口尺寸公式：scale 有效性矩阵 + CSS↔JS 契约锁定。
// 纯函数测试（无 DOM、无 core）：直接跑 vitest，不需 case-runner。
// 契约：常量与 styles.css #view/#face 的标注 token 一一对应（pet-size.ts 头部 ⚠ 注释）。

import { describe, expect, it } from "vitest";
import {
  BASELINE_H,
  BORDER_PX,
  contextSize,
  MAX_FACE_MARGIN,
  MIN_FACE_W,
  obstacleSize,
  PAD_LR,
  windowSize,
} from "../src/pet-size";
import { MOTIONS, type MotionOverflow } from "../src/motions";

const STILL: MotionOverflow = { top: 0, bottom: 0, left: 0, right: 0 };

describe("CSS↔JS 契约：常量与 styles.css token 一致", () => {
  it("设计基底（viewScale=1 基准）", () => {
    expect(BASELINE_H).toBe(20); // #view height
    expect(MIN_FACE_W).toBe(36); // #view min-width
    expect(PAD_LR).toBe(22); // #view padding 11px × 2
    expect(BORDER_PX).toBe(2); // #view border 1px × 2（不随 scale）
    expect(MAX_FACE_MARGIN).toBe(4); // maxFaceWidth 防 clip 余量
  });
});

describe("scale 有效性矩阵", () => {
  // view_scale 值域 0.2–4.0（core config schema Range）
  const SCALES = [0.2, 0.5, 1, 1.5, 2, 4];
  // 短/中/长颜文字（11.25px 字体下 idle 实测 ~47px；取 40/85/140 覆盖 <min / 常规 / 超长）
  const FACES = [0, 40, 85, 140];

  it("公式精确值：contextW = max(min,face)×scale + pad×scale + border", () => {
    for (const scale of SCALES) {
      for (const face of FACES) {
        const c = contextSize(face, scale);
        expect(c.w).toBe(Math.max(MIN_FACE_W, face) * scale + PAD_LR * scale + BORDER_PX);
        expect(c.h).toBe(BASELINE_H * scale + BORDER_PX);
        expect(c.w).toBeGreaterThan(0);
        expect(c.h).toBeGreaterThan(0);
      }
    }
  });

  it("随 scale 单调不减（face 固定）", () => {
    for (const face of FACES) {
      let prevW = -1;
      let prevH = -1;
      for (const scale of SCALES) {
        const c = contextSize(face, scale);
        expect(c.w).toBeGreaterThanOrEqual(prevW);
        expect(c.h).toBeGreaterThanOrEqual(prevH);
        prevW = c.w;
        prevH = c.h;
      }
    }
  });

  it("卡点语义：faceW < minFaceW 时用 minFaceW（所有 scale）", () => {
    for (const scale of SCALES) {
      const short = contextSize(10, scale); // face 10 < min 36
      const atMin = contextSize(MIN_FACE_W, scale);
      expect(short.w).toBe(atMin.w);
    }
  });

  it("windowSize = contextSize + 当前 motion 四向溢出", () => {
    for (const m of MOTIONS) {
      for (const scale of SCALES) {
        const w = windowSize(85, scale, m.overflow);
        const c = contextSize(85, scale);
        expect(w.w).toBe(c.w + m.overflow.left + m.overflow.right);
        expect(w.h).toBe(c.h + m.overflow.top + m.overflow.bottom);
      }
    }
  });

  it("障碍区 ≥ 内容区（防 clip），且覆盖所有 motion 最大溢出", () => {
    for (const scale of SCALES) {
      const ob = obstacleSize(140, scale);
      for (const m of MOTIONS) {
        const w = windowSize(140, scale, m.overflow);
        expect(ob.w).toBeGreaterThanOrEqual(w.w);
        expect(ob.h).toBeGreaterThanOrEqual(w.h);
      }
      expect(ob.w).toBeGreaterThan(0);
      expect(ob.h).toBeGreaterThan(0);
    }
  });

  it("scale=1 基准：idle 短颜文字窗口 ≥ 可点下限（高 ≥ 20、宽 ≥ 50）", () => {
    const w = windowSize(40, 1, STILL);
    expect(w.h).toBeGreaterThanOrEqual(20);
    expect(w.w).toBeGreaterThanOrEqual(50);
  });
});
