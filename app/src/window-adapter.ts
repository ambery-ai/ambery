// WindowAdapter（feat）：统一窗口尺寸 API，Tauri 模式调 OS 窗口，浏览器模式用红绿框模拟。
// 使 pet.ts 可以在两种环境下一致调试。

export interface WindowAdapter {
  setSize(w: number, h: number): Promise<void>;
  setPosition(x: number, y: number): Promise<void>;
  setOffset(top: number, left: number): void;
  show(): Promise<void>;
  hide(): Promise<void>;
  /** 屏幕逻辑高度（issues #20：card 高度 cap 的唯一取口，docs/window-follow.md §显示器几何） */
  getScreenHeight(): Promise<number>;
}

/** Tauri 模式：真实 OS 窗口 */
export async function createTauriAdapter(
  viewEl: HTMLElement,
  _dpr: number,
): Promise<WindowAdapter> {
  const { getCurrentWindow, currentMonitor, PhysicalSize, PhysicalPosition } = await import("@tauri-apps/api/window");
  const win = getCurrentWindow();

  return {
    async setSize(w: number, h: number) {
      await win.setSize(new PhysicalSize(w, h));
    },
    async setPosition(x: number, y: number) {
      await win.setPosition(new PhysicalPosition(x, y));
    },
    setOffset(top: number, left: number) {
      viewEl.style.top = `${top}px`;
      viewEl.style.left = `${left}px`;
    },
    async show() { await win.show(); await win.setFocus(); },
    async hide() { await win.hide(); },
    async getScreenHeight() {
      const mon = await currentMonitor();
      return mon ? mon.size.height / mon.scaleFactor : 1080;
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
    async show() { wrapper.style.display = ""; overlay.style.display = ""; overlay.style.borderColor = "red"; },
    async hide() { wrapper.style.display = "none"; overlay.style.display = "none"; overlay.style.borderColor = "lime"; },
    async getScreenHeight() {
      return window.screen.availHeight;
    },
  };
}
