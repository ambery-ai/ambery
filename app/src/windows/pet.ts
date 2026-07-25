// Pet 窗口入口（docs/multi-window.md）：ペット + Autonomy + 位置广播 + 动画窗口自适应
import { Autonomy } from "../autonomy";
import { BrowserMockBridge, createBridge, type Motion } from "../bridge";
import { View, type Edge } from "../view";
import { createBrowserAdapter, createTauriAdapter, type WindowAdapter } from "../window-adapter";

export async function main() {
  if (!("__TAURI_INTERNALS__" in window)) document.documentElement.classList.add("browser");

  const bridge = await createBridge();
  bridge.getConfig().then((cfg) => {
    document.getElementById("view")!.style.setProperty("--view-scale", String(cfg.viewScale ?? 1));
  });

  const mount = document.getElementById("app")!;
  const view = new View(mount);

  // ── 适配模式 ──
  const isTauri = "__TAURI_INTERNALS__" in window;
  const adapter: WindowAdapter = isTauri
    ? await createTauriAdapter(view.el, window.devicePixelRatio || 1)
    : createBrowserAdapter(mount, view.el, view);

  // ── 初始测量 & 动画 ──
  const dpr = isTauri ? (window.devicePixelRatio || 1) : 1;
  const r = view.el.getBoundingClientRect();
  let baseW = Math.ceil(r.width * dpr);
  let baseH = Math.ceil(r.height * dpr);
  await adapter.setSize(baseW, baseH);
  adapter.setOffset(0, 0);

  let adjustWindowForMotion = async (motion: Motion) => {
    switch (motion) {
      case "bounce": await adapter.setSize(baseW, baseH + Math.ceil(18 * dpr)); adapter.setOffset(18, 0); break;
      case "float": await adapter.setSize(baseW, baseH + Math.ceil(10 * dpr)); adapter.setOffset(10, 0); break;
      case "shake": await adapter.setSize(baseW + Math.ceil(12 * dpr), baseH); adapter.setOffset(0, 6); break;
      default: await adapter.setSize(baseW, baseH); adapter.setOffset(0, 0);
    }
  };

  // ── Tauri 特有 ──
  if (isTauri) {
    view.el.dataset.tauriDragRegion = "";
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const { emit } = await import("@tauri-apps/api/event");
    const win = getCurrentWindow();
    view.tauriStartDrag = () => win.startDragging();
    async function broadcastPosition() {
      const pos = await win.outerPosition();
      const size = await win.outerSize();
      emit("pet:moved", { x: pos.x + size.width / 2, y: pos.y + size.height / 2 });
    }
    broadcastPosition();
    await win.onMoved(() => broadcastPosition());
  } else {
    // 浏览器模式：ChatPanel / ComponentManager
    const { ChatPanel } = await import("../chat");
    const { ComponentManager } = await import("../components/component-manager");
    new ComponentManager(mount, bridge, () => view.center());
    const chatPanel = new ChatPanel(mount, bridge, () => view.center());
    let dockedEdge: Edge = "top";
    view.el.addEventListener("view:docked", (ev) => {
      dockedEdge = (ev as CustomEvent<{ edge: Edge }>).detail.edge;
      chatPanel.show(dockedEdge);
    });
    view.el.addEventListener("view:undocked", () => chatPanel.hide());
    view.el.addEventListener("click", () => {
      if (view.isDocked() && !chatPanel.isVisible()) chatPanel.show(dockedEdge);
    });

    // debug：positioning 面板（α/β 滑块 + 窗口注册）
    const { PositioningEngine } = await import("../positioning/engine");
    const { DebugPositioningPanel } = await import("../positioning/debug-vite-panel");
    const engine = new PositioningEngine();
    const panel = new DebugPositioningPanel(engine);
    const syncPanel = () => {
      const c = view.center();
      const r = view.el.getBoundingClientRect();
      panel.setPet(c, { w: Math.round(r.width), h: Math.round(r.height) });
      engine.registerPet(c, { w: Math.round(r.width), h: Math.round(r.height) });
    };
    view.el.addEventListener("view:moved", syncPanel);
    view.el.addEventListener("view:docked", syncPanel);
    view.el.addEventListener("view:undocked", syncPanel);
    syncPanel();
  }

  const autonomy = new Autonomy(bridge, (e) => {
    view.setExpression(e);
    const rr = view.el.getBoundingClientRect();
    baseW = Math.ceil(rr.width * dpr);
    baseH = Math.ceil(rr.height * dpr);
    adjustWindowForMotion(e.motion);
  });
  bridge.onSetAutonomy?.((args) => autonomy.setAutonomy(args));
  bridge.onConfigChanged?.((cfg) => autonomy.updateConfig(cfg));

  // 吸附态 → 通知 chat 窗口弹出（Tauri）
  if (isTauri) {
    const { emit } = await import("@tauri-apps/api/event");
    view.el.addEventListener("view:docked", (ev) => { emit("chat:show", (ev as CustomEvent).detail); });
    view.el.addEventListener("view:undocked", () => { emit("chat:hide"); });
    view.el.addEventListener("click", () => { if (view.isDocked()) emit("chat:show", { edge: view.dockEdge() }); });
  }

  await autonomy.init();

  const debug: Partial<Window["__overseer"]> = {
    setAutonomy: (args) => autonomy.setAutonomy(args),
    viewState: () => ({
      docked: view.isDocked(), center: view.center(),
      face: document.getElementById("face")?.textContent ?? null,
      motion: view.el.dataset.motion ?? "still",
    }),
  };
  if (bridge instanceof BrowserMockBridge) {
    debug.setInstanceStatus = (n, s) => bridge.debugSetInstanceStatus(n, s);
    debug.addInstance = (n, s) => bridge.debugAddInstance(n, s);
    debug.notify = (n) => bridge.debugNotify(n);
    debug.clearNotifications = () => bridge.debugClearNotifications();
    debug.callComponent = (spec) => bridge.debugCallComponent(spec);
    debug.eventBuffer = () => bridge.debugEventBuffer();
    debug.flushEventBuffer = () => bridge.debugFlushEventBuffer();
    debug.appendMessage = (role, content) => bridge.debugAppendMessage(role, content);
  }
  window.__overseer = debug as Window["__overseer"];
}
