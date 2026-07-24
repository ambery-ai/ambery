// Autonomy（concepts §4，设计 docs/autonomy.md）：
// 默认行为由顶层状态规则推导（不经 LLM）；ペット可经 set_autonomy 覆盖，TTL 到期回落。

import type { AppConfig, Bridge, Motion, TopState } from "./bridge";

export interface Expression {
  face: string;
  motion: Motion;
}

interface Override {
  face?: string;
  motion?: Motion;
}

const FALLBACK: Record<string, Expression> = {
  notify: { face: "✧*｡٩(ˊᗜˋ*)و✧*｡", motion: "bounce" },
  processing: { face: "(ˇωˇ」∠)_", motion: "float" },
  idle: { face: "(´ω`)", motion: "still" },
};

export class Autonomy {
  private topState: TopState = { instances: [], pendingNotifications: 0 };
  private config: AppConfig | null = null;
  private override: Override | null = null;
  private overrideTimer: number | null = null;

  constructor(
    private bridge: Bridge,
    private onExpression: (e: Expression) => void,
  ) {}

  async init() {
    this.config = await this.bridge.getConfig();
    this.topState = await this.bridge.getTopState();
    this.bridge.onTopStateChanged((s) => {
      this.topState = s;
      // 覆盖期间顶层状态变化不中断覆盖（docs/autonomy.md §set_autonomy）
      if (!this.override) this.apply();
    });
    this.apply();
  }

  /** 默认推导（优先级：notify > processing > idle） */
  deriveDefault(state: TopState): Expression {
    const key =
      state.pendingNotifications > 0
        ? "notify"
        : state.instances.some((i) => i.status === "processing")
          ? "processing"
          : "idle";
    return this.config?.kaomoji[key] ?? FALLBACK[key];
  }

  /** edit_config 推送后热更新映射表（RemoteBridge onConfigChanged） */
  updateConfig(config: AppConfig) {
    this.config = config;
    this.apply();
  }

  /** set_autonomy tool 语义（docs/autonomy.md） */
  setAutonomy(args: { face?: string; motion?: Motion; ttlMs?: number }) {
    const isClear =
      (args.face === undefined && args.motion === undefined) || args.ttlMs === 0;
    if (isClear) {
      this.clearOverride();
      this.apply();
      return;
    }
    this.override = { face: args.face, motion: args.motion };
    const ttl = args.ttlMs ?? this.config?.setAutonomyDefaultTtlMs ?? 5000;
    if (this.overrideTimer !== null) clearTimeout(this.overrideTimer);
    this.overrideTimer = window.setTimeout(() => {
      this.override = null;
      this.overrideTimer = null;
      this.apply();
    }, ttl);
    this.apply();
  }

  private clearOverride() {
    this.override = null;
    if (this.overrideTimer !== null) {
      clearTimeout(this.overrideTimer);
      this.overrideTimer = null;
    }
  }

  private apply() {
    const d = this.deriveDefault(this.topState);
    const e: Expression = this.override
      ? { face: this.override.face ?? d.face, motion: this.override.motion ?? d.motion }
      : d;
    this.onExpression(e);
  }
}
