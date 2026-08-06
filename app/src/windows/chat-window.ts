// Chat 窗口入口（docs/multi-window.md）：ChatPanel + 窗口定位
// Tauri B 方案：通过 requestPlace/requestRemove 调 pet 窗 engine
import { createBridge } from "../bridge";
import { ChatPanel } from "./chat";
import { Store } from "../store";
import { wireTheme } from "../theme";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import { requestPlace, requestRelease, reportMoved } from "../positioning/tauri-server";

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
    // 唤出/关闭由吸附驱动（docs/chat-panel.md §唤出与关闭 + docs/view.md §状态机）：
    // view:docked → chat:open（含吸附边，按边向屏幕内侧展开）；view:undocked → chat:close；
    // × 关闭后左键单击 View → chat:open（重新唤出）
    await listen<{ edge?: string }>("chat:open", (ev) => {
      if (chatPanel?.isVisible()) return;
      chatPanel?.intentOpen();
      void showChat(ev.payload.edge ?? null);
    });
    await listen("chat:close", () => {
      if (chatPanel?.isVisible()) chatPanel.intentClose(); // 统一 API（onIntentClose 钩子做 release+hide）
    });
    // 系统藏（pet 拖动/托盘）：统一 API，只藏不动 userClosed——占区原地保留，不调 release
    await listen("chat:hide", () => {
      chatPanel?.systemHide();
      void adapter?.hide();
    });
    // 系统恢复：统一 API 判定（A 语义）
    await listen("chat:show", () => {
      if (chatPanel?.systemRestore()) void showChat();
    });
    win.onCloseRequested(() => {
      chatPanel?.intentClose(); // OS 关闭请求 = 用户意图关（统一 API）
    });

    // #8②：chat 头部可拖拽（排除 × 按钮）
    document.addEventListener("mousedown", (e) => {
      const t = e.target as HTMLElement;
      if (t.closest(".chat-header") && !t.closest(".chat-close")) {
        void import("../tauri_runtime_actions").then((m) => m.startDragging(win));
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
  const store = await Store.create(bridge);
  wireTheme(store); // docs/theme.md：新窗口随当前主题，切换即生效
  const mount = document.getElementById("app")!;
  // ChatPanel 不需要 engine，toggle 直接用 DOM show/hide
  chatPanel = new ChatPanel(mount, bridge, store, null!, true);
  // 统一关闭副作用（#26：× / toggle 关 / OS 关闭请求同一收口）：
  // 用户隐藏释放占区、布局入记忆（重开原位恢复），窗口随藏
  chatPanel.onIntentClose = () => {
    void requestRelease("chat-panel");
    void adapter?.hide();
  };

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

async function showChat(edge?: string | null) {
  if (!chatPanel) return;
  chatPanel.showPanel();
  // 按吸附边向屏幕内侧展开（docs/chat-panel.md §布局）；无吸附信息回落默认方位
  const dir = ChatPanel.dirFromEdge(edge);
  const pos = await requestPlace("chat-panel", { id: "chat-panel", width: panelW, height: panelH }, dir);
  await adapter?.setPosition(Math.round(pos.x - panelW / 2), Math.round(pos.y - panelH / 2));
  await adapter?.show();
}
