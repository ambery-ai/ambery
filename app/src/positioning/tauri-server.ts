// positioning/tauri-server — pet 窗持有 engine，chat/cards 通过 Tauri 事件请求 place
// pet.ts 只需 import { engine } 并调 setupServer()。chat/cards 调 requestPlace()。

import { PositioningEngine } from "./engine";
import type { Point, WindowSpec } from "./types";
import { Direction } from "./types";

export const engine = new PositioningEngine();

/** pet 窗调用一次，注册处理 chat/cards 的 place 请求 */
export async function setupServer() {
  const { listen } = await import("@tauri-apps/api/event");
  const { emit } = await import("@tauri-apps/api/event");

  await listen<{ id: string; spec: WindowSpec; preferred: string }>("engine:place", (ev) => {
    const dir = Direction[ev.payload.preferred as keyof typeof Direction] ?? Direction.sse;
    const pos = engine.place(ev.payload.spec, dir);
    emit("engine:placed", { id: ev.payload.id, x: pos.x, y: pos.y });
  });

  await listen<{ id: string }>("engine:remove", (ev) => {
    engine.remove(ev.payload.id);
  });
}

/** chat/cards 窗调用：请求 engine 计算位置，返回 Promise<Point> */
export function requestPlace(
  id: string,
  spec: WindowSpec,
  preferred: Direction,
): Promise<Point> {
  return new Promise(async (resolve) => {
    const { emit } = await import("@tauri-apps/api/event");
    const { listen } = await import("@tauri-apps/api/event");
    const unlisten = await listen<{ id: string; x: number; y: number }>("engine:placed", (ev) => {
      if (ev.payload.id === id) {
        unlisten();
        resolve({ x: ev.payload.x, y: ev.payload.y });
      }
    });
    emit("engine:place", { id, spec, preferred: Direction[preferred] });
  });
}

/** chat/cards 窗调用：请求移除占区 */
export async function requestRemove(id: string) {
  const { emit } = await import("@tauri-apps/api/event");
  emit("engine:remove", { id });
}
