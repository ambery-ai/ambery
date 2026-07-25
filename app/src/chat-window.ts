// Chat 窗口入口（docs/multi-window.md）：ChatPanel + 窗口定位
import { createBridge } from "./bridge";
import { ChatPanel } from "./chat";
import type { Edge } from "./view";

const VIEW_RADIUS_X = 36;
const VIEW_RADIUS_Y = 20;
const MARGIN = 12;

let petCenter = { x: 0, y: 0 };
let chatPanel: ChatPanel | null = null;
/** 面板实际尺寸（启动时量一次，docs/multi-window.md §窗口自适应） */
let panelW = 320;
let panelH = 380;

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
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().onCloseRequested(async () => {
      await getCurrentWindow().hide();
    });
  }

  const bridge = await createBridge();
  const mount = document.getElementById("app")!;
  chatPanel = new ChatPanel(mount, bridge, () => petCenter, true /* windowed */);

  // 量面板实际尺寸 → 窗口贴合（tauri.conf.json 的 chat width/height 为占位值）
  const el = document.getElementById("chat-panel");
  if (el) {
    const r = el.getBoundingClientRect();
    panelW = Math.ceil(r.width);
    panelH = Math.ceil(r.height);
    if ("__TAURI_INTERNALS__" in window) {
      const { getCurrentWindow, PhysicalSize } = await import("@tauri-apps/api/window");
      await getCurrentWindow().setSize(new PhysicalSize(panelW, panelH));
    }
  }
}

async function showChat(edge: Edge) {
  if (!chatPanel || chatPanel.isVisible()) return;
  // 面板显示（windowed 模式跳过 DOM 定位）
  chatPanel.show(edge);
  // 计算窗口位置（docs/multi-window.md §位置同步）
  let left: number;
  let top: number;
  if (edge === "top") {
    left = petCenter.x - panelW / 2;
    top = petCenter.y + VIEW_RADIUS_Y + MARGIN;
  } else if (edge === "bottom") {
    left = petCenter.x - panelW / 2;
    top = petCenter.y - VIEW_RADIUS_Y - MARGIN - panelH;
  } else if (edge === "left") {
    left = petCenter.x + VIEW_RADIUS_X + MARGIN;
    top = petCenter.y - panelH / 2;
  } else {
    left = petCenter.x - VIEW_RADIUS_X - MARGIN - panelW;
    top = petCenter.y - panelH / 2;
  }
  left = Math.max(8, Math.min(left, screen.availWidth - panelW - 8));
  top = Math.max(8, Math.min(top, screen.availHeight - panelH - 8));

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
