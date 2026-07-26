// Cards 窗口入口（docs/multi-window.md）：ComponentManager + 窗口定位
import { createBridge } from "../bridge";
import { ComponentManager } from "../components/component-manager";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import { requestPlace } from "../positioning/tauri-server";
import { Direction } from "../positioning/types";

let adapter: WindowAdapter | null = null;

export async function main() {
  if ("__TAURI_INTERNALS__" in window) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    adapter = await createTauriAdapter(document.body, 1);
    const win = getCurrentWindow();
    win.onCloseRequested(async () => { await adapter?.hide(); });
    await adapter.hide();
    // 拖拽 hide/restore 事件
    const { listen } = await import("@tauri-apps/api/event");
    await listen("cards:hide", () => adapter?.hide());
    await listen("cards:show", () => adapter?.show());
  }

  const bridge = await createBridge();
  const mount = document.getElementById("app")!;
  new ComponentManager(mount, bridge, () => ({ x: 0, y: 0 }), true);

  if ("__TAURI_INTERNALS__" in window) {
    let posTimer: ReturnType<typeof setTimeout> | null = null;
    const observer = new MutationObserver(() => {
      if (posTimer) clearTimeout(posTimer);
      posTimer = setTimeout(() => positionWindow(), 50);
    });
    observer.observe(mount, { childList: true, subtree: true });
  }
}

function cardDirection(el: HTMLElement): Direction {
  const d = el.dataset.direction;
  if (d && d !== "auto") {
    const dir = Direction[d as keyof typeof Direction];
    if (dir !== undefined) return dir;
  }
  return Direction.sse;
}

async function positionWindow() {
  if (!adapter) return;
  const card = document.querySelector(".component") as HTMLElement | null;
  if (!card) { await adapter.hide(); return; }
  const dir = cardDirection(card);
  const cw = card.offsetWidth || 260;
  const ch = card.offsetHeight || 140;
  const id = `card-${card.dataset.id || Date.now()}`;
  const pos = await requestPlace(id, { id, width: cw, height: ch }, dir);
  await adapter.setPosition(Math.round(pos.x - cw / 2), Math.round(pos.y - ch / 2));
  await adapter.setSize(cw + 4, ch + 4);
  await adapter.show();
}
