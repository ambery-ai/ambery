// tauri_runtime_actions — WebView 侧非只读 Tauri 运行时动作的唯一出口
// （docs/effect-reporting.md §运行时动作层）。
// 每个语义化动作 = 真实 @tauri-apps/api 写调用，成功后才经 effects 单点记录对应
// effect（失败原样抛错、不写“已发生”的 effect；上报本身 fire-and-forget）。
// 只读调用（getByLabel / outerPosition / currentMonitor / listen）不属于本层，
// 调用处直接使用；浏览器模拟（DOM）不进本层、不埋点。

import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { emit, emitTo } from "@tauri-apps/api/event";
import { reportEffect } from "./effects";

/** WebviewWindow 构造参数（url + 窗口选项的完整交集类型，跟随 API 版本） */
type CardWindowOptions = ConstructorParameters<typeof WebviewWindow>[1];

/** 动作目标：`@tauri-apps/api/window` 的 Window 与 webviewWindow 的 WebviewWindow
 *  共有的写方法结构子集——动作层不关心调用处拿到的是哪一种 */
export interface WindowLike {
  readonly label: string;
  setSize(size: PhysicalSize): Promise<void>;
  setPosition(position: PhysicalPosition): Promise<void>;
  show(): Promise<void>;
  setFocus(): Promise<void>;
  hide(): Promise<void>;
  close(): Promise<void>;
  startDragging(): Promise<void>;
}

/** resize_window：setSize → window_resized {window, w, h} */
export async function resizeWindow(win: WindowLike, w: number, h: number): Promise<void> {
  await win.setSize(new PhysicalSize(w, h));
  reportEffect("window_resized", { window: win.label, w, h });
}

/** resize_window 的 setOffset 半动作（Tauri 模式内 adapter 的 CSS 偏移）→ window_resized {window, top, left} */
export function offsetWindow(win: WindowLike, el: HTMLElement, top: number, left: number): void {
  el.style.top = `${top}px`;
  el.style.left = `${left}px`;
  reportEffect("window_resized", { window: win.label, top, left });
}

/** move_window：setPosition → window_moved {window, x, y} */
export async function moveWindow(win: WindowLike, x: number, y: number): Promise<void> {
  await win.setPosition(new PhysicalPosition(x, y));
  reportEffect("window_moved", { window: win.label, x, y });
}

/** show_window：show + setFocus → window_visible {window} */
export async function showWindow(win: WindowLike): Promise<void> {
  await win.show();
  await win.setFocus();
  reportEffect("window_visible", { window: win.label });
}

/** hide_window：hide → window_hidden {window} */
export async function hideWindow(win: WindowLike): Promise<void> {
  await win.hide();
  reportEffect("window_hidden", { window: win.label });
}

/** start_dragging：startDragging → window_drag {window} */
export async function startDragging(win: WindowLike): Promise<void> {
  await win.startDragging();
  reportEffect("window_drag", { window: win.label });
}

/** create_card_window：new WebviewWindow；tauri://created 成功 → window_opened {window}
 *  （tauri://error = 创建失败，只回调不记录）；onCreated 在记录之后链入业务收尾 */
export function createCardWindow(
  label: string,
  options: CardWindowOptions,
  onCreated?: () => void,
  onError?: (e: unknown) => void,
): WebviewWindow {
  const webview = new WebviewWindow(label, options);
  webview.once("tauri://created", () => {
    reportEffect("window_opened", { window: label });
    onCreated?.();
  });
  webview.once("tauri://error", (e) => onError?.(e));
  return webview;
}

/** close_window：close 成功 → window_closed {window} */
export async function closeWindow(win: WindowLike): Promise<void> {
  const label = win.label;
  await win.close();
  reportEffect("window_closed", { window: label });
}

/** emit_event：emit / emitTo 成功 → event_emit {event, target?}
 *  （先执行真实发送再记录；调用方可 void 保持 fire-and-forget） */
export async function emitEvent(event: string, payload?: unknown, target?: string): Promise<void> {
  if (target !== undefined) {
    await emitTo(target, event, payload);
    reportEffect("event_emit", { event, target });
  } else {
    await emit(event, payload);
    reportEffect("event_emit", { event });
  }
}
