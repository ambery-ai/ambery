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

/** docs/components.md：Component 方位 */
export type Direction =
  | "top"
  | "bottom"
  | "left"
  | "right"
  | "top-left"
  | "top-right"
  | "bottom-left"
  | "bottom-right"
  | "auto";

/** docs/components.md：call_component 协议（判别联合） */
export type ComponentSpec =
  | { id: string; type: "text_card"; direction?: Direction; title: string; text: string }
  | { id: string; type: "quick_jump"; direction?: Direction; label: string; target: string }
  | {
      id: string;
      type: "git_display";
      direction?: Direction;
      title: string;
      entries: { hash: string; msg: string; time: string }[];
      diff?: string;
    }
  | {
      id: string;
      type: "data_chart";
      direction?: Direction;
      title: string;
      chart: {
        kind: "line" | "bar" | "pie";
        labels: string[];
        series: { name: string; data: number[] }[];
      };
    }
  | {
      id: string;
      type: "todobox";
      direction?: Direction;
      title: string;
      items: { text: string; done: boolean }[];
    };

/** docs/chat-panel.md：Queue 消息（concepts §10c 四 role） */
export interface QueueMessage {
  role: "user" | "assistant" | "tool" | "system";
  content: string;
  ts: number;
}

export interface Bridge {
  getConfig(): Promise<AppConfig>;
  getTopState(): Promise<TopState>;
  onTopStateChanged(cb: (s: TopState) => void): void;
  /** Overseer → UI：渲染 Component（ペット call_component 的执行结果） */
  onRenderComponent(cb: (spec: ComponentSpec) => void): void;
  /** UI → Harness：Component 交互事件写入 Event Buffer（concepts §10e） */
  pushEvent(desc: string): void;
  /** Queue：对话历史读取 + 用户输入写入 user role（concepts §3a） */
  getQueue(): Promise<QueueMessage[]>;
  appendUserMessage(text: string): void;
  onQueueChanged(cb: (msgs: QueueMessage[]) => void): void;
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
  /** 模拟ペット的 call_component tool call */
  callComponent(spec: ComponentSpec): void;
  /** 读取 Event Buffer 当前内容（不写 Queue user role，concepts §10e） */
  eventBuffer(): string[];
  /** 模拟 LLM 触发时合并注入后清空 Buffer */
  flushEventBuffer(): string[];
  /** 模拟ペット回复 / Overseer 注入 system 消息（真实链路由 Rust Harness 写入） */
  appendMessage(role: QueueMessage["role"], content: string): void;
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
  private renderListeners: ((spec: ComponentSpec) => void)[] = [];
  private queueListeners: ((msgs: QueueMessage[]) => void)[] = [];
  private events: string[] = [];
  private queue: QueueMessage[] = [];

  async getConfig(): Promise<AppConfig> {
    return structuredClone(DEFAULT_CONFIG);
  }

  async getTopState(): Promise<TopState> {
    return structuredClone(this.state);
  }

  onTopStateChanged(cb: (s: TopState) => void): void {
    this.listeners.push(cb);
  }

  onRenderComponent(cb: (spec: ComponentSpec) => void): void {
    this.renderListeners.push(cb);
  }

  pushEvent(desc: string): void {
    this.events.push(desc);
  }

  debugCallComponent(spec: ComponentSpec) {
    for (const cb of this.renderListeners) cb(structuredClone(spec));
  }

  debugEventBuffer(): string[] {
    return [...this.events];
  }

  debugFlushEventBuffer(): string[] {
    const out = [...this.events];
    this.events = [];
    return out;
  }

  async getQueue(): Promise<QueueMessage[]> {
    return structuredClone(this.queue);
  }

  appendUserMessage(text: string): void {
    this.queue.push({ role: "user", content: text, ts: Date.now() });
    this.emitQueue();
  }

  onQueueChanged(cb: (msgs: QueueMessage[]) => void): void {
    this.queueListeners.push(cb);
  }

  debugAppendMessage(role: QueueMessage["role"], content: string) {
    this.queue.push({ role, content, ts: Date.now() });
    this.emitQueue();
  }

  private emitQueue() {
    const snapshot = structuredClone(this.queue);
    for (const cb of this.queueListeners) cb(snapshot);
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
