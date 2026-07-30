// Pet 窗口入口（docs/multi-window.md）：ペット + Autonomy + 位置广播 + 动画窗口自适应
import { Autonomy } from "../autonomy";
import { BrowserMockBridge, createBridge, type Motion } from "../bridge";
import { engine, setupServer } from "../positioning/tauri-server";
import { View } from "../view";
import { createBrowserAdapter, createTauriAdapter, type WindowAdapter } from "../window-adapter";

export async function main() {
  if (!("__TAURI_INTERNALS__" in window)) document.documentElement.classList.add("browser");

  const bridge = await createBridge();
  bridge.getConfig().then((cfg) => {
    document.getElementById("view")!.style.setProperty("--view-scale", String(cfg.viewScale ?? 1));
  });

  const mount = document.getElementById("app")!;
  const view = new View(mount);

  // #5 pet 未读角标（spec：默认纯数字、容器内右上；样式/方位走 Config）
  const badge = document.createElement("div");
  badge.id = "pet-badge";
  const applyBadgeStyle = (style: string, side: string) => {
    const pos = side === "left" ? "left:8px;" : "right:8px;";
    const base = `display:none;position:absolute;${pos}top:50%;transform:translateY(-50%);font-size:12px;font-weight:700;line-height:1;z-index:10;pointer-events:none;`;
    badge.style.cssText = style === "bubble"
      ? `${base}background:#f38ba8;color:#fff;border-radius:10px;padding:1px 6px;`
      : `${base}color:#f38ba8;`;
  };
  applyBadgeStyle("number", "right"); // 默认（Config 加载后覆盖）
  view.el.appendChild(badge);
  bridge.getConfig().then((cfg) => {
    applyBadgeStyle(cfg.badgeStyle ?? "number", cfg.badgeSide ?? "right");
  });
  let unreadCount = 0;
  bridge.onContextChanged((msgs) => {
    const userMsgs = msgs.filter(m => m.role === "user").length;
    const prev = unreadCount > 0 ? unreadCount : userMsgs;
    const newAssist = msgs.filter(m => m.role === "assistant").length;
    unreadCount = Math.max(0, newAssist - prev);
    badge.textContent = String(unreadCount);
    badge.style.display = unreadCount > 0 ? "" : "none";
  });

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
      // #19 坐标契约：petCenter 物理 → pet 尺寸必须同帧（DOM 值 ×dpr 才进 engine）
      const dpr = window.devicePixelRatio || 1;
      engine.registerPet(c, { w: Math.round(vr.width * dpr) + ANIM_W, h: Math.round(vr.height * dpr) + ANIM_H });
      emit("pet:moved", c);
      onMove(c);
    }

    const onMove = dragDebounce(
      // 系统藏（#12 定案：不动 engine，无快照）
      () => { emit("chat:hide"); emit("cards:hide"); },
      (latest: { x: number; y: number }) => {
        const r = engine.restorePositions(latest);
        if (r.some((w) => w.id === "chat-panel")) emit("chat:show");
        for (const w of r) {
          if (w.id.startsWith("card-")) emit("cards:show", { id: w.id, x: w.center.x, y: w.center.y });
        }
      },
      200,
    );

    // #13: pet 隐藏时 card 窗口延迟到恢复显示
    let petVisible = true;
    type PendingCard = { label: string; spec: any };
    const pendingCards: PendingCard[] = [];

    const { listen } = await import("@tauri-apps/api/event");
    listen("pet:hidden", () => { petVisible = false; });
    listen("pet:shown", () => {
      petVisible = true;
      for (const pc of pendingCards.splice(0)) {
        emitTo(pc.label, "card:spec", pc.spec);
      }
      // 托盘回来：恢复位置广播（#12 定案 grill⑤——系统藏的系统恢复，各窗口自查 userClosed）
      void (async () => {
        const pos = await win.outerPosition();
        const size = await win.outerSize();
        const c = { x: pos.x + size.width / 2, y: pos.y + size.height / 2 };
        const r = engine.restorePositions(c);
        if (r.some((w) => w.id === "chat-panel")) emit("chat:show");
        for (const w of r) {
          if (w.id.startsWith("card-")) emit("cards:show", { id: w.id, x: w.center.x, y: w.center.y });
        }
      })();
    });

    // #9: 每个 card 一个独立 Tauri 窗口，由 pet 动态创建
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    bridge.onRenderComponent(async (spec) => {
      const label = `card-${spec.id}`;
      const existing = await WebviewWindow.getByLabel(label);
      // 持续管理协议：同 id = 原地更新（重发 spec），不再 toggle 关闭
      if (existing) {
        emitTo(label, "card:spec", spec);
        return;
      }
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
      webview.once("tauri://created", async () => {
        await new Promise(r => setTimeout(r, 500));
        if (petVisible) {
          emitTo(label, "card:spec", spec);
        } else {
          pendingCards.push({ label, spec });
        }
      });
      webview.once("tauri://error", (e: any) => {
        console.error("[pet] WebviewWindow error:", e);
      });
    });

    // 显式关闭（持续管理协议：agent close action）
    bridge.onCloseComponent?.(async (id) => {
      const label = `card-${id}`;
      const existing = await WebviewWindow.getByLabel(label);
      if (existing) await existing.close();
      engine.remove(label);
    });

    broadcastPosition();
    await win.onMoved(() => broadcastPosition());
  } else if (!import.meta.env.PROD) {
    // 浏览器模式（仅 Vite dev / preview，prod build tree-shaking 剔除）
    const { ChatPanel } = await import("./chat");
    const { ComponentManager } = await import("../components/component-manager");
    const mgr = new ComponentManager(mount, bridge, () => view.center(), false, engine);
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
      // 系统藏（统一 API，无快照，#12 定案）；debug marks 单独处理
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
      chatPanel.systemHide();
      mgr.systemHideAll();
    });
    view.el.addEventListener("view:moved", () => {
      const wr = view.el.parentElement!.getBoundingClientRect();
      const petC = { x: wr.x + wr.width/2, y: wr.y + wr.height/2 };
      syncPanel();
      // 系统恢复（统一 API：systemRestore 判定 + showAt 定位，不再 toggle）
      const restored = engine.restorePositions(petC);
      for (const r of restored) {
        if (r.id === "chat-panel" && chatPanel.systemRestore()) {
          chatPanel.showAt(r.center);
        }
      }
      // card 跟随（browser DOM 卡片纳入 engine 语义，#12）
      mgr.followRestore(restored);
      mgr.systemShowAll();
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

  // #18：motion 预留量（用于从现 rect 中扣掉动画增量，得回「内容基准」）
  const motionExtra = (m: Motion): { w: number; h: number } => {
    switch (m) {
      case "bounce": return { w: 0, h: ANIM_H };
      case "float": return { w: 0, h: Math.ceil(10 * dpr) };
      case "shake": return { w: ANIM_W, h: 0 };
      default: return { w: 0, h: 0 };
    }
  };
  let curExtraW = 0, curExtraH = 0;

  const autonomy = new Autonomy(bridge, (e) => {
    view.setExpression(e);
    const rr = view.el.getBoundingClientRect();
    // #18：基准 = 现 rect − 当前 motion 预留——防止动画增量污染基准导致累积膨胀
    // （原实现直接拿 rect 当基准，Tauri 窗口被 setSize 撑大后基准跟着涨）
    baseW = Math.ceil(rr.width * dpr - curExtraW);
    baseH = Math.ceil(rr.height * dpr - curExtraH);
    const extra = motionExtra(e.motion);
    curExtraW = extra.w;
    curExtraH = extra.h;
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
