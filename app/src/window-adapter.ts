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

/** 浏览器调试模式：红框=窗口边界 绿框=face边界 */
export function createBrowserAdapter(
  mount: HTMLElement,
  viewEl: HTMLElement,
): WindowAdapter {
  const frame = document.createElement("div");
  frame.id = "debug-frame";
  frame.style.cssText = "position:fixed;border:2px solid red;pointer-events:none;z-index:9999;background:transparent";
  mount.appendChild(frame);

  let offsetTop = 0;
  let offsetLeft = 0;

  // 每帧同步 overlay 位置（覆盖拖拽、动画、dock 等所有 View 位移）
  const tick = () => {
    const vr = viewEl.getBoundingClientRect();
    frame.style.top = `${vr.top - offsetTop}px`;
    frame.style.left = `${vr.left - offsetLeft}px`;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);

  return {
    async setSize(w: number, h: number) {
      frame.style.width = `${w}px`;
      frame.style.height = `${h}px`;
    },
    setOffset(top: number, left: number) {
      offsetTop = top;
      offsetLeft = left;
    },
    async setPosition(x: number, y: number) {
      frame.style.left = `${x}px`;
      frame.style.top = `${y}px`;
    },
    async show() { frame.style.display = ""; frame.style.borderColor = "red"; },
    async hide() { frame.style.display = "none"; frame.style.borderColor = "lime"; },
  };
}
