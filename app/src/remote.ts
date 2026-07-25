// RemoteBridge：前端经 HTTP+WS loopback 连 overseer-core（docs/agent-loop.md §协议）。
// 浏览器调试模式与 Tauri 内嵌模式共用同一协议，前端代码不变（docs/harness.md 末节）。

import type {
  AppConfig,
  Bridge,
  ComponentSpec,
  Motion,
  QueueMessage,
  TopState,
} from "./bridge";

const BASE = "http://127.0.0.1:47601"; // WORKTREE
const WS_URL = "ws://127.0.0.1:47601/ws"; // WORKTREE

export class RemoteBridge implements Bridge {
  private topStateListeners: ((s: TopState) => void)[] = [];
  private renderListeners: ((spec: ComponentSpec) => void)[] = [];
  private queueListeners: ((m: QueueMessage[]) => void)[] = [];
  private autonomyListeners: ((args: {
    face?: string;
    motion?: Motion;
    ttlMs?: number;
  }) => void)[] = [];
  private configListeners: ((cfg: AppConfig) => void)[] = [];

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
    const ws = new WebSocket(WS_URL);
    // debug 模式不做重连：server 重启后刷新页面即可
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data as string) as {
        kind: string;
        state?: TopState;
        spec?: ComponentSpec;
        face?: string;
        motion?: Motion;
        ttlMs?: number;
        config?: AppConfig;
      };
      switch (msg.kind) {
        case "top_state":
          if (msg.state) this.topStateListeners.forEach((cb) => cb(msg.state!));
          break;
        case "render_component":
          if (msg.spec) this.renderListeners.forEach((cb) => cb(msg.spec!));
          break;
        case "queue_changed":
          void this.getQueue().then((m) =>
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
      }
    });
  }

  async getConfig(): Promise<AppConfig> {
    return (await fetch(`${BASE}/config`)).json() as Promise<AppConfig>;
  }

  async getTopState(): Promise<TopState> {
    return (await fetch(`${BASE}/state`)).json() as Promise<TopState>;
  }

  async getQueue(): Promise<QueueMessage[]> {
    return (await fetch(`${BASE}/queue`)).json() as Promise<QueueMessage[]>;
  }

  appendUserMessage(text: string): void {
    void fetch(`${BASE}/queue/user`, post({ text }));
  }

  pushEvent(desc: string): void {
    void fetch(`${BASE}/events`, post({ desc }));
  }

  onTopStateChanged(cb: (s: TopState) => void): void {
    this.topStateListeners.push(cb);
  }

  onRenderComponent(cb: (spec: ComponentSpec) => void): void {
    this.renderListeners.push(cb);
  }

  onQueueChanged(cb: (m: QueueMessage[]) => void): void {
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
}

function post(body: unknown): RequestInit {
  return {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  };
}
