// positioning/engine — stateful layout engine

import { computeCDSegments } from "./geometry";
import { ternarySearch } from "./math";
import { directionAngle, type Direction, type Point, type WindowSpec } from "./types";

const DEFAULT_ALPHA = 20;
const DEFAULT_BETA = 0.1;
const DEFAULT_GAP = 12;
const TOL = 1e-4; // 三元搜索 t 空间收敛精度

interface Occupied {
  id: string;
  center: Point;
  w: number;
  h: number;
}

export class PositioningEngine {
  private occupied: Occupied[] = [];
  private hidden: { id: string; offset: Point }[] | null = null;

  constructor(
    public alpha: number = DEFAULT_ALPHA,
    public beta: number = DEFAULT_BETA,
  ) {}

  place(newWindow: WindowSpec, preferred: Direction, petCenter: Point, petSize: { w: number; h: number }): Point {
    const mAngle = directionAngle(preferred);
    const others = this.occupied.filter((o) => o.id !== "_pet_");
    const segs = computeCDSegments(
      petCenter, petSize, newWindow,
      others.map((o) => ({ center: o.center, w: o.w, h: o.h })),
      DEFAULT_GAP,
    );

    let best: Point | null = null;
    let bestVal = Infinity;

    for (const [C, D] of segs) {
      const t = ternarySearch(C, D, petCenter, mAngle, this.alpha, this.beta, TOL);
      const B = { x: C.x + (D.x - C.x) * t, y: C.y + (D.y - C.y) * t };
      const v = this._valueAt(B, petCenter, mAngle);
      if (v < bestVal) {
        bestVal = v;
        best = B;
      }
    }

    const result = best ?? { x: petCenter.x, y: petCenter.y - petSize.h / 2 - DEFAULT_GAP - newWindow.height / 2 };
    this.occupied.push({ id: newWindow.id, center: result, w: newWindow.width, h: newWindow.height });
    return result;
  }

  remove(id: string): void {
    this.occupied = this.occupied.filter((o) => o.id !== id);
  }

  /** 清空所有占区（保留 pet） */
  clear(): void {
    this.occupied = this.occupied.filter((o) => o.id === "_pet_");
  }

  hideAll(): void {
    if (this.occupied.length === 0) return;
    const pet = this.occupied.find((o) => o.id === "_pet_");
    const petCtr = pet?.center ?? { x: 0, y: 0 };
    this.hidden = this.occupied
      .filter((o) => o.id !== "_pet_")
      .map((o) => ({ id: o.id, offset: { x: o.center.x - petCtr.x, y: o.center.y - petCtr.y } }));
  }

  restoreAll(petCenter: Point): { id: string; center: Point }[] {
    if (!this.hidden) return [];
    const result = this.hidden.map((h) => ({
      id: h.id,
      center: { x: petCenter.x + h.offset.x, y: petCenter.y + h.offset.y },
    }));
    for (const r of result) {
      const oc = this.occupied.find((o) => o.id === r.id);
      if (oc) oc.center = r.center;
    }
    this.hidden = null;
    return result;
  }

  registerPet(center: Point, size: { w: number; h: number }): void {
    const pet = this.occupied.find((o) => o.id === "_pet_");
    if (pet) {
      pet.center = center;
      pet.w = size.w;
      pet.h = size.h;
    } else {
      this.occupied.push({ id: "_pet_", center, w: size.w, h: size.h });
    }
  }

  private _valueAt(B: Point, A: Point, mAngle: number): number {
    const dx = B.x - A.x;
    const dy = B.y - A.y;
    const d = Math.sqrt(dx * dx + dy * dy);
    const a = Math.atan2(dy, dx);
    const theta = Math.min(Math.abs(a - mAngle), 2 * Math.PI - Math.abs(a - mAngle));
    return this.alpha * theta * theta + this.beta * d;
  }
}
