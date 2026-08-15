// tauri_runtime_actions — WebView 侧非只读 Tauri 运行时动作的唯一出口
// 
// 每个语义化动作 = 真实 @tauri-apps/api 写调用，成功后才经 effects 单点记录对应
// effect（失败原样抛错、不写“已发生”的 effect；上报本身 fire-and-forget）。
// 只读调用（getByLabel / outerPosition / currentMonitor / listen）不属于本层，
// 调用处直接使用；浏览器模拟（DOM）不进本层、不埋点。

import { PhysicalPosition, PhysicalSize, type Window as TauriWindow } from "@tauri-apps/api/window";
import { emit, emitTo } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { reportEffect } from "./effects";

export interface SizeLike {
  width: number;
  height: number;
}

export interface PositionLike {
  x: number;
  y: number;
}

/** 动作目标：Tauri 真实窗口的适配包装，或 headless 的 MockWindow/DOM 窗口。
 *  统一用普通结构参数——Tauri 物理类型在调用方适配层构造，
 *  这样 mock 窗口层与真实窗口走同一条动作层。 */
export interface WindowLike {
  readonly label: string;
  setSize(size: SizeLike): Promise<void>;
  setPosition(position: PositionLike): Promise<void>;
  show(): Promise<void>;
  setFocus(): Promise<void>;
  hide(): Promise<void>;
  close(): Promise<void>;
  startDragging(): Promise<void>;
}

/** Tauri Window → 普通结构 WindowLike（动作层与 headless mock 窗口共用同一接口） */
export function tauriWindowLike(raw: TauriWindow): WindowLike {
  return {
    label: raw.label,
    setSize: (s) => raw.setSize(new PhysicalSize(s.width, s.height)),
    setPosition: (p) => raw.setPosition(new PhysicalPosition(p.x, p.y)),
    show: () => raw.show(),
    setFocus: () => raw.setFocus(),
    hide: () => raw.hide(),
    close: () => raw.close(),
    startDragging: () => raw.startDragging(),
  };
}

/** resize_window：setSize → window_resized {window, w, h} */
export async function resizeWindow(win: WindowLike, w: number, h: number): Promise<void> {
  await win.setSize({ width: w, height: h });
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
  await win.setPosition({ x, y });
  reportEffect("window_moved", { window: win.label, x, y });
}

/** show_window：show + setFocus → window_visible {window} + window_focused {window}
 *  （一调用两动作分别记录） */
export async function showWindow(win: WindowLike): Promise<void> {
  await win.show();
  await win.setFocus();
  reportEffect("window_visible", { window: win.label });
  reportEffect("window_focused", { window: win.label });
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
 *  getByLabel 自决存在性）。window_opened 与
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

/** export_theme：主题导出到 config_root/themes/<name>.theme.json */
export async function exportTheme(name: string): Promise<{ ok: boolean; path?: string; error?: string }> {
  return (await invoke("export_theme", { name })) as { ok: boolean; path?: string; error?: string };
}

/** import_theme：主题导入（版本检查/兼容/校验在 Rust 端；config_update 由端点记录） */
export async function importTheme(file: string): Promise<{ ok: boolean; name?: string; error?: string }> {
  return (await invoke("import_theme", { file })) as { ok: boolean; name?: string; error?: string };
}
