// Store：core 拥有的可读状态在前端集中持有。
// 边界判据：core 拥有 + 多窗口/组件读 + 变化驱动 UI + 体积可控
//   → 收敛四类：config / top_state / context / cards；
//   前端局部瞬态（面板开关、输入框、拖拽中位置）与写意图（invoke 写指令）不进 store。
// 读取一律走 store（getter 取基线 + onX 订阅变化）；写入一律走 bridge 写方法 / 动作层。
// invoke 只允许出现在两个收口：store 的刷新（bridge 读方法）+ 动作层的写指令。
//
// 本文件须保持 node --experimental-strip-types 可执行（scripts/smoke-store.ts 直接跑）：
// 不用 parameter properties / enum 等需变换的 TS 语法。

import type { AppConfig, Bridge, ContextMessage, RestoredCard, TopState } from "./bridge";

export class Store {
  private bridge: Bridge;

  private configData: AppConfig | null = null;
  private topStateData: TopState | null = null;
  private contextData: ContextMessage[] | null = null;
  private cardsData: RestoredCard[] | null = null;

  private configListeners = new Set<(c: AppConfig) => void>();
  private topStateListeners = new Set<(s: TopState) => void>();
  private contextListeners = new Set<(m: ContextMessage[]) => void>();
  private cardsListeners = new Set<(c: RestoredCard[]) => void>();

  private constructor(bridge: Bridge) {
    this.bridge = bridge;
  }

  /** 基线拉取（各类独立兜底：单类失败不拖垮其余，保 null 待事件修复）+ 事件接线 */
  static async create(bridge: Bridge): Promise<Store> {
    const s = new Store(bridge);
    const pull = <T>(tag: string, p: Promise<T>): Promise<T | null> =>
      p.catch((e) => {
        console.error(`[store] ${tag} 基线拉取失败`, e);
        return null;
      });
    const [config, topState, context, cards] = await Promise.all([
      pull("config", bridge.getConfig()),
      pull("top_state", bridge.getTopState()),
      pull("context", bridge.getContext()),
      pull("cards", bridge.listCards?.() ?? Promise.resolve<RestoredCard[]>([])),
    ]);
    s.configData = config;
    s.topStateData = topState;
    s.contextData = context;
    s.cardsData = cards;

    // 事件提示 → 按需重拉/直写。事件载荷自带新值（config/top_state/context）的直写；
    // cards 是裸信号（render/close effect），重拉注册表
    bridge.onConfigChanged?.((c) => {
      s.configData = c;
      for (const cb of s.configListeners) cb(c);
    });
    bridge.onTopStateChanged((t) => {
      s.topStateData = t;
      for (const cb of s.topStateListeners) cb(t);
    });
    bridge.onContextChanged((m) => {
      s.contextData = m;
      for (const cb of s.contextListeners) cb(m);
    });
    bridge.onCardsChanged?.(() => {
      void s.refreshCards();
    });
    return s;
  }

  get config(): AppConfig | null {
    return this.configData;
  }
  get topState(): TopState | null {
    return this.topStateData;
  }
  get context(): ContextMessage[] | null {
    return this.contextData;
  }
  get cards(): RestoredCard[] | null {
    return this.cardsData;
  }

  /** 仅变化时触发；基线在 create() 已完成，初值用对应 getter 取 */
  onConfig(cb: (c: AppConfig) => void): void {
    this.configListeners.add(cb);
  }
  onTopState(cb: (s: TopState) => void): void {
    this.topStateListeners.add(cb);
  }
  onContext(cb: (m: ContextMessage[]) => void): void {
    this.contextListeners.add(cb);
  }
  onCards(cb: (c: RestoredCard[]) => void): void {
    this.cardsListeners.add(cb);
  }

  /** cards 重拉（写动作后的确定性刷新；事件驱动刷新也走这里） */
  async refreshCards(): Promise<void> {
    if (!this.bridge.listCards) return;
    const cards = await this.bridge
      .listCards()
      .catch((e) => {
        console.error("[store] cards 重拉失败", e);
        return null;
      });
    if (cards === null) return;
    this.cardsData = cards;
    for (const cb of this.cardsListeners) cb(cards);
  }
}
