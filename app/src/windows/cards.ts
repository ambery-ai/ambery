// Cards 窗口入口（docs/multi-window.md）：ComponentManager + 窗口定位
import { createBridge } from "../bridge";
import { ComponentManager } from "../components/component-manager";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import { requestPlace, requestRemove } from "../positioning/tauri-server";
import { Direction } from "../positioning/types";

let adapter: WindowAdapter | null = null;
let lastCardId: string | null = null;

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
    await listen<{ x: number; y: number }>("cards:show", (ev) => {
      const pos = ev.payload;
      if (pos) {
        const card = document.querySelector(".component") as HTMLElement | null;
        if (card) {
          const cw = card.offsetWidth || 260;
          const ch = card.offsetHeight || 140;
          adapter?.setPosition(Math.round(pos.x - cw / 2), Math.round(pos.y - ch / 2));
        }
      }
      adapter?.show();
    });
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
    observer.observe(mount, { childList: true, subtree: true, characterData: true });
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
  if (!card) {
    // 卡片被关闭：清除引擎占区 + 隐藏窗口
    if (lastCardId) { requestRemove(lastCardId); lastCardId = null; }
    await adapter.hide();
    return;
  }
  const dir = cardDirection(card);
  // offsetWidth/Height = 当前渲染尺寸，搭配 CSS 无固定宽高 + min/max 约束
  const cw = card.offsetWidth || 260;
  const ch = card.offsetHeight || 140;
  lastCardId = `card-${card.dataset.id || Date.now()}`;
  const pos = await requestPlace(lastCardId, { id: lastCardId, width: cw, height: ch }, dir);
  await adapter.setPosition(Math.round(pos.x - cw / 2), Math.round(pos.y - ch / 2));
  await adapter.setSize(cw + 8, ch + 8);
  await adapter.show();
}
