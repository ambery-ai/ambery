// positioning/types — shared types for window positioning engine

/** 16 方位，0 = 顶（north），顺时针 22.5° 递增 */
export enum Direction {
  n = 0, nne = 1, ne = 2, ene = 3,
  e = 4, ese = 5, se = 6, sse = 7,
  s = 8, ssw = 9, sw = 10, wsw = 11,
  w = 12, wnw = 13, nw = 14, nnw = 15,
}

export function directionAngle(d: Direction): number {
  // 0=n=顶=270°, 顺时针 22.5° 递增
  return ((d * 22.5 + 270) * Math.PI) / 180;
}

/** 方位名 → 枚举值 */
export function directionFromName(name: string): Direction | null {
  const idx = Object.values(Direction).indexOf(name as any);
  return idx >= 0 ? idx : null;
}

export interface Point {
  x: number;
  y: number;
}

export interface WindowSpec {
  id: string;
  width: number;
  height: number;
  gap?: number;
}
