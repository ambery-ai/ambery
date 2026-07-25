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

/** 两个矩形并集的外边界（取包围盒），简化版 */
function unionRect(a: Rect, b: Rect): Rect {
  return {
    minX: Math.min(a.minX, b.minX),
    minY: Math.min(a.minY, b.minY),
    maxX: Math.max(a.maxX, b.maxX),
    maxY: Math.max(a.maxY, b.maxY),
  };
}

/** 给定障碍物矩形，提取新窗口中心可放置的四边 CD 段 */
function rectToSegments(r: Rect): [Point, Point][] {
  const segs: [Point, Point][] = [];
  segs.push([{ x: r.minX, y: r.minY }, { x: r.maxX, y: r.minY }]); // top
  segs.push([{ x: r.minX, y: r.maxY }, { x: r.maxX, y: r.maxY }]); // bottom
  segs.push([{ x: r.minX, y: r.minY }, { x: r.minX, y: r.maxY }]); // left
  segs.push([{ x: r.maxX, y: r.minY }, { x: r.maxX, y: r.maxY }]); // right
  return segs;
}

/**
 * 计算新窗口可放置的 CD 段列表。
 * @param petCenter  pet 中心
 * @param petSize    pet 宽高
 * @param newWindow  新窗口规格
 * @param occupied   已有窗口的中心 + 宽高
 * @param gap        最小间距
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

  // 所有障碍物并集
  let union: Rect = expandWindow(petCenter, petSize.w, petSize.h, outX, outY);
  for (const oc of occupied) {
    union = unionRect(union, expandWindow(oc.center, oc.w, oc.h, outX, outY));
  }

  return rectToSegments(union);
}
