// Cards 窗口入口（docs/multi-window.md）：ComponentManager + 窗口定位
import { createBridge } from "../bridge";
import { ComponentManager } from "../components/component-manager";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import type { PositioningEngine } from "../positioning/engine";
import { Direction } from "../positioning/types";

let adapter: WindowAdapter | null = null;
let engine: PositioningEngine | null = null;

export async function main(eng: PositioningEngine) {
  engine = eng;

  // Tauri 模式：订阅 pet 位置
  if ("__TAURI_INTERNALS__" in window) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    adapter = await createTauriAdapter(document.body, 1);
    const win = getCurrentWindow();
    win.onCloseRequested(async () => {
      await adapter?.hide();
    });
  }

  const bridge = await createBridge();
  const mount = document.getElementById("app")!;
  new ComponentManager(mount, bridge, () => ({ x: 0, y: 0 }), true);

  // Tauri 模式：卡片渲染时移动窗口
  if ("__TAURI_INTERNALS__" in window) {
    const observer = new MutationObserver(() => { positionWindow(); });
    observer.observe(mount, { childList: true, subtree: true });
  }
}

function cardDirection(el: HTMLElement): Direction {
  const d = el.dataset.direction;
  if (d && d !== "auto") {
    const dir = Direction[d as keyof typeof Direction];
    if (dir !== undefined) return dir;
  }
  // 自动选空闲方向
  for (const d of [Direction.sse, Direction.e, Direction.w, Direction.s, Direction.n]) {
    return d;
  }
  return Direction.sse;
}

async function positionWindow() {
  if (!adapter || !engine) return;
  const card = document.querySelector(".component") as HTMLElement | null;
  if (!card) { await adapter.hide(); return; }
  const dir = cardDirection(card);
  const cw = card.offsetWidth || 260;
  const ch = card.offsetHeight || 140;
  const pos = engine.place(
    { id: `card-${card.dataset.id || Date.now()}`, width: cw + 4, height: ch + 4 },
    dir,
  );
  await adapter.setPosition(Math.round(pos.x - cw/2), Math.round(pos.y - ch/2));
  await adapter.setSize(cw + 4, ch + 4);
  await adapter.show();
}
