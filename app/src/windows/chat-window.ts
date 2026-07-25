// Chat 窗口入口（docs/multi-window.md）：ChatPanel + 窗口定位
import { createBridge } from "../bridge";
import { ChatPanel } from "./chat";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import type { PositioningEngine } from "../positioning/engine";
import { Direction } from "../positioning/types";

let petCenter = { x: 0, y: 0 };
let chatPanel: ChatPanel | null = null;
let adapter: WindowAdapter | null = null;
let engine: PositioningEngine | null = null;
let panelW = 320;
let panelH = 380;

export async function main(eng: PositioningEngine) {
  engine = eng;
  // WindowAdapter（feat）
  if ("__TAURI_INTERNALS__" in window) {
    adapter = await createTauriAdapter(document.body, 1);
  }

  // Tauri 模式：订阅 pet 位置 + chat 显隐事件
  if ("__TAURI_INTERNALS__" in window) {
    const { listen } = await import("@tauri-apps/api/event");
    await listen<{ x: number; y: number }>("pet:moved", (ev) => {
      petCenter = ev.payload;
    });
    await listen("chat:toggle", () => {
      if (chatPanel?.isVisible()) hideChat();
      else showChat();
    });
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().onCloseRequested(async () => {
      await adapter?.hide();
    });
  }

  const bridge = await createBridge();
  const mount = document.getElementById("app")!;
  chatPanel = new ChatPanel(mount, bridge, () => petCenter, true /* windowed */);

  // 量面板实际尺寸 → 窗口贴合（hidden 时 layout 未算，先显示等一帧再量；DPI 修正）
  const el = document.getElementById("chat-panel");
  if (el) {
    el.hidden = false;
    el.style.visibility = "hidden";
    await new Promise((r) => requestAnimationFrame(r));
    const r = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    panelW = Math.ceil(r.width * dpr) || 320;
    panelH = Math.ceil(r.height * dpr) || 380;
    el.hidden = true;
    el.style.visibility = "";
    await adapter?.setSize(panelW, panelH);
  }
}

async function showChat() {
  if (!chatPanel || chatPanel.isVisible()) return;
  chatPanel.show("bottom" as any);
  if (engine) {
    const pos = engine.place(
      { id: "chat-panel", width: panelW, height: panelH },
      Direction.sse, petCenter, { w: 72, h: 40 },
    );
    await adapter?.setPosition(Math.round(pos.x - panelW/2), Math.round(pos.y - panelH/2));
  }
  await adapter?.show();
}

async function hideChat() {
  await adapter?.hide();
  chatPanel?.hide();
}
