// positioning/tauri-server — pet 窗持有 engine，chat/cards 通过 Tauri 事件请求 place
// pet.ts 只需 import { engine } 并调 setupServer()。chat/cards 调 requestPlace()。

import { PositioningEngine } from "./engine";
import type { Point, WindowSpec } from "./types";
import { Direction } from "./types";
import { emitEvent } from "../tauri_runtime_actions";

export const engine = new PositioningEngine();

/** pet 窗调用一次，注册处理 chat/cards 的 place 请求 */
export async function setupServer() {
  const { listen } = await import("@tauri-apps/api/event");

  await listen<{ id: string; spec: WindowSpec; preferred: string }>("engine:place", (ev) => {
    const dir = Direction[ev.payload.preferred as keyof typeof Direction] ?? Direction.sse;
    const pos = engine.place(ev.payload.spec, dir);
    void emitEvent("engine:placed", { id: ev.payload.id, x: pos.x, y: pos.y });
  });

  await listen<{ id: string }>("engine:remove", (ev) => {
    engine.remove(ev.payload.id);
  });

  // #12/#15/#8①：chat/cards 拖拽结束回写真实位置 → 新跟随基准
  await listen<{ id: string; x: number; y: number }>("engine:moved", (ev) => {
    engine.updateCenter(ev.payload.id, { x: ev.payload.x, y: ev.payload.y });
  });
}

/** chat/cards 窗调用：拖拽结束回写中心点（OS 真实位置换算） */
export async function reportMoved(id: string, center: Point) {
  void emitEvent("engine:moved", { id, x: center.x, y: center.y });
}

/** chat/cards 窗调用：请求 engine 计算位置，返回 Promise<Point> */
export function requestPlace(
  id: string,
  spec: WindowSpec,
  preferred: Direction,
): Promise<Point> {
  return new Promise(async (resolve) => {
    const { listen } = await import("@tauri-apps/api/event");
    let settled = false;
    // B8: 2s 超时——pet 窗崩溃/reload 时防止 Promise 永不 resolve
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      unlisten();
      console.warn("[requestPlace] timeout for", id);
      resolve({ x: 0, y: 0 }); // 回退到原点，至少窗口可见
    }, 2000);
    const unlisten = await listen<{ id: string; x: number; y: number }>("engine:placed", (ev) => {
      if (ev.payload.id === id) {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        unlisten();
        resolve({ x: ev.payload.x, y: ev.payload.y });
      }
    });
    void emitEvent("engine:place", { id, spec, preferred: Direction[preferred] });
  });
}

/** chat/cards 窗调用：请求移除占区 */
export async function requestRemove(id: string) {
  void emitEvent("engine:remove", { id });
}
