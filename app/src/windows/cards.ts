// Cards 窗口入口（docs/multi-window.md）：ComponentManager + 窗口定位
import type { Direction } from "../bridge";
import { createBridge } from "../bridge";
import { ComponentManager } from "../components/component-manager";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";

const GAP = 12;
const VIEW_RADIUS_X = 36;
const VIEW_RADIUS_Y = 20;
const EDGE_MARGIN = 8;

let petCenter = { x: 0, y: 0 };

let adapter: WindowAdapter | null = null;

export async function main() {
  // Tauri 模式：订阅 pet 位置
  if ("__TAURI_INTERNALS__" in window) {
    const { listen } = await import("@tauri-apps/api/event");
    await listen<{ x: number; y: number }>("pet:moved", (ev) => {
      petCenter = ev.payload;
    });
    // 隐藏窗口 → 隐藏（不退出）
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    adapter = await createTauriAdapter(document.body, 1);
    const win = getCurrentWindow();
    win.onCloseRequested(async () => {
      await adapter?.hide();
    });
  }

  const bridge = await createBridge();
  const mount = document.getElementById("app")!;
  // 卡片在窗口内自然流式布局（非 fixed），窗口位置由外部计算
  new ComponentManager(mount, bridge, () => petCenter, true /* windowed */);

  // Tauri 模式：卡片渲染时移动窗口
  if ("__TAURI_INTERNALS__" in window) {
    const observer = new MutationObserver(() => {
      positionWindow();
    });
    observer.observe(mount, { childList: true, subtree: true });
  }
}

/** 根据卡片内容 + pet 位置 + 方向计算并设置窗口位置 */
async function positionWindow() {
  if (!adapter) return;
  const card = document.querySelector(".component") as HTMLElement | null;
  if (!card) {
    await adapter.hide();
    return;
  }
  const dir = (card.dataset.direction ?? "auto") as Direction;
  const cw = card.offsetWidth || 260;
  const ch = card.offsetHeight || 140;
  const pos = calcWindowPos(petCenter, dir, cw, ch);
  await adapter.setSize(cw + 4, ch + 4);
  await adapter.setPosition(pos.x, pos.y);
  await adapter.show();
}

function calcWindowPos(
  anchor: { x: number; y: number },
  dir: string,
  cw: number,
  ch: number,
): { x: number; y: number } {
  let left = anchor.x - cw / 2;
  let top = anchor.y - ch / 2;
  if (dir.includes("left")) left = anchor.x - VIEW_RADIUS_X - GAP - cw;
  if (dir.includes("right")) left = anchor.x + VIEW_RADIUS_X + GAP;
  if (dir.includes("top")) top = anchor.y - VIEW_RADIUS_Y - GAP - ch;
  if (dir.includes("bottom")) top = anchor.y + VIEW_RADIUS_Y + GAP;
  if (dir === "left" || dir === "right") top = anchor.y - ch / 2;
  if (dir === "top" || dir === "bottom") left = anchor.x - cw / 2;
  if (dir === "auto") {
    // 自动选空间最大的方向
    const spaces: [string, number][] = [
      ["left", anchor.x],
      ["right", screen.availWidth - anchor.x],
      ["top", anchor.y],
      ["bottom", screen.availHeight - anchor.y],
    ];
    spaces.sort((a, b) => b[1] - a[1]);
    return calcWindowPos(anchor, spaces[0][0], cw, ch);
  }
  return {
    x: Math.max(EDGE_MARGIN, Math.min(left, screen.availWidth - cw - EDGE_MARGIN)),
    y: Math.max(EDGE_MARGIN, Math.min(top, screen.availHeight - ch - EDGE_MARGIN)),
  };
}
