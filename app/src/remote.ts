// RemoteBridge：前端经 HTTP+WS loopback 连 overseer-core（docs/agent-loop.md §协议）。
// 浏览器调试模式与 Tauri 内嵌模式共用同一协议，前端代码不变（docs/harness.md 末节）。

import type {
  AppConfig,
  Bridge,
  ComponentSpec,
  Motion,
  ContextMessage,
  TopState,
} from "./bridge";

const BASE = "http://127.0.0.1:47600";
const WS_URL = "ws://127.0.0.1:47600/ws";

export class RemoteBridge implements Bridge {
  private topStateListeners: ((s: TopState) => void)[] = [];
  private renderListeners: ((spec: ComponentSpec) => void)[] = [];
  private queueListeners: ((m: ContextMessage[]) => void)[] = [];
  private autonomyListeners: ((args: {
    face?: string;
    motion?: Motion;
    ttlMs?: number;
  }) => void)[] = [];
  private configListeners: ((cfg: AppConfig) => void)[] = [];
  private deltaListeners: ((d: { content?: string; reasoning_content?: string }) => void)[] = [];
  private doneListeners: (() => void)[] = [];

  /** 探测 overseer-core debug server 是否在跑（决定用 Remote 还是 Mock） */
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

  connect() {
    this._connect_ws();
  }

  private _connect_ws() {
    const ws = new WebSocket(WS_URL);
    ws.addEventListener("open", () => {
      console.log("[remote] WS open");
    });
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data as string) as {
        kind: string;
        state?: TopState;
        spec?: ComponentSpec;
        face?: string;
        motion?: Motion;
        ttlMs?: number;
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
        case "context_changed":
          void this.getContext().then((m) =>
            this.queueListeners.forEach((cb) => cb(m)),
          );
          break;
        case "set_autonomy":
          this.autonomyListeners.forEach((cb) =>
            cb({ face: msg.face, motion: msg.motion, ttlMs: msg.ttlMs }),
          );
          break;
        case "config":
          if (msg.config) this.configListeners.forEach((cb) => cb(msg.config!));
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

  appendUserMessage(text: string): void {
    void fetch(`${BASE}/queue/user`, post({ text }));
  }

  pushEvent(desc: string, opts?: { cardId?: string; state?: unknown }): void {
    void fetch(`${BASE}/events`, post({ desc, card_id: opts?.cardId, state: opts?.state }));
  }

  onTopStateChanged(cb: (s: TopState) => void): void {
    this.topStateListeners.push(cb);
  }

  onRenderComponent(cb: (spec: ComponentSpec) => void): void {
    this.renderListeners.push(cb);
  }

  onContextChanged(cb: (m: ContextMessage[]) => void): void {
    this.queueListeners.push(cb);
  }

  onSetAutonomy(
    cb: (args: { face?: string; motion?: Motion; ttlMs?: number }) => void,
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
}

function post(body: unknown): RequestInit {
  return {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  };
}
