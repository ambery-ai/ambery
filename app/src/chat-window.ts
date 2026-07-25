// Chat 窗口入口（docs/multi-window.md）：ChatPanel + 窗口定位
import { createBridge } from "./bridge";
import { ChatPanel } from "./chat";
import type { Edge } from "./view";

const PANEL_W = 320;
const PANEL_H = 380;
const VIEW_RADIUS_X = 36;
const VIEW_RADIUS_Y = 20;
const MARGIN = 12;

let petCenter = { x: 0, y: 0 };
let chatPanel: ChatPanel | null = null;

export async function main() {
  // Tauri 模式：订阅 pet 位置 + chat 显隐事件
  if ("__TAURI_INTERNALS__" in window) {
    const { listen } = await import("@tauri-apps/api/event");
    await listen<{ x: number; y: number }>("pet:moved", (ev) => {
      petCenter = ev.payload;
    });
    await listen<{ edge: Edge }>("chat:show", (ev) => {
      showChat(ev.payload.edge);
    });
    await listen("chat:hide", () => {
      hideChat();
    });
    // 隐藏窗口 → 隐藏（不退出）
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().onCloseRequested(async () => {
      await getCurrentWindow().hide();
    });
  }

  const bridge = await createBridge();
  const mount = document.getElementById("app")!;
  chatPanel = new ChatPanel(mount, bridge, () => petCenter, true /* windowed */);
}

async function showChat(edge: Edge) {
  if (!chatPanel || chatPanel.isVisible()) return;
  // 面板显示（windowed 模式跳过 DOM 定位）
  chatPanel.show(edge);
  // 计算窗口位置（docs/multi-window.md §位置同步）
  let left: number;
  let top: number;
  if (edge === "top") {
    left = petCenter.x - PANEL_W / 2;
    top = petCenter.y + VIEW_RADIUS_Y + MARGIN;
  } else if (edge === "bottom") {
    left = petCenter.x - PANEL_W / 2;
    top = petCenter.y - VIEW_RADIUS_Y - MARGIN - PANEL_H;
  } else if (edge === "left") {
    left = petCenter.x + VIEW_RADIUS_X + MARGIN;
    top = petCenter.y - PANEL_H / 2;
  } else {
    left = petCenter.x - VIEW_RADIUS_X - MARGIN - PANEL_W;
    top = petCenter.y - PANEL_H / 2;
  }
  left = Math.max(8, Math.min(left, screen.availWidth - PANEL_W - 8));
  top = Math.max(8, Math.min(top, screen.availHeight - PANEL_H - 8));

  if ("__TAURI_INTERNALS__" in window) {
    const { getCurrentWindow, PhysicalPosition } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();
    await win.setPosition(new PhysicalPosition(left, top));
    await win.show();
    await win.setFocus();
  }
}

async function hideChat() {
  if ("__TAURI_INTERNALS__" in window) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().hide();
  }
  chatPanel?.hide();
}
