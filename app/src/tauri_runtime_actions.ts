// tauri_runtime_actions — WebView 侧非只读 Tauri 运行时动作的唯一出口
// （docs/effect-reporting.md §运行时动作层）。
// 每个语义化动作 = 真实 @tauri-apps/api 写调用，成功后才经 effects 单点记录对应
// effect（失败原样抛错、不写“已发生”的 effect；上报本身 fire-and-forget）。
// 只读调用（getByLabel / outerPosition / currentMonitor / listen）不属于本层，
// 调用处直接使用；浏览器模拟（DOM）不进本层、不埋点。

import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/window";
import { emit, emitTo } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { reportEffect } from "./effects";

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

/** ensure_card_window：Rust 权威注册表同步决策 create/reuse（#25 断根——前端不再
 *  getByLabel 自决存在性，docs/case-runner.md §窗口决策上提）。window_opened 与
 *  card:spec 的 event_emit 由 Rust 端记录，动作层只收口 invoke */
export async function ensureCardWindow(id: string, spec: unknown): Promise<{ result: string }> {
  return (await invoke("ensure_card_window", { id, spec })) as { result: string };
}

/** close_card_window：统一关闭（destroy 同步出注册表，无将死窗口窗口期）。
 *  agent close / shelf dismiss / 用户 × 三路径收口；window_closed 由 Rust 端记录 */
export async function closeCardWindow(id: string): Promise<{ result: string }> {
  return (await invoke("close_card_window", { id })) as { result: string };
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

/** toggle_pet：pet 显隐复合入口（Rust 端逐动作自记 window_hidden/window_visible/event_emit，
 *  动作层只收口 invoke，不重复记录） */
export async function togglePet(): Promise<void> {
  await invoke("toggle_pet");
}

/** quit_app：退出应用（进程随即退出，无 effect 可记） */
export async function quitApp(): Promise<void> {
  await invoke("quit_app");
}
