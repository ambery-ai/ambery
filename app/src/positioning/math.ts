// positioning/math — value function + ternary search (stateless)

import type { Point } from "./types";

/** 向量 AB 的长度 */
function dist(A: Point, B: Point): number {
  const dx = B.x - A.x;
  const dy = B.y - A.y;
  return Math.sqrt(dx * dx + dy * dy);
}

/** 自由向量 (dx, dy) 与 preferred 方向 mAngle 的最小夹角（弧度，[0, π]） */
function angleDiff(dx: number, dy: number, mAngle: number): number {
  const a = Math.atan2(dy, dx);
  const d = Math.abs(a - mAngle);
  return Math.min(d, 2 * Math.PI - d);
}

/** V(B) = α·θ² + β·|AB| */
export function value(
  A: Point,
  B: Point,
  mAngle: number,
  alpha: number,
  beta: number,
): number {
  const d = dist(A, B);
  const dx = B.x - A.x;
  const dy = B.y - A.y;
  const theta = angleDiff(dx, dy, mAngle);
  return alpha * theta * theta + beta * d;
}

/** CD 段上三分搜索 V(B) 的极小值对应的 t ∈ [0, 1]，tol 是搜索精度 */
export function ternarySearch(
  C: Point,
  D: Point,
  A: Point,
  mAngle: number,
  alpha: number,
  beta: number,
  tol: number,
): number {
  let l = 0;
  let r = 1;
  while (r - l > tol) {
    const m1 = l + (r - l) / 3;
    const m2 = r - (r - l) / 3;
    const b1 = lerp(C, D, m1);
    const b2 = lerp(C, D, m2);
    if (value(A, b1, mAngle, alpha, beta) < value(A, b2, mAngle, alpha, beta)) {
      r = m2;
    } else {
      l = m1;
    }
  }
  return (l + r) / 2;
}

function lerp(C: Point, D: Point, t: number): Point {
  return { x: C.x + (D.x - C.x) * t, y: C.y + (D.y - C.y) * t };
}
