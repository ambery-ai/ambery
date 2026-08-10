// RemoteBridge：前端经 HTTP+WS loopback 连 overseer-core（docs/agent-loop.md §协议）。
// 浏览器调试模式与 Tauri 内嵌模式共用同一协议，前端代码不变（docs/harness.md 末节）。

import type {
  AppConfig,
  Bridge,
  ComponentSpec,
  ConfigSchemaResp,
  Motion,
  ContextMessage,
  SetConfigResp,
  TopState,
  UserActionEvent,
} from "./bridge";

// 端口：默认 47600（生产/浏览器调试）；case-runner 拉起 TS 测试进程时经
// __OVERSEER_PORT__ 注入独立端口避让生产（docs/case-runner.md §壳类比；
// 该全局必须在导入本模块前设置——app/test/shim.ts 顶部）
const PORT =
  (globalThis as Record<string, unknown>).__OVERSEER_PORT__ ?? "47600";
const BASE = `http://127.0.0.1:${PORT}`;
const WS_URL = `ws://127.0.0.1:${PORT}/ws`;

export class RemoteBridge implements Bridge {
  private topStateListeners: ((s: TopState) => void)[] = [];
  private renderListeners: ((spec: ComponentSpec) => void)[] = [];
  private queueListeners: ((m: ContextMessage[]) => void)[] = [];
  private autonomyListeners: ((args: {
    face?: string;
    motion?: Motion;
    ttlMs?: number;
    once?: boolean;
  }) => void)[] = [];
  private configListeners: ((cfg: AppConfig) => void)[] = [];
  private closeListeners: ((id: string) => void)[] = [];
  private deltaListeners: ((d: { content?: string; reasoning_content?: string }) => void)[] = [];
  private doneListeners: (() => void)[] = [];

  /** 探测 debug server（overseer-case serve 完整 router）是否在跑（决定用 Remote 还是 Mock） */
  static async probe(timeoutMs = 800): Promise<boolean> {
    try {
      const r = await fetch(`${BASE}/state`, {
        signal: AbortSignal.timeout(timeoutMs),
      });
      return r.ok;
    } catch {
      return false;
    }
  }

  /** 首个 WS open 即 resolve（createBridge 等待它再返回，调用方拿到即可收发的桥） */
  private readyResolve!: () => void;
  readonly ready: Promise<void> = new Promise((r) => (this.readyResolve = r));

  connect() {
    this._connect_ws();
  }

  private _connect_ws() {
    const ws = new WebSocket(WS_URL);
    ws.addEventListener("open", () => {
      console.log("[remote] WS open");
      this.readyResolve();
    });
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data as string) as {
        kind: string;
        state?: TopState;
        spec?: ComponentSpec;
        id?: string;
        face?: string;
        motion?: Motion;
        ttlMs?: number;
        once?: boolean;
        config?: AppConfig;
        content?: string;
        reasoning_content?: string;
      };
      switch (msg.kind) {
        case "top_state":
          if (msg.state) this.topStateListeners.forEach((cb) => cb(msg.state!));
          break;
        case "render_component":
          if (msg.spec) this.renderListeners.forEach((cb) => cb(msg.spec!));
          break;
        case "close_component":
          // 持续管理协议显式关闭（docs/components.md；与 TauriBridge 同一语义）
          if (msg.id) this.closeListeners.forEach((cb) => cb(msg.id!));
          break;
        case "context_changed":
          void this.getContext().then((m) =>
            this.queueListeners.forEach((cb) => cb(m)),
          );
          break;
        case "set_autonomy":
          this.autonomyListeners.forEach((cb) =>
            cb({ face: msg.face, motion: msg.motion, ttlMs: msg.ttlMs, once: msg.once }),
          );
          break;
        case "config":
          // config effect 是裸信号（无载荷）——按需重拉（与 TauriBridge 同一刷新语义，
          // docs/case-runner.md §前端读取架构：事件提示时按需重拉）
          void this.getConfig().then((cfg) => this.configListeners.forEach((cb) => cb(cfg)));
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
    ws.addEventListener("error", (e) => {
      console.warn("[remote] WS error, will retry in 2s", e);
    });
    ws.addEventListener("close", () => {
      console.warn("[remote] WS closed, reconnecting in 2s…");
      setTimeout(() => this._connect_ws(), 2000);
    });
  }

  async getConfig(): Promise<AppConfig> {
    return (await fetch(`${BASE}/config`)).json() as Promise<AppConfig>;
  }

  async getTopState(): Promise<TopState> {
    return (await fetch(`${BASE}/state`)).json() as Promise<TopState>;
  }

  async getContext(): Promise<ContextMessage[]> {
    return (await fetch(`${BASE}/context`)).json() as Promise<ContextMessage[]>;
  }

  async appendUserMessage(text: string): Promise<boolean> {
    try {
      const r = await fetch(`${BASE}/queue/user`, post({ text }));
      return r.ok;
    } catch {
      return false;
    }
  }

  pushEvent(ev: UserActionEvent): void {
    void fetch(`${BASE}/events`, post({
      action: ev.action,
      card_id: ev.cardId,
      card_type: ev.cardType,
      title: ev.title,
      text: ev.text,
      target: ev.target,
      checked: ev.checked,
      state: ev.state,
    }));
  }

  onTopStateChanged(cb: (s: TopState) => void): void {
    this.topStateListeners.push(cb);
  }

  onRenderComponent(cb: (spec: ComponentSpec) => void): void {
    this.renderListeners.push(cb);
  }

  onCloseComponent(cb: (id: string) => void): void {
    this.closeListeners.push(cb);
  }

  onContextChanged(cb: (m: ContextMessage[]) => void): void {
    this.queueListeners.push(cb);
  }

  onSetAutonomy(
    cb: (args: { face?: string; motion?: Motion; ttlMs?: number; once?: boolean }) => void,
  ): void {
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

  // 设置面板（docs/config.md §统一修改入口 Server API；debug 全量 router 才有这两端点）
  async getConfigSchema(): Promise<ConfigSchemaResp> {
    return (await fetch(`${BASE}/config/schema`)).json() as Promise<ConfigSchemaResp>;
  }

  async setConfig(path: string, value: unknown): Promise<SetConfigResp> {
    return (await fetch(`${BASE}/config`, post({ path, value }))).json() as Promise<SetConfigResp>;
  }
}

function post(body: unknown): RequestInit {
  return {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  };
}
