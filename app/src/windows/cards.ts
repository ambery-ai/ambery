// Cards 窗口入口（docs/multi-window.md）：ComponentManager + 窗口定位
import { createBridge } from "../bridge";
import { ComponentManager } from "../components/component-manager";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import { requestPlace, requestRemove } from "../positioning/tauri-server";
import { Direction } from "../positioning/types";

let adapter: WindowAdapter | null = null;
let lastCardId: string | null = null;
/** 缓存在 positionWindow 中测量的物理像素尺寸，供 cards:show 隐藏时复用（B1） */
let lastPw = 260, lastPh = 140;
/** Tauri 物理像素 dpr（B3） */
let dpr = 1;

export async function main() {
  if ("__TAURI_INTERNALS__" in window) {
    dpr = window.devicePixelRatio || 1;
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    adapter = await createTauriAdapter(document.body, dpr);
    const win = getCurrentWindow();
    win.onCloseRequested(async () => { await adapter?.hide(); });
    await adapter.hide();
    // 拖拽 hide/restore 事件
    const { listen } = await import("@tauri-apps/api/event");
    await listen("cards:hide", () => adapter?.hide());
    await listen<{ x: number; y: number }>("cards:show", async (ev) => {
      const pos = ev.payload;
      // B5: 无 payload 或无 card → 重新 place，不盲 show
      if (!pos || !Number.isFinite(pos.x) || !Number.isFinite(pos.y)) {
        void positionWindow();
        return;
      }
      const card = document.querySelector(".component") as HTMLElement | null;
      if (!card) { await adapter?.hide(); return; }
      try {
        // B2: await setPosition，避免 show 时位置未更新
        // B1: 用缓存物理尺寸（positionWindow 测量时窗口刚 show 完、尺寸准确）
        await adapter?.setPosition(
          Math.round(pos.x - lastPw / 2),
          Math.round(pos.y - lastPh / 2),
        );
      } catch (e) { console.warn("[cards] restore setPosition failed", e); }
      await adapter?.show();
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
  // B3: offsetWidth/Height 是 CSS 像素，×dpr 转物理像素（engine 坐标系）
  const pw = Math.ceil((card.offsetWidth || 260) * dpr);
  const ph = Math.ceil((card.offsetHeight || 140) * dpr);
  // B1: 缓存物理尺寸，供 cards:show 隐藏状态复用
  lastPw = pw;
  lastPh = ph;
  lastCardId = `card-${card.dataset.id || Date.now()}`;
  const pos = await requestPlace(lastCardId, { id: lastCardId, width: pw, height: ph }, dir);
  // pos 是 engine 返回的物理坐标中心点
  await adapter.setPosition(Math.round(pos.x - pw / 2), Math.round(pos.y - ph / 2));
  await adapter.setSize(pw + 8, ph + 8);
  await adapter.show();
}
