// 前端进 case 的 shim（docs/case-runner.md §前端进 case，壳类比落地形态）：
//   case-runner 进程主体内嵌 core 并拉起本 TS 测试进程（ambery-case frontend）；
//   测试经 RemoteBridge 连内嵌 core（createBridge 无 __TAURI_INTERNALS__ 自动选中，
//   HTTP+WS 真链路）；本文件不拦截任何 Tauri API，只提供：
//   - 端口接线（__AMBERY_PORT__，必须先于 src 模块导入生效）
//   - core 就绪等待（serve 与 vitest 并发启动，测试可能先跑）
//   - 沙盒 effect.jsonl 读取（AMBERY_STORAGE_DIR 由 case-runner 注入）
//   - mock 窗口层基建（MockWindow 注册表——多窗口测试的窗口动作记录面）
// shell 窗口决策（ensure/close_card_window）是壳侧 Rust 逻辑，由壳 cargo 测试覆盖；
// 本环境跑浏览器分支（无 __TAURI_INTERNALS__），卡片走 ComponentManager DOM 模式。

import { readFileSync } from "node:fs";
import { join } from "node:path";
import WebSocket from "ws"; // node 原生 ws 包：jsdom 环境下 undici WebSocket 有 Event realm 冲突

// 端口接线必须先于任何 src 模块导入（remote.ts 在模块加载时读取该全局）
(globalThis as Record<string, unknown>).__AMBERY_PORT__ =
  process.env.AMBERY_PORT ?? "47600";
// RemoteBridge 的 new WebSocket(...) 走全局构造器——node 下以 ws 包替换（realm 冲突规避）
(globalThis as Record<string, unknown>).WebSocket = WebSocket;

export function coreBase(): string {
  return `http://127.0.0.1:${process.env.AMBERY_PORT ?? "47600"}`;
}

/** 等 case-runner 内嵌 core 就绪（serve 与 vitest 子进程并发启动，测试可能先跑） */
export async function waitCore(ms = 30000): Promise<void> {
  const t0 = Date.now();
  for (;;) {
    try {
      const r = await fetch(`${coreBase()}/state`);
      if (r.ok) return;
    } catch {
      // core 未就绪
    }
    if (Date.now() - t0 > ms) throw new Error("core 未就绪（ambery-case frontend 未完成装配）");
    await new Promise((r) => setTimeout(r, 200));
  }
}

/** 沙盒 effect.jsonl（storage 目录经 AMBERY_STORAGE_DIR 由 case-runner 注入） */
export function readEffects(): string {
  try {
    return readFileSync(join(process.env.AMBERY_STORAGE_DIR!, "effect.jsonl"), "utf8");
  } catch {
    return "";
  }
}

// ── mock 窗口层基建（不拦截 Tauri API；多窗口测试的记录面） ──

export interface WinAction {
  action: string;
  label: string;
  detail?: unknown;
}
export const windowLog: WinAction[] = [];

type WinListener = (ev: { payload: unknown }) => void;

export class MockWindow {
  readonly label: string;
  private listeners = new Map<string, WinListener[]>();
  visible = false;
  destroyed = false;
  constructor(label: string) {
    this.label = label;
  }
  async setSize(size: { width: number; height: number }) {
    windowLog.push({ action: "setSize", label: this.label, detail: size });
  }
  async setPosition(pos: { x: number; y: number }) {
    windowLog.push({ action: "setPosition", label: this.label, detail: pos });
  }
  async show() {
    this.visible = true;
    windowLog.push({ action: "show", label: this.label });
  }
  async hide() {
    this.visible = false;
    windowLog.push({ action: "hide", label: this.label });
  }
  async setFocus() {}
  async close() {
    await this.destroy();
  }
  async destroy() {
    this.destroyed = true;
    windowRegistry.delete(this.label);
    windowLog.push({ action: "destroy", label: this.label });
  }
  async startDragging() {
    windowLog.push({ action: "startDragging", label: this.label });
  }
  async outerSize() {
    return { width: 160, height: 100 };
  }
  async outerPosition() {
    return { x: 100, y: 100 };
  }
  async isVisible() {
    return this.visible;
  }
  async onMoved(_cb: WinListener) {}
  async onCloseRequested(cb: (ev: { preventDefault(): void }) => void) {
    this.listeners.set("tauri://close-requested", [cb as unknown as WinListener]);
  }
  async onFocusChanged(_cb: WinListener) {}
  async listen(event: string, cb: WinListener) {
    const arr = this.listeners.get(event) ?? [];
    arr.push(cb);
    this.listeners.set(event, arr);
    return () => {};
  }
  deliver(event: string, payload: unknown) {
    for (const cb of this.listeners.get(event) ?? []) cb({ payload });
  }
}

const windowRegistry = new Map<string, MockWindow>();
export function setCurrentWindow(label: string) {
  if (!windowRegistry.has(label)) windowRegistry.set(label, new MockWindow(label));
}
export function getMockWindow(label: string) {
  return windowRegistry.get(label);
}

export function resetWindowWorld() {
  windowRegistry.clear();
  windowLog.length = 0;
  setCurrentWindow("pet");
}
