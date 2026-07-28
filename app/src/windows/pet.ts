// Pet 窗口入口（docs/multi-window.md）：ペット + Autonomy + 位置广播 + 动画窗口自适应
import { Autonomy } from "../autonomy";
import { BrowserMockBridge, createBridge, type Motion } from "../bridge";
import { engine, setupServer } from "../positioning/tauri-server";
import { View } from "../view";
import { createBrowserAdapter, createTauriAdapter, type WindowAdapter } from "../window-adapter";

export async function main() {
  if (!("__TAURI_INTERNALS__" in window)) document.documentElement.classList.add("browser");

  const bridge = await createBridge();
  (window as any).__overseer_bridge_type = bridge.constructor.name;
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

  const ANIM_H = Math.ceil(18 * dpr); // 纵向最大预留（bounce）
  const ANIM_W = Math.ceil(12 * dpr); // 横向最大预留（shake）

  function checkOverflow(label: string, h: number, w: number) {
    if (h > ANIM_H) console.warn(`[pet] ${label} h overflow: ${h} > ${ANIM_H}`);
    if (w > ANIM_W) console.warn(`[pet] ${label} w overflow: ${w} > ${ANIM_W}`);
  }

  let adjustWindowForMotion = async (motion: Motion) => {
    switch (motion) {
      case "bounce": await adapter.setSize(baseW, baseH + ANIM_H); adapter.setOffset(18, 0); break;
      case "float": { const h = Math.ceil(10 * dpr); checkOverflow("float", h, 0); await adapter.setSize(baseW, baseH + h); adapter.setOffset(10, 0); break; }
      case "shake": { const w = ANIM_W; checkOverflow("shake", 0, w); await adapter.setSize(baseW + w, baseH); adapter.setOffset(0, 6); break; }
      default: await adapter.setSize(baseW, baseH); adapter.setOffset(0, 0);
    }
  };

  // ── Tauri 特有 ──
  if (isTauri) {
    view.el.dataset.tauriDragRegion = "";
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const { emit, emitTo } = await import("@tauri-apps/api/event");
    const win = getCurrentWindow();
    setupServer();
    view.tauriStartDrag = () => win.startDragging();

    const { dragDebounce } = await import("../utils/debounce");

    async function broadcastPosition() {
      const pos = await win.outerPosition();
      const size = await win.outerSize();
      const c = { x: pos.x + size.width / 2, y: pos.y + size.height / 2 };
      const vr = view.el.getBoundingClientRect();
      engine.registerPet(c, { w: Math.round(vr.width) + ANIM_W, h: Math.round(vr.height) + ANIM_H });
      emit("pet:moved", c);
      onMove(c);
    }

    const onMove = dragDebounce(
      () => { engine.hideAll(); emit("chat:hide"); emit("cards:hide"); },
      (latest: { x: number; y: number }) => {
        const r = engine.restoreAll(latest);
        if (r.some((w) => w.id === "chat-panel")) emit("chat:show");
        for (const w of r) {
          if (w.id.startsWith("card-")) emit("cards:show", { id: w.id, x: w.center.x, y: w.center.y });
        }
      },
      200,
    );

    // #9: 每个 card 一个独立 Tauri 窗口，由 pet 动态创建
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    bridge.onRenderComponent(async (spec) => {
      (window as any).__overseer_last_render = { ts: Date.now(), id: spec.id, type: spec.type };
      document.title = `🟢 ${spec.id}`;
      const label = `card-${spec.id}`;
      const existing = await WebviewWindow.getByLabel(label);
      (window as any).__overseer_last_label = label;
      if (existing) { existing.close(); engine.remove(label); return; }
      try {
      const webview = new WebviewWindow(label, {
        url: "index.html#card",
        width: 520,
        height: 440,
        decorations: false,
        transparent: true,
        alwaysOnTop: true,
        focus: false,
        shadow: false,
        skipTaskbar: true,
        visible: false,
      });
      webview.once("tauri://created", () => {
        emitTo(label, "card:spec", spec);
      });
      (window as any).__overseer_last_window = "created";
      } catch (e: any) {
        (window as any).__overseer_last_error = String(e?.message ?? e);
        console.error("[pet] WebviewWindow error:", e);
      }
    });

    broadcastPosition();
    await win.onMoved(() => broadcastPosition());
  } else if (!import.meta.env.PROD) {
    // 浏览器模式（仅 Vite dev / preview，prod build tree-shaking 剔除）
    const { ChatPanel } = await import("./chat");
    const { ComponentManager } = await import("../components/component-manager");
    new ComponentManager(mount, bridge, () => view.center());
    const chatPanel = new ChatPanel(mount, bridge, engine);
    view.el.addEventListener("chat:toggle", () => chatPanel.toggle());

    // debug：positioning 面板（α/β 滑块 + 窗口注册）
    const { DebugPositioningPanel } = await import("../positioning/debug-vite-panel");
    const panel = new DebugPositioningPanel(engine);
    // 拖拽时隐藏所有附属窗口，结束后以相对偏移恢复
    let markOffsets: { dx: number; dy: number; css: string }[] = [];
    const syncPanel = () => {
      const wr = view.el.parentElement!.getBoundingClientRect();
      const c = { x: wr.x + wr.width / 2, y: wr.y + wr.height / 2 };
      const r = view.el.getBoundingClientRect();
      panel.setPet(c, { w: Math.round(r.width), h: Math.round(r.height) });
      engine.registerPet(c, { w: Math.round(r.width) + ANIM_W, h: Math.round(r.height) + ANIM_H });
    };
    view.el.addEventListener("view:drag-start", () => {
      engine.hideAll();
      // 同时处理 debug marks
      const wr = view.el.parentElement!.getBoundingClientRect();
      const petX = wr.x + wr.width / 2;
      const petY = wr.y + wr.height / 2;
      markOffsets = [];
      document.querySelectorAll(".dbg-place-mark").forEach((el) => {
        const s = (el as HTMLElement).style;
        markOffsets.push({
          dx: parseFloat(s.left) + 75 - petX,
          dy: parseFloat(s.top) + 50 - petY,
          css: s.cssText,
        });
        el.remove();
      });
      // 隐藏 chatpanel/cards
      chatPanel.hide();
    });
    view.el.addEventListener("view:moved", () => {
      const wr = view.el.parentElement!.getBoundingClientRect();
      const petC = { x: wr.x + wr.width/2, y: wr.y + wr.height/2 };
      syncPanel();
      // 恢复 engine 窗口位置
      const restored = engine.restoreAll(petC);
      for (const r of restored) {
        if (r.id === "chat-panel") chatPanel.toggle();
      }
      // 恢复 debug marks
      for (const mo of markOffsets) {
        const mark = document.createElement("div");
        mark.className = "dbg-place-mark";
        mark.style.cssText = mo.css;
        mark.style.left = `${petC.x + mo.dx - 75}px`;
        mark.style.top = `${petC.y + mo.dy - 50}px`;
        document.body.appendChild(mark);
      }
    });
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

  // 右键 → 通知 chat 窗口弹出（Tauri）
  if (isTauri) {
    const { emit } = await import("@tauri-apps/api/event");
    view.el.addEventListener("chat:toggle", () => { emit("chat:toggle"); });
  }

  await autonomy.init();

  const debug: Record<string, unknown> = {
    setAutonomy: (args: any) => autonomy.setAutonomy(args),
    viewState: () => ({
      center: view.center(),
      face: document.getElementById("face")?.textContent ?? null,
      motion: view.el.dataset.motion ?? "still",
    }),
  };
  if (bridge instanceof BrowserMockBridge) {
    debug.setInstanceStatus = (n: any, s: any) => bridge.debugSetInstanceStatus(n, s);
    debug.addInstance = (n: any, s: any) => bridge.debugAddInstance(n, s);
    debug.notify = (n: any) => bridge.debugNotify(n);
    debug.clearNotifications = () => bridge.debugClearNotifications();
    debug.callComponent = (spec: any) => bridge.debugCallComponent(spec);
    debug.eventBuffer = () => bridge.debugEventBuffer();
    debug.flushEventBuffer = () => bridge.debugFlushEventBuffer();
    debug.appendMessage = (role: any, content: any) => bridge.debugAppendMessage(role, content);
  }
  window.__overseer = debug as any;
}
