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
  /** View 缩放（concepts §3，默认 0.5） */
  viewScale: number;
  /** 未读角标样式（#5）：number 纯数字（默认）/ bubble 气泡 */
  badgeStyle?: "number" | "bubble";
  /** 未读角标方位（#5）：right（默认）/ left */
  badgeSide?: "right" | "left";
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
export interface ContextMessage {
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
  getContext(): Promise<ContextMessage[]>;
  appendUserMessage(text: string): void;
  onContextChanged(cb: (msgs: ContextMessage[]) => void): void;
  /** 可选（RemoteBridge）：Overseer 推送 set_autonomy（ペット的 tool call 结果） */
  onSetAutonomy?(
    cb: (args: { face?: string; motion?: Motion; ttlMs?: number }) => void,
  ): void;
  /** 可选（RemoteBridge）：Overseer 推送 Config 变更（edit_config 的结果） */
  onConfigChanged?(cb: (cfg: AppConfig) => void): void;
  /** 可选：流式增量（docs/streaming.md）——assistant 回复片段，纯显示优化 */
  onAssistantDelta?(cb: (d: { content?: string; reasoning_content?: string }) => void): void;
  /** 可选：一轮回复完毕（loading 收尾，完整回复已写 Context） */
  onAssistantDone?(cb: () => void): void;
  /** 可选：显式关闭卡片（Component 持续管理协议：action="close"） */
  onCloseComponent?(cb: (id: string) => void): void;
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
  appendMessage(role: ContextMessage["role"], content: string): void;
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
  viewScale: 1,
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
  private queueListeners: ((msgs: ContextMessage[]) => void)[] = [];
  private events: string[] = [];
  private queue: ContextMessage[] = [];

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

  async getContext(): Promise<ContextMessage[]> {
    return structuredClone(this.queue);
  }

  appendUserMessage(text: string): void {
    this.queue.push({ role: "user", content: text, ts: Date.now() });
    this.emitQueue();
  }

  onContextChanged(cb: (msgs: ContextMessage[]) => void): void {
    this.queueListeners.push(cb);
  }

  debugAppendMessage(role: ContextMessage["role"], content: string) {
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

/** Tauri 原生 IPC bridge（invoke + listen，docs/core-server.md） */
class TauriBridge implements Bridge {
  private renderListeners: ((spec: ComponentSpec) => void)[] = [];
  private contextListeners: ((m: ContextMessage[]) => void)[] = [];
  private topStateListeners: ((s: TopState) => void)[] = [];
  private autonomyListeners: ((args: { face?: string; motion?: Motion; ttlMs?: number }) => void)[] = [];
  private configListeners: ((cfg: AppConfig) => void)[] = [];
  private deltaListeners: ((d: { content?: string; reasoning_content?: string }) => void)[] = [];
  private doneListeners: (() => void)[] = [];
  private closeListeners: ((id: string) => void)[] = [];

  constructor(
    private invokeFn: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>,
    listenFn: (event: string, cb: (ev: { payload: unknown }) => void) => Promise<unknown>,
  ) {
    void listenFn("effect", (ev) => {
      const msg = ev.payload as {
        kind?: string;
        spec?: ComponentSpec;
        face?: string;
        motion?: Motion;
        ttlMs?: number;
        state?: TopState;
        content?: string;
        reasoning_content?: string;
        id?: string;
      };
      if (!msg?.kind) return;
      switch (msg.kind) {
        case "render_component":
          if (msg.spec) this.renderListeners.forEach((cb) => cb(msg.spec!));
          break;
        case "close_component":
          if (msg.id) this.closeListeners.forEach((cb) => cb(msg.id!));
          break;
        case "set_autonomy":
          this.autonomyListeners.forEach((cb) =>
            cb({ face: msg.face, motion: msg.motion, ttlMs: msg.ttlMs }),
          );
          break;
        case "config":
          void this.getConfig().then((cfg) => this.configListeners.forEach((cb) => cb(cfg)));
          break;
        case "context_changed":
          void this.getContext().then((m) => this.contextListeners.forEach((cb) => cb(m)));
          break;
        case "top_state":
          if (msg.state) this.topStateListeners.forEach((cb) => cb(msg.state!));
          break;
        case "assistant_delta":
          this.deltaListeners.forEach((cb) =>
            cb({ content: msg.content, reasoning_content: msg.reasoning_content }),
          );
          break;
        case "assistant_done":
          this.doneListeners.forEach((cb) => cb());
          break;
      }
    });
  }

  async getConfig(): Promise<AppConfig> {
    return this.invokeFn("get_config") as Promise<AppConfig>;
  }
  async getTopState(): Promise<TopState> {
    // wait_state 竞态期（run_core 未注入）保底：不炸前端，返回空态
    return (this.invokeFn("get_state") as Promise<TopState>).catch((e) => {
      console.error("[bridge] get_state", e);
      return { instances: [], pendingNotifications: 0 };
    });
  }
  async getContext(): Promise<ContextMessage[]> {
    return (this.invokeFn("get_context") as Promise<ContextMessage[]>).catch((e) => {
      console.error("[bridge] get_context", e);
      return [];
    });
  }
  appendUserMessage(text: string): void {
    void this.invokeFn("append_user", { text }).catch((e) => console.error("[bridge] append_user", e));
  }
  pushEvent(desc: string): void {
    void this.invokeFn("push_event", { desc }).catch((e) => console.error("[bridge] push_event", e));
  }
  onRenderComponent(cb: (spec: ComponentSpec) => void): void {
    this.renderListeners.push(cb);
  }
  onContextChanged(cb: (m: ContextMessage[]) => void): void {
    this.contextListeners.push(cb);
  }
  onTopStateChanged(cb: (s: TopState) => void): void {
    this.topStateListeners.push(cb);
  }
  onSetAutonomy(cb: (args: { face?: string; motion?: Motion; ttlMs?: number }) => void): void {
    this.autonomyListeners.push(cb);
  }
  onConfigChanged(cb: (cfg: AppConfig) => void): void {
    this.configListeners.push(cb);
  }
  onAssistantDelta(cb: (d: { content?: string; reasoning_content?: string }) => void): void {
    this.deltaListeners.push(cb);
  }
  onAssistantDone(cb: () => void): void {
    this.doneListeners.push(cb);
  }
  onCloseComponent(cb: (id: string) => void): void {
    this.closeListeners.push(cb);
  }
}

/** Tauri 模式 → TauriBridge（原生 IPC）；浏览器 → overseer-core 在跑用 RemoteBridge，否则内存 mock */
export async function createBridge(): Promise<Bridge> {
  if ("__TAURI_INTERNALS__" in window) {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");
    return new TauriBridge(invoke, listen);
  }
  const { RemoteBridge } = await import("./remote");
  if (await RemoteBridge.probe()) {
    const b = new RemoteBridge();
    b.connect();
    return b;
  }
  return new BrowserMockBridge();
}
