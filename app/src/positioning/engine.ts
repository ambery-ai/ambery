// positioning/engine — stateful layout engine
// 坐标系：pet 相对（pet 固定 0,0）。occupied 全存「相对 pet 中心的偏移」，
// pet 移动只改 petCenter，无快照、无平移、无 stale（#12 设计定案）。

import { computeCDSegments } from "./geometry";
import { ternarySearch } from "./math";
import { monitorOf } from "./monitors";
import { directionAngle, type Direction, type Point, type WindowSpec } from "./types";

const DEFAULT_ALPHA = 20;
const DEFAULT_BETA = 0.1;
const DEFAULT_GAP = 12;
const TOL = 1e-4; // 三元搜索 t 空间收敛精度

interface Occupied {
  id: string;
  /** 相对 pet 中心的偏移（pet 移动的不变量） */
  offset: Point;
  w: number;
  h: number;
  /** 用户亲手拖过（#12/#15/#8①）：place 时保持偏移不重算 */
  manual?: boolean;
}

export class PositioningEngine {
  private occupied: Occupied[] = [];
  /** 布局记忆（docs/window-follow.md §一致性剖析）：用户隐藏释放占区但保留布局；
   *  与系统临时隐藏（占区原地保留）和 dismiss（remove：结束 Surface 并忘记布局）区分 */
  private layoutMemory = new Map<string, { offset: Point; w: number; h: number; manual?: boolean }>();
  private petCenter: Point = { x: 0, y: 0 };
  private petSize = { w: 0, h: 0 };

  constructor(
    public alpha: number = DEFAULT_ALPHA,
    public beta: number = DEFAULT_BETA,
  ) {}

  /** pet 当前屏幕中心（pet.ts onMoved 持续回写，保证换算基准新鲜） */
  registerPet(center: Point, size: { w: number; h: number }): void {
    this.petCenter = center;
    this.petSize = size;
  }

  /** 放置新窗口：自动布局（或 manual 保持偏移）。返回屏幕绝对坐标（左上角换算由调用方做）。
   *  出屏兜底（docs/window-follow.md §出屏与重叠，issues #21）：完全出屏 → 全 16 方位环
   *  重试（±1~±8，首选命中即止）；全失败取最初结果（不压人，不做位置修正）。
   *  算法层零改动，重试是薄包装。 */
  place(newWindow: WindowSpec, preferred: Direction): Point {
    const dirs: Direction[] = [preferred];
    for (let i = 1; i <= 7; i++) {
      dirs.push((((preferred + i) % 16) + 16) % 16, (((preferred - i) % 16) + 16) % 16);
    }
    dirs.push((((preferred + 8) % 16) + 16) % 16); // 正对面
    // 首个「非完全出屏」即止（部分可见即可，用户不要求完全可见，#21）
    let first: Point | null = null;
    for (const dir of dirs) {
      const p = this.placeOnce(newWindow, dir);
      first ??= p;
      if (!this._fullyOffscreen(p, newWindow)) return p;
    }
    return first!;
  }

  /** 完全出屏判定（零相交才否决；部分出屏接受；视口 = 缓存 monitor 表，docs/window-follow.md） */
  private _fullyOffscreen(center: Point, spec: WindowSpec): boolean {
    const m = monitorOf(this.petCenter);
    return (
      center.x + spec.width / 2 < m.x ||
      center.x - spec.width / 2 > m.x + m.width ||
      center.y + spec.height / 2 < m.y ||
      center.y - spec.height / 2 > m.y + m.height
    );
  }

  private placeOnce(newWindow: WindowSpec, preferred: Direction): Point {
    const existing = this.occupied.find((o) => o.id === newWindow.id && o.id !== "_pet_");
    if (existing?.manual) {
      // 手动位优先（#12）：保持偏移，仅刷新尺寸
      existing.w = newWindow.width;
      existing.h = newWindow.height;
      return {
        x: this.petCenter.x + existing.offset.x,
        y: this.petCenter.y + existing.offset.y,
      };
    }

    // 布局记忆命中（用户隐藏后重开）：恢复记忆偏移并重占区，不参与自动重排
    const remembered = this.layoutMemory.get(newWindow.id);
    if (!existing && remembered) {
      this.layoutMemory.delete(newWindow.id);
      this.occupied.push({
        id: newWindow.id,
        offset: remembered.offset,
        w: newWindow.width,
        h: newWindow.height,
        manual: remembered.manual,
      });
      const result = {
        x: this.petCenter.x + remembered.offset.x,
        y: this.petCenter.y + remembered.offset.y,
      };
      console.info("[engine] place（布局记忆恢复）", newWindow.id, "→", Math.round(result.x), Math.round(result.y));
      return result;
    }

    const petCenter = this.petCenter;
    const mAngle = directionAngle(preferred);
    const others = this.occupied.filter((o) => o.id !== "_pet_");
    const segs = computeCDSegments(
      petCenter, this.petSize, newWindow,
      others.map((o) => ({
        center: { x: petCenter.x + o.offset.x, y: petCenter.y + o.offset.y },
        w: o.w, h: o.h,
      })),
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

    const result = best ?? {
      x: petCenter.x,
      y: petCenter.y - this.petSize.h / 2 - DEFAULT_GAP - newWindow.height / 2,
    };
    const offset = { x: result.x - petCenter.x, y: result.y - petCenter.y };
    if (existing) {
      existing.offset = offset;
      existing.w = newWindow.width;
      existing.h = newWindow.height;
    } else {
      this.occupied.push({ id: newWindow.id, offset, w: newWindow.width, h: newWindow.height });
    }
    console.info("[engine] place", newWindow.id, "→", Math.round(result.x), Math.round(result.y));
    return result;
  }

  /** 拖拽结束回写（#12/#15/#8①）：OS 真实屏幕中心 → 换算偏移 → manual 标记 */
  updateCenter(id: string, center: Point): void {
    const o = this.occupied.find((o) => o.id === id && o.id !== "_pet_");
    if (!o) return;
    o.offset = { x: center.x - this.petCenter.x, y: center.y - this.petCenter.y };
    o.manual = true;
    console.info("[engine] updateCenter (manual)", id, "→ offset", Math.round(o.offset.x), Math.round(o.offset.y));
  }

  /** 恢复坐标（pet 移动/托盘回来）：现算 petCenter + offset，无快照（设计定案） */
  restorePositions(petCenter: Point): { id: string; center: Point }[] {
    return this.occupied
      .filter((o) => o.id !== "_pet_")
      .map((o) => ({
        id: o.id,
        center: { x: petCenter.x + o.offset.x, y: petCenter.y + o.offset.y },
      }));
  }

  /** 用户隐藏：释放占区但保留布局记忆（重开时 place 原位恢复）。
   *  系统临时隐藏不调它（占区原地保留）；dismiss 用 remove（连布局一起忘记）。 */
  release(id: string): void {
    const o = this.occupied.find((o) => o.id === id && o.id !== "_pet_");
    if (!o) return;
    this.layoutMemory.set(id, { offset: o.offset, w: o.w, h: o.h, manual: o.manual });
    this.occupied = this.occupied.filter((x) => x.id !== id);
    console.info("[engine] release（用户隐藏，布局入记忆）", id);
  }

  /** dismiss：结束 Surface——占区与布局记忆一并忘记 */
  remove(id: string): void {
    console.info("[engine] remove（dismiss，忘记布局）", id);
    this.occupied = this.occupied.filter((o) => o.id !== id);
    this.layoutMemory.delete(id);
  }

  /** 清空所有占区与布局记忆 */
  clear(): void {
    this.occupied = [];
    this.layoutMemory.clear();
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
