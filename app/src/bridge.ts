// Bridge：前端与 Overseer 内核之间的唯一边界。
// Tauri 模式走 IPC（后续 Tauri 壳迭代接入），浏览器测试模式走内存 mock，
// 使全部显示逻辑可在 Chrome DevTools 中直接驱动验证。

// ── 领域模型（与 concepts.md 对齐） ──

/** concepts §9a Status 状态机 */
export type CodeCliStatus = "idle" | "processing" | "unknown";

export interface CodeCliInstance {
  id: string;
  name: string;
  status: CodeCliStatus;
}

/** concepts §4：Autonomy 顶层状态 = Context 中 Code CLI Status 一览 + 未决通知数 */
export interface TopState {
  instances: CodeCliInstance[];
  pendingNotifications: number;
}

/** docs/autonomy.md：Motion 枚举 */
export type Motion = "still" | "float" | "bounce" | "shake";

export interface KaomojiEntry {
  face: string;
  motion: Motion;
}

/** concepts §12 Config（当前仅前端所需子集） */
export interface AppConfig {
  /** 状态 key → 表达式 映射，edit_config 可增改 */
  kaomoji: Record<string, KaomojiEntry>;
  /** set_autonomy 省略 ttlMs 时的默认值 */
  setAutonomyDefaultTtlMs: number;
}

export interface Bridge {
  getConfig(): Promise<AppConfig>;
  getTopState(): Promise<TopState>;
  onTopStateChanged(cb: (s: TopState) => void): void;
}

// ── Chrome DevTools 调试驱动接口（window.__overseer） ──

export interface DebugApi {
  setInstanceStatus(name: string, status: CodeCliStatus): void;
  addInstance(name: string, status: CodeCliStatus): void;
  notify(n?: number): void;
  clearNotifications(): void;
  setAutonomy(args: { face?: string; motion?: Motion; ttlMs?: number }): void;
  viewState(): {
    docked: boolean;
    center: { x: number; y: number };
    face: string | null;
    motion: string;
  };
}

declare global {
  interface Window {
    __overseer?: DebugApi;
  }
}

// ── 浏览器 mock ──

const DEFAULT_CONFIG: AppConfig = {
  kaomoji: {
    idle: { face: "(´ω`)", motion: "still" },
    processing: { face: "(ˇωˇ」∠)_", motion: "float" },
    notify: { face: "✧*｡٩(ˊᗜˋ*)و✧*｡", motion: "bounce" },
  },
  setAutonomyDefaultTtlMs: 5000,
};

export class BrowserMockBridge implements Bridge {
  private state: TopState = {
    instances: [
      { id: "mock-1", name: "mock-a", status: "processing" },
      { id: "mock-2", name: "mock-b", status: "idle" },
    ],
    pendingNotifications: 0,
  };
  private listeners: ((s: TopState) => void)[] = [];

  async getConfig(): Promise<AppConfig> {
    return structuredClone(DEFAULT_CONFIG);
  }

  async getTopState(): Promise<TopState> {
    return structuredClone(this.state);
  }

  onTopStateChanged(cb: (s: TopState) => void): void {
    this.listeners.push(cb);
  }

  private emit() {
    const snapshot = structuredClone(this.state);
    for (const cb of this.listeners) cb(snapshot);
  }

  /** 模拟 Hook 触发后 Overseer 更新的实例状态（真实链路后续由 Rust 内核喂入） */
  debugSetInstanceStatus(name: string, status: CodeCliStatus) {
    const inst = this.state.instances.find((i) => i.name === name);
    if (inst) inst.status = status;
    this.emit();
  }

  debugAddInstance(name: string, status: CodeCliStatus) {
    this.state.instances.push({ id: `mock-${Date.now()}`, name, status });
    this.emit();
  }

  debugNotify(n = 1) {
    this.state.pendingNotifications += n;
    this.emit();
  }

  debugClearNotifications() {
    this.state.pendingNotifications = 0;
    this.emit();
  }
}

export function createBridge(): Bridge {
  // Tauri 壳迭代时在此分流：'__TAURI__' in window → TauriBridge
  return new BrowserMockBridge();
}
