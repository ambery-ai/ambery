// Autonomy（concepts §4，设计 docs/autonomy.md）：
// 默认行为由顶层状态规则推导（不经 LLM）；ペット可经 set_autonomy 覆盖，TTL 到期回落。

import type { AppConfig, Motion, TopState } from "./bridge";
import type { Store } from "./store";
import { motionDef } from "./motions.ts"; // 显式扩展名：node 冒烟可直接跑本模块链

export interface Expression {
  face: string;
  motion: Motion;
}

interface Override {
  face?: string;
  motion?: Motion;
}

/** 表情变化来源（#27 effect 语义显式）：set_autonomy 覆盖 / revert TTL 回落 / derive 默认推导 */
export type AutonomySource = "set_autonomy" | "revert" | "derive";

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

  /** 上次实际发出的表情（#27：未变不 emit——effect 流不再靠 window_resized 侧击推断） */
  private lastFace: string | null = null;
  private lastMotion: Motion | null = null;

  constructor(
    private store: Store,
    private onExpression: (e: Expression, source: AutonomySource) => void,
  ) {}

  /** 基线从 store 读（createStore 已完成拉取）；变化经 store 订阅 */
  init() {
    this.config = this.store.config;
    this.topState = this.store.topState ?? { instances: [], pendingNotifications: 0 };
    this.store.onTopState((s) => {
      this.topState = s;
      // 覆盖期间顶层状态变化不中断覆盖（docs/autonomy.md §set_autonomy）
      if (!this.override) this.apply("derive");
    });
    this.apply("derive");
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
    this.apply("derive");
  }

  /** set_autonomy tool 语义（docs/autonomy.md）：once=true 按 MotionDef.durationMs 取 TTL
   *  （动画 CSS 仍循环，TTL 到期回落默认，由此收束为一次性动作）；与 ttlMs 互斥（core 校验） */
  setAutonomy(args: { face?: string; motion?: Motion; ttlMs?: number; once?: boolean }) {
    // 全空 = 立即清除（once 不是覆盖字段：仅 once 单传视同全空，docs/autonomy.md）
    const isClear =
      (args.face === undefined && args.motion === undefined) || args.ttlMs === 0;
    if (isClear) {
      this.clearOverride();
      this.apply("set_autonomy");
      return;
    }
    this.override = { face: args.face, motion: args.motion };
    // 省略 ttlMs 的默认：config 下发值（默认 60000，docs/autonomy.md）；5000 仅为投影缺失兜底
    let ttl = args.ttlMs ?? this.config?.setAutonomyDefaultTtlMs ?? 60000;
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
      this.apply("revert");
    }, ttl);
    this.apply("set_autonomy");
  }

  private clearOverride() {
    this.override = null;
    if (this.overrideTimer !== null) {
      clearTimeout(this.overrideTimer);
      this.overrideTimer = null;
    }
  }

  private apply(source: AutonomySource) {
    const d = this.deriveDefault(this.topState);
    const e: Expression = this.override
      ? { face: this.override.face ?? d.face, motion: this.override.motion ?? d.motion }
      : d;
    // #27：表情未实际变化不 emit——调用处（尺寸重算 / expression_changed 上报）随之跳过
    if (e.face === this.lastFace && e.motion === this.lastMotion) return;
    this.lastFace = e.face;
    this.lastMotion = e.motion;
    this.onExpression(e, source);
  }
}
