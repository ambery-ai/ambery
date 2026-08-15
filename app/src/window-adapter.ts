// WindowAdapter（feat）：统一窗口尺寸 API，Tauri 模式调 OS 窗口，浏览器模式用红绿框模拟。
// 使 pet.ts 可以在两种环境下一致调试。

export interface WindowAdapter {
  setSize(w: number, h: number): Promise<void>;
  setPosition(x: number, y: number): Promise<void>;
  /** 当前窗口左上角（engine 帧：Tauri 物理 px / browser CSS px）——中心锚定的读取入口 */
  getPosition(): Promise<{ x: number; y: number }>;
  setOffset(top: number, left: number): void;
  show(): Promise<void>;
  hide(): Promise<void>;
  /** 屏幕逻辑高度（issues #20：card 高度 cap 的唯一取口） */
  getScreenHeight(): Promise<number>;
}

/** Tauri 模式：真实 OS 窗口。写操作全部经 WebView 动作层执行+留痕
 *  （tauri_runtime_actions）；读取直调。 */
export async function createTauriAdapter(
  viewEl: HTMLElement,
  _dpr: number,
): Promise<WindowAdapter> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const actions = await import("./tauri_runtime_actions");
  const win = getCurrentWindow();

  return {
    setSize: (w, h) => actions.resizeWindow(win, w, h),
    setPosition: (x, y) => actions.moveWindow(win, x, y),
    async getPosition() {
      const p = await win.outerPosition();
      return { x: p.x, y: p.y };
    },
    setOffset: (top, left) => actions.offsetWindow(win, viewEl, top, left),
    show: () => actions.showWindow(win),
    hide: () => actions.hideWindow(win),
    async getScreenHeight() {
      // 读缓存 monitor 表（其 Tauri 实现内部读本表；
      // 窗口实际所在屏 = 当前位置命中项，多屏正确）
      const p = await win.outerPosition();
      const { monitorOf } = await import("./positioning/monitors");
      const m = monitorOf({ x: p.x, y: p.y });
      return m.height / m.scaleFactor;
    },
  };
}

/** 浏览器调试模式：wrapper 容器定位，内放 View + overlay 红绿框 */
export function createBrowserAdapter(
  mount: HTMLElement,
  viewEl: HTMLElement,
  viewInstance?: { dragTarget: HTMLElement },
): WindowAdapter {
  const wrapper = document.createElement("div");
  wrapper.id = "debug-wrapper";
  wrapper.style.cssText = "position:fixed;";
  wrapper.appendChild(viewEl);
  viewEl.style.position = "absolute";
  if (viewInstance) viewInstance.dragTarget = wrapper;
  // 同步 wrapper 初始位置到 View screen 坐标，View 偏移归零
  const vr0 = viewEl.getBoundingClientRect();
  wrapper.style.left = `${vr0.x}px`;
  wrapper.style.top = `${vr0.y}px`;
  viewEl.style.left = "0px";
  viewEl.style.top = "0px";

  // overlay 独立 div，兄弟节点叠在 wrapper 上
  const overlay = document.createElement("div");
  overlay.id = "debug-frame";
  overlay.style.cssText = "position:fixed;border:2px solid red;pointer-events:none;background:transparent;z-index:9998";
  mount.appendChild(wrapper);
  mount.appendChild(overlay);

  const syncOverlay = () => {
    const wr = wrapper.getBoundingClientRect();
    overlay.style.top = `${wr.top}px`;
    overlay.style.left = `${wr.left}px`;
    overlay.style.width = `${wr.width}px`;
    overlay.style.height = `${wr.height}px`;
    requestAnimationFrame(syncOverlay);
  };
  requestAnimationFrame(syncOverlay);

  return {
    async setSize(w: number, h: number) {
      wrapper.style.width = `${w}px`;
      wrapper.style.height = `${h}px`;
    },
    setOffset(top: number, left: number) {
      viewEl.style.top = `${top}px`;
      viewEl.style.left = `${left}px`;
    },
    async setPosition(x: number, y: number) {
      wrapper.style.left = `${x}px`;
      wrapper.style.top = `${y}px`;
    },
    async getPosition() {
      const r = wrapper.getBoundingClientRect();
      return { x: r.left, y: r.top };
    },
    async show() { wrapper.style.display = ""; overlay.style.display = ""; overlay.style.borderColor = "red"; },
    async hide() { wrapper.style.display = "none"; overlay.style.display = "none"; overlay.style.borderColor = "lime"; },
    async getScreenHeight() {
      return window.screen.availHeight;
    },
  };
}
