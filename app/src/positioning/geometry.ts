// positioning/geometry — obstacle expansion + CD segment extraction (stateless)

import type { Point, WindowSpec } from "./types";

interface Rect {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** 已有窗口 BBox 四向外扩 gap + newWinHalfSize → 禁止区矩形 */
function expandWindow(
  center: Point,
  w: number,
  h: number,
  outX: number,
  outY: number,
): Rect {
  return {
    minX: center.x - w / 2 - outX,
    minY: center.y - h / 2 - outY,
    maxX: center.x + w / 2 + outX,
    maxY: center.y + h / 2 + outY,
  };
}

/** 水平线段 [x1,x2] 在固定 y */
interface HSeg { x1: number; x2: number; y: number; }
/** 垂直线段 [y1,y2] 在固定 x */
interface VSeg { y1: number; y2: number; x: number; }

/**
 * 从一条水平线段中减去矩形 r 的投影覆盖部分，返回剩余段（0-2 个）
 */
function subtractHSeg(seg: HSeg, r: Rect): HSeg[] {
  if (seg.y < r.minY || seg.y > r.maxY) return [seg]; // 不在矩形高度内
  const cx1 = Math.max(seg.x1, r.minX);
  const cx2 = Math.min(seg.x2, r.maxX);
  if (cx1 >= cx2) return [seg]; // 无重叠
  const remain: HSeg[] = [];
  if (seg.x1 < cx1) remain.push({ x1: seg.x1, x2: cx1, y: seg.y });
  if (cx2 < seg.x2) remain.push({ x1: cx2, x2: seg.x2, y: seg.y });
  return remain;
}

/** 同步垂直线段版 */
function subtractVSeg(seg: VSeg, r: Rect): VSeg[] {
  if (seg.x < r.minX || seg.x > r.maxX) return [seg];
  const cy1 = Math.max(seg.y1, r.minY);
  const cy2 = Math.min(seg.y2, r.maxY);
  if (cy1 >= cy2) return [seg];
  const remain: VSeg[] = [];
  if (seg.y1 < cy1) remain.push({ y1: seg.y1, y2: cy1, x: seg.x });
  if (cy2 < seg.y2) remain.push({ y1: cy2, y2: seg.y2, x: seg.x });
  return remain;
}

function segLen(C: Point, D: Point): number {
  return Math.abs(D.x - C.x) + Math.abs(D.y - C.y);
}

/**
 * 计算新窗口可放置的 CD 段列表（正交矩形并集外轮廓，精确）。
 */
export function computeCDSegments(
  petCenter: Point,
  petSize: { w: number; h: number },
  newWindow: WindowSpec,
  occupied: { center: Point; w: number; h: number }[],
  gap: number,
): [Point, Point][] {
  const gapN = newWindow.gap ?? gap;
  const outX = gapN + newWindow.width / 2;
  const outY = gapN + newWindow.height / 2;

  const obstacles: Rect[] = [expandWindow(petCenter, petSize.w, petSize.h, outX, outY)];
  for (const oc of occupied) {
    obstacles.push(expandWindow(oc.center, oc.w, oc.h, outX, outY));
  }

  // 收集所有水平/垂直原始边，同时记录源障碍物
  let hSegs: (HSeg & { src: number })[] = [];
  let vSegs: (VSeg & { src: number })[] = [];
  for (let i = 0; i < obstacles.length; i++) {
    const obs = obstacles[i];
    hSegs.push({ src: i, x1: obs.minX, x2: obs.maxX, y: obs.minY });
    hSegs.push({ src: i, x1: obs.minX, x2: obs.maxX, y: obs.maxY });
    vSegs.push({ src: i, y1: obs.minY, y2: obs.maxY, x: obs.minX });
    vSegs.push({ src: i, y1: obs.minY, y2: obs.maxY, x: obs.maxX });
  }

  // 每条边减去所有其他障碍物（跳过来源，结果继承 src）
  for (let i = 0; i < obstacles.length; i++) {
    const obs = obstacles[i];
    hSegs = hSegs.flatMap((s) =>
      s.src === i ? [s] : subtractHSeg(s, obs).map((r) => ({ ...r, src: s.src })));
    vSegs = vSegs.flatMap((s) =>
      s.src === i ? [s] : subtractVSeg(s, obs).map((r) => ({ ...r, src: s.src })));
  }

  // 过滤 0 长度段，转为 Point 对
  const result: [Point, Point][] = [];
  for (const s of hSegs) {
    const C = { x: s.x1, y: s.y };
    const D = { x: s.x2, y: s.y };
    if (segLen(C, D) > 1) result.push([C, D]);
  }
  for (const s of vSegs) {
    const C = { x: s.x, y: s.y1 };
    const D = { x: s.x, y: s.y2 };
    if (segLen(C, D) > 1) result.push([C, D]);
  }
  return result;
}
