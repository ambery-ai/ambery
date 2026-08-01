// Autonomy（concepts §4，设计 docs/autonomy.md）：
// 默认行为由顶层状态规则推导（不经 LLM）；ペット可经 set_autonomy 覆盖，TTL 到期回落。

import type { AppConfig, Bridge, Motion, TopState } from "./bridge";
import { motionDef } from "./motions";

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

  /** 默认推导（优先级：notify > processing > idle）；key 在两池并集解析（docs/autonomy.md） */
  deriveDefault(state: TopState): Expression {
    const key =
      state.pendingNotifications > 0
        ? "notify"
        : state.instances.some((i) => i.status === "processing")
          ? "processing"
          : "idle";
    // 校验保证两池不相交，顺序无歧义；约定 system 先查（与 core kaomoji_resolve 一致）。
    // key 不再存在时回落内置默认（docs/autonomy.md：当前状态 key 消失 → 回落默认状态）
    return (
      this.config?.kaomoji.system[key] ??
      this.config?.kaomoji.user[key] ??
      FALLBACK[key]
    );
  }

  /** edit_config 推送后热更新映射表（RemoteBridge onConfigChanged） */
  updateConfig(config: AppConfig) {
    this.config = config;
    this.apply();
  }

  /** set_autonomy tool 语义（docs/autonomy.md）：once=true 按 MotionDef.durationMs 取 TTL
   *  （动画 CSS 仍循环，TTL 到期回落默认，由此收束为一次性动作）；与 ttlMs 互斥（core 校验） */
  setAutonomy(args: { face?: string; motion?: Motion; ttlMs?: number; once?: boolean }) {
    const isClear =
      (args.face === undefined && args.motion === undefined && !args.once) || args.ttlMs === 0;
    if (isClear) {
      this.clearOverride();
      this.apply();
      return;
    }
    this.override = { face: args.face, motion: args.motion };
    let ttl = args.ttlMs ?? this.config?.setAutonomyDefaultTtlMs ?? 5000;
    if (args.once) {
      // 一次播放：持续时间为「生效 motion」的注册表时长（未传 motion = 默认推导 motion）；
      // still 无 durationMs → 0（立即回落）
      const effectiveMotion = args.motion ?? this.deriveDefault(this.topState).motion;
      ttl = motionDef(effectiveMotion).durationMs ?? 0;
    }
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
