// 前端进 case v2 的 shim（docs/case-runner.md §前端进 case）：
//   真前端 JS（vitest + jsdom）+ 真 core（overseer-debug 子进程，沙盒 storage/config）
//   + window.__TAURI_INTERNALS__ 占位（createBridge 选中真生产桥 TauriBridge）
//   + vi.mock 拦截 4 个 Tauri API 模块：invoke → HTTP 到 core；listen ← WS 订阅 effect 流；
//     WebviewWindow / window → 内存 mock（窗口动作序列进 windowLog 供断言）
// shell 窗口决策（ensure/close_card_window）由 mock 注册表复刻决策语义（生产逻辑由
// 壳 cargo 测试覆盖，此处测前端接线）；窗口层照发 window_opened / window_closed
// effect（POST /effect，与生产同通道落 effect.jsonl）。

import { vi } from "vitest";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import WebSocket from "ws"; // node 原生 ws 包：jsdom 环境下 undici WebSocket 有 Event realm 冲突

// ── core 子进程生命周期 ──

export interface CoreHandle {
  base: string;
  dir: string;
  proc: ChildProcess;
}

export async function startCore(port = 47655): Promise<CoreHandle> {
  const dir = mkdtempSync(join(tmpdir(), "overseer-vitest-"));
  const bin = join(__dirname, "../../target/debug/overseer-debug.exe");
  const proc = spawn(bin, [], {
    env: {
      ...process.env,
      OVERSEER_STORAGE_DIR: dir,
      OVERSEER_CONFIG_DIR: dir,
      OVERSEER_PORT: String(port),
      OVERSEER_SIDECAR: "", // 无 sidecar：读通道只剩 mock 面
    },
    stdio: ["ignore", "ignore", "pipe"],
  });
  let stderr = "";
  proc.stderr?.on("data", (d) => (stderr += String(d)));
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 120; i++) {
    try {
      const r = await fetch(`${base}/state`);
      if (r.ok) return { base, dir, proc };
    } catch {
      if (proc.exitCode !== null) throw new Error(`core 早退: ${stderr.slice(-400)}`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  proc.kill();
  throw new Error(`core 未就绪: ${stderr.slice(-400)}`);
}

export function stopCore(h: CoreHandle) {
  h.proc.kill();
  rmSync(h.dir, { recursive: true, force: true });
}

export function readEffects(h: CoreHandle): string {
  try {
    return readFileSync(join(h.dir, "effect.jsonl"), "utf8");
  } catch {
    return "";
  }
}

// ── 窗口 mock 注册表（shell ensure/close 决策语义复刻 + 动作序列日志） ──

export interface WinAction {
  action: string;
  label: string;
  detail?: unknown;
}
export const windowLog: WinAction[] = [];

type WinListener = (ev: { payload: unknown }) => void;

class MockWindow {
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
  async emit(event: string, payload?: unknown) {
    emitLocal(event, payload);
  }
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
/** 当前窗口 label（每个测试文件一个“当前窗口”，默认 pet） */
let currentLabel = "pet";
export function setCurrentWindow(label: string) {
  currentLabel = label;
  if (!windowRegistry.has(label)) windowRegistry.set(label, new MockWindow(label));
}
export function getMockWindow(label: string) {
  return windowRegistry.get(label);
}

function emitLocal(event: string, payload: unknown, target?: string) {
  for (const [label, w] of windowRegistry) {
    if (target !== undefined && label !== target) continue;
    w.deliver(event, payload);
  }
}

// ── vi.mock 四个 Tauri API 模块 ──

vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: Record<string, unknown>) => {
    const base = (globalThis as Record<string, unknown>).__CORE_BASE__ as string;
    const http = async (method: string, path: string, body?: unknown) => {
      const r = await fetch(`${base}${path}`, {
        method,
        headers: { "Content-Type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      return r.json();
    };
    switch (cmd) {
      // core 共享逻辑命令 → HTTP（与生产 Tauri command 同一 core 逻辑）
      case "get_state": return http("GET", "/state");
      case "get_context": return http("GET", "/context");
      case "get_config": return http("GET", "/config");
      case "get_config_schema": return http("GET", "/config/schema");
      case "set_config": return http("POST", "/config", args);
      case "append_user": return http("POST", "/queue/user", args);
      case "push_event":
        return http("POST", "/events", {
          desc: args?.desc,
          card_id: args?.cardId,
          state: args?.stateSnapshot,
        });
      case "record_effect": return http("POST", "/effect", args);
      case "list_cards": return http("GET", "/cards");
      case "update_card_layout": return http("POST", "/cards/layout", args);
      case "set_card_user_closed": return http("POST", "/cards/user_closed", args);
      // shell 窗口决策 → mock 注册表（生产 Rust 逻辑由壳测试覆盖；此处测前端接线）
      case "ensure_card_window": {
        const id = String(args?.id ?? "");
        const label = `card-${id}`;
        const existing = windowRegistry.get(label);
        if (existing && !existing.destroyed) {
          existing.deliver("card:spec", args?.spec);
          await http("POST", "/effect", { kind: "event_emit", payload: { event: "card:spec", target: label } });
          return { result: "reused" };
        }
        const w = new MockWindow(label);
        windowRegistry.set(label, w);
        windowLog.push({ action: "create", label, detail: { title: label } });
        await http("POST", "/effect", { kind: "window_opened", payload: { window: label } });
        // 生产是 500ms 延迟推 spec；mock 立即推（listener 注册由测试保证时序）
        setTimeout(() => w.deliver("card:spec", args?.spec), 10);
        return { result: "opened" };
      }
      case "close_card_window": {
        const label = `card-${String(args?.id ?? "")}`;
        const w = windowRegistry.get(label);
        if (!w) return { result: "absent" };
        await w.destroy();
        await http("POST", "/effect", { kind: "window_closed", payload: { window: label } });
        return { result: "closed" };
      }
      case "toggle_pet":
      case "quit_app":
        return { ok: true };
      default:
        throw new Error(`shim 未覆盖 invoke: ${cmd}`);
    }
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: async (event: string, cb: WinListener) => {
    if (event === "effect") {
      // effect 下行总线 ← core WS（shim 启动时连接一次）
      effectListeners.push(cb);
      ensureWs();
      return () => {};
    }
    const w = windowRegistry.get(currentLabel);
    if (w) return w.listen(event, cb);
    return () => {};
  },
  emit: async (event: string, payload?: unknown) => {
    windowLog.push({ action: "emit", label: currentLabel, detail: { event } });
    emitLocal(event, payload);
    const base = (globalThis as Record<string, unknown>).__CORE_BASE__ as string;
    await fetch(`${base}/effect`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ kind: "event_emit", payload: { event } }),
    }).catch(() => {});
  },
  emitTo: async (target: string, event: string, payload?: unknown) => {
    windowLog.push({ action: "emit", label: currentLabel, detail: { event, target } });
    emitLocal(event, payload, target);
  },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowRegistry.get(currentLabel),
  availableMonitors: async () => [
    {
      position: { x: 0, y: 0 },
      size: { width: 1920, height: 1080 },
      workArea: { position: { x: 0, y: 0 }, size: { width: 1920, height: 1040 } },
      scaleFactor: 1,
    },
  ],
  currentMonitor: async () => ({
    position: { x: 0, y: 0 },
    size: { width: 1920, height: 1080 },
    scaleFactor: 1,
  }),
  PhysicalSize: class {
    constructor(
      public width: number,
      public height: number,
    ) {}
  },
  PhysicalPosition: class {
    constructor(
      public x: number,
      public y: number,
    ) {}
  },
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  WebviewWindow: class {
    static async getByLabel(label: string) {
      return windowRegistry.get(label) ?? null;
    }
    constructor(
      public label: string,
      _options?: unknown,
    ) {
      windowRegistry.set(label, new MockWindow(label));
    }
    once(_event: string, cb: () => void) {
      cb(); // mock：创建即成功
    }
  },
}));

// ── effect WS 订阅（shim 单例） ──

const effectListeners: WinListener[] = [];
let wsStarted = false;
function ensureWs() {
  if (wsStarted) return;
  wsStarted = true;
  const base = (globalThis as Record<string, unknown>).__CORE_BASE__ as string;
  const ws = new WebSocket(base.replace("http", "ws") + "/ws");
  ws.on("message", (data) => {
    const msg = JSON.parse(String(data)) as unknown;
    for (const cb of effectListeners) cb({ payload: msg });
  });
}

/** 初始化 shim 环境（每个测试文件 beforeAll 调用一次） */
export async function setupShim(core: CoreHandle) {
  (globalThis as Record<string, unknown>).__CORE_BASE__ = core.base;
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  setCurrentWindow("pet");
}

export function resetWindowWorld() {
  windowRegistry.clear();
  windowLog.length = 0;
  setCurrentWindow("pet");
}
