// Chat 窗口入口（docs/multi-window.md）：ChatPanel + 窗口定位
// Tauri B 方案：通过 requestPlace/requestRemove 调 pet 窗 engine
import { createBridge } from "../bridge";
import { ChatPanel } from "./chat";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import { requestPlace, reportMoved } from "../positioning/tauri-server";
import { Direction } from "../positioning/types";

let chatPanel: ChatPanel | null = null;
let adapter: WindowAdapter | null = null;
let panelW = 320;
let panelH = 380;

export async function main() {
  if ("__TAURI_INTERNALS__" in window) {
    adapter = await createTauriAdapter(document.body, 1);
    const { listen } = await import("@tauri-apps/api/event");
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();
    await listen("pet:moved", () => {}); // 占位，确保事件系统初始化
    await listen("chat:toggle", () => {
      if (chatPanel?.isVisible()) {
        chatPanel.intentClose(); // 用户意图关（统一 API）
        void adapter?.hide();
      } else {
        chatPanel?.intentOpen();
        void showChat();
      }
    });
    // 系统藏（pet 拖动/托盘）：统一 API，只藏不动 userClosed
    await listen("chat:hide", () => {
      chatPanel?.systemHide();
      void adapter?.hide();
    });
    // 系统恢复：统一 API 判定（A 语义）
    await listen("chat:show", () => {
      if (chatPanel?.systemRestore()) void showChat();
    });
    win.onCloseRequested(async () => {
      chatPanel?.intentClose(); // × = 用户意图关
      await adapter?.hide();
    });

    // #8②：chat 头部可拖拽（排除 × 按钮）
    document.addEventListener("mousedown", (e) => {
      const t = e.target as HTMLElement;
      if (t.closest(".chat-header") && !t.closest(".chat-close")) {
        void import("../effects").then((m) => m.reportEffect("window_drag", { window: win.label }));
        void win.startDragging();
      }
    });
    // #12/#8①②：拖拽结束（onMoved 防抖）→ 回写真实位置为跟随基准
    let moveTimer: number | undefined;
    await win.onMoved(() => {
      clearTimeout(moveTimer);
      moveTimer = window.setTimeout(async () => {
        const pos = await win.outerPosition();
        await reportMoved("chat-panel", { x: pos.x + panelW / 2, y: pos.y + panelH / 2 });
      }, 250);
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
