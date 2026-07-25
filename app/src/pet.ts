// Pet 窗口入口（docs/multi-window.md）：ペット + Autonomy + 位置广播 + 动画窗口自适应
import { Autonomy } from "./autonomy";
import { BrowserMockBridge, createBridge, type Motion } from "./bridge";
import { View, type Edge } from "./view";
import { createBrowserAdapter, createTauriAdapter, type WindowAdapter } from "./window-adapter";

export async function main() {
  // 浏览器调试模式：加暗色背景
  if (!("__TAURI_INTERNALS__" in window)) document.documentElement.classList.add("browser");

  const bridge = await createBridge();
  bridge.getConfig().then((cfg) => {
    document.getElementById("view")!.style.setProperty("--view-scale", String(cfg.viewScale ?? 1));
  });

  const mount = document.getElementById("app")!;
  const view = new View(mount);

  // Tauri 模式：窗口自适应 View 尺寸 + 动画动态扩缩
  let adjustWindowForMotion: ((m: Motion) => void) | null = null;
  if ("__TAURI_INTERNALS__" in window) {
    view.el.dataset.tauriDragRegion = "";
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const { emit } = await import("@tauri-apps/api/event");
    const win = getCurrentWindow();
    const adapter: WindowAdapter = await createTauriAdapter(view.el, window.devicePixelRatio || 1);

    // 量 View 实际尺寸 → 窗口贴合（DPI 修正：CSS px → 物理 px）
    const r = view.el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    let baseW = Math.ceil(r.width * dpr);
    let baseH = Math.ceil(r.height * dpr);
    await adapter.setSize(baseW, baseH);
    adapter.setOffset(0, 0);

    // 颜文字变化 → 窗口自适应 + 更新动画 base，动画偏移在 resize 后执行
    let pendingMotion: Motion | null = null;
    const resizeWindow = () => {
      const rr = view.el.getBoundingClientRect();
      baseW = Math.ceil(rr.width * dpr);
      baseH = Math.ceil(rr.height * dpr);
      adapter.setSize(baseW, baseH);
      if (pendingMotion !== null) {
        adjustWindowForMotion(pendingMotion);
        pendingMotion = null;
      }
    };
    new ResizeObserver(() => resizeWindow()).observe(view.el);

    // 动画播放时动态扩大窗口 → 结束后缩回（DPI 修正）
    adjustWindowForMotion = async (motion: Motion) => {
      switch (motion) {
        case "bounce":
          await adapter.setSize(baseW, baseH + Math.ceil(18 * dpr));
          adapter.setOffset(18, 0);
          break;
        case "float":
          await adapter.setSize(baseW, baseH + Math.ceil(10 * dpr));
          adapter.setOffset(10, 0);
          break;
        case "shake":
          await adapter.setSize(baseW + Math.ceil(12 * dpr), baseH);
          adapter.setOffset(0, 6);
          break;
        default: // still
          await adapter.setSize(baseW, baseH);
          adapter.setOffset(0, 0);
      }
    };

    view.tauriStartDrag = () => win.startDragging();
    async function broadcastPosition() {
      const pos = await win.outerPosition();
      const size = await win.outerSize();
      emit("pet:moved", {
        x: pos.x + size.width / 2,
        y: pos.y + size.height / 2,
      });
    }
    broadcastPosition();
    await win.onMoved(() => broadcastPosition());
  }

  const autonomy = new Autonomy(bridge, (e) => {
    view.setExpression(e);
    adjustWindowForMotion?.(e.motion);
  });
  bridge.onSetAutonomy?.((args) => autonomy.setAutonomy(args));
  bridge.onConfigChanged?.((cfg) => autonomy.updateConfig(cfg));

  // 吸附态 → 通知 chat 窗口弹出
  if ("__TAURI_INTERNALS__" in window) {
    const { emit } = await import("@tauri-apps/api/event");
    view.el.addEventListener("view:docked", (ev) => { emit("chat:show", (ev as CustomEvent).detail); });
    view.el.addEventListener("view:undocked", () => { emit("chat:hide"); });
    view.el.addEventListener("click", () => { if (view.isDocked()) emit("chat:show", { edge: view.dockEdge() }); });
  } else {
    // 浏览器模式：ChatPanel / ComponentManager 内联在当前窗口
    // 浏览器模式：仅 pet 窗口需 adapter 模拟 Tauri 窗口边界；
    // chat/cards 在多窗口架构中独立加载走 TauriAdapter，浏览器不分路由到它们
    const adapter = createBrowserAdapter(mount, view.el);
    const r = view.el.getBoundingClientRect();
    await adapter.setSize(Math.ceil(r.width), Math.ceil(r.height));
    adapter.setOffset(0, 0);
    const { ChatPanel } = await import("./chat");
    const { ComponentManager } = await import("./components");
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
  }

  await autonomy.init();

  // Chrome DevTools 调试接口
  const debug: Partial<Window["__overseer"]> = {
    setAutonomy: (args) => autonomy.setAutonomy(args),
    viewState: () => ({
      docked: view.isDocked(),
      center: view.center(),
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
