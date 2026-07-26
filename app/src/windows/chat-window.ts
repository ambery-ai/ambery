// Chat 窗口入口（docs/multi-window.md）：ChatPanel + 窗口定位
// Tauri B 方案：通过 requestPlace/requestRemove 调 pet 窗 engine
import { createBridge } from "../bridge";
import { ChatPanel } from "./chat";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import { requestPlace, requestRemove } from "../positioning/tauri-server";
import { Direction } from "../positioning/types";

let chatPanel: ChatPanel | null = null;
let adapter: WindowAdapter | null = null;
let panelW = 320;
let panelH = 380;

export async function main() {
  if ("__TAURI_INTERNALS__" in window) {
    adapter = await createTauriAdapter(document.body, 1);
    const { listen } = await import("@tauri-apps/api/event");
    await listen("pet:moved", () => {}); // 占位，确保事件系统初始化
    await listen("chat:toggle", () => {
      if (chatPanel?.isVisible()) hideChat();
      else showChat();
    });
    await listen("chat:hide", () => hideChat());
    await listen("chat:show", () => showChat());
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().onCloseRequested(async () => {
      await adapter?.hide();
    });
  }

  const bridge = await createBridge();
  const mount = document.getElementById("app")!;
  // ChatPanel 不需要 engine，toggle 直接用 DOM show/hide
  chatPanel = new ChatPanel(mount, bridge, null!, true);

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
  await adapter?.hide();
}

async function showChat() {
  if (!chatPanel) return;
  chatPanel.showPanel();
  const pos = await requestPlace("chat-panel", { id: "chat-panel", width: panelW, height: panelH }, Direction.sse);
  await adapter?.setPosition(Math.round(pos.x - panelW / 2), Math.round(pos.y - panelH / 2));
  await adapter?.show();
}

async function hideChat() {
  await adapter?.hide();
  chatPanel?.hidePanel();
  requestRemove("chat-panel");
}
