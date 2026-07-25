// WindowAdapter（feat）：统一窗口尺寸 API，Tauri 模式调 OS 窗口，浏览器模式用红绿框模拟。
// 使 pet.ts 可以在两种环境下一致调试。

export interface WindowAdapter {
  /** 设置窗口/容器尺寸（物理 px） */
  setSize(w: number, h: number): Promise<void>;
  /** 设置窗口位置（物理 px） */
  setPosition(x: number, y: number): Promise<void>;
  /** 设置 View 在窗口内的偏移（CSS px，主要用于 pet 动画） */
  setOffset(top: number, left: number): void;
  /** 显示窗口 */
  show(): Promise<void>;
  /** 隐藏窗口 */
  hide(): Promise<void>;
}

/** Tauri 模式：真实 OS 窗口 */
export async function createTauriAdapter(
  viewEl: HTMLElement,
  dpr: number,
): Promise<WindowAdapter> {
  const { getCurrentWindow, PhysicalSize, PhysicalPosition } = await import("@tauri-apps/api/window");
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
  };
}

/** 浏览器调试模式：wrapper 容器定位，内放 View + overlay 红绿框 */
export function createBrowserAdapter(
  mount: HTMLElement,
  viewEl: HTMLElement,
): WindowAdapter {
  const wrapper = document.createElement("div");
  wrapper.id = "debug-wrapper";
  wrapper.style.cssText = "position:fixed;z-index:9999;";

  // overlay 填满 wrapper，紧贴边界
  const overlay = document.createElement("div");
  overlay.id = "debug-frame";
  overlay.style.cssText = "position:absolute;inset:0;border:2px solid red;pointer-events:none;background:transparent";

  // 把 View 移入 wrapper，定位改为 relative
  const prevParent = viewEl.parentElement;
  wrapper.appendChild(viewEl);
  wrapper.appendChild(overlay);
  mount.appendChild(wrapper);
  viewEl.style.position = "absolute"; // 跟 wrapper 移动，不受宽度约束
  (viewEl as any).dragTarget = wrapper;

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
    async show() { wrapper.style.display = ""; overlay.style.borderColor = "red"; },
    async hide() { wrapper.style.display = "none"; overlay.style.borderColor = "lime"; },
  };
}
