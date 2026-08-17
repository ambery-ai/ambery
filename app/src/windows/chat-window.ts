// Chat 窗口入口：ChatPanel + 窗口定位
// Tauri B 方案：通过 requestPlace/requestRemove 调 pet 窗 engine
import { createBridge } from "../bridge";
import { ChatPanel } from "./chat";
import { openSetupModal } from "../setup";
import { Store } from "../store";
import { wireTheme } from "../theme";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import { requestPlace, requestRelease, reportMoved } from "../positioning/tauri-server";
import { Direction } from "../positioning/types";

let chatPanel: ChatPanel | null = null;
let adapter: WindowAdapter | null = null;
let panelW = 320;
let panelH = 380;
/** 未配置检测是否已跑过（首次打开 chat 时弹 modal，之后不重复弹） */
let checkedUnconfigured = false;

export async function main() {
  if ("__TAURI_INTERNALS__" in window) {
    adapter = await createTauriAdapter(document.body, 1);
    const { listen } = await import("@tauri-apps/api/event");
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();
    await listen("pet:moved", () => {}); // 占位，确保事件系统初始化
    // 唤出/关闭由 pet 右键 toggle 驱动：chat:toggle → 开则关、关则开；
    // 位置经 engine.place 固定 sse（自己的定位引擎，非 OS 贴靠）
    await listen("chat:toggle", () => {
      if (chatPanel?.isVisible()) {
        chatPanel.intentClose(); // 统一 API（onIntentClose 钩子做 release+hide）
        return;
      }
      chatPanel?.intentOpen();
      void showChat();
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
        void import("../tauri_runtime_actions").then((m) =>
          m.startDragging(m.tauriWindowLike(win)),
        );
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
  bridgeRefCached = bridge;
  const store = await Store.create(bridge);
  wireTheme(store); // 新窗口随当前主题，切换即生效
  const mount = document.getElementById("app")!;
  // ChatPanel 不需要 engine，toggle 直接用 DOM show/hide
  chatPanel = new ChatPanel(mount, bridge, store, null!, true);
  // 打开配置引导 modal（未配置/连接失败同一 modal 两种状态）
  let setupDismiss: (() => void) | null = null;
  chatPanel.onOpenSetup = () => {
    setupDismiss?.();
    setupDismiss = openSetupModal(bridge);
  };
  // 统一关闭副作用（#26：× / toggle 关 / OS 关闭请求同一收口）：
  // 用户隐藏释放占区、布局入记忆（重开原位恢复），窗口随藏
  chatPanel.onIntentClose = () => {
    setupDismiss?.();
    setupDismiss = null;
    void requestRelease("chat-panel");
    void adapter?.hide();
  };
  // LLM 未配置检测（llm-setup.md）：在 chat 打开路径触发（showChat），不在窗口初始化时——
  // 窗口常驻，初始化即弹会弹在隐藏窗口里（只弹 modal 不开 chat）

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

/** 模块级 bridge 引用（main 初始化后赋值；showChat 的未配置检测用） */
let bridgeRefCached: import("../bridge").Bridge | null = null;

async function showChat() {
  if (!chatPanel || !bridgeRefCached) return;
  chatPanel.showPanel();
  // LLM 未配置检测：首次打开 chat 时弹 modal + 提示条（之后不重复弹）
  if (!checkedUnconfigured) {
    checkedUnconfigured = true;
    await checkUnconfigured(bridgeRefCached, chatPanel);
  }
  // 固定 sse 方位经 engine.place 落到 pet 旁
  const pos = await requestPlace("chat-panel", { id: "chat-panel", width: panelW, height: panelH }, Direction.sse);
  await adapter?.setPosition(Math.round(pos.x - panelW / 2), Math.round(pos.y - panelH / 2));
  await adapter?.show();
}

/** LLM 未配置检测（docs/llm-setup.md）：llm.active == "unconfigured" → 弹引导 modal + 提示条 */
async function checkUnconfigured(bridge: import("../bridge").Bridge, panel: ChatPanel) {
  try {
    const resp = await bridge.getConfigSchema!();
    const active = resp.nodes.find((n) => n.path === "llm.active")?.value;
    if (active === "unconfigured") {
      panel.showSetupHint();
      panel.onOpenSetup?.();
    }
  } catch {
    // core 不可达：不弹（offline 已有独立提示）
  }
}
