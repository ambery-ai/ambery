// Store 冒烟（node --experimental-strip-types scripts/smoke-store.ts）：
// 假 Bridge 验证基线拉取、变化订阅、cards 重拉、可选能力缺席与失败兜底。
// 本文件与 src/store.ts 一样保持 strip-types 可执行（无 enum / parameter properties）。

import { Store } from "../src/store.ts";
import type {
  AppConfig,
  Bridge,
  ContextMessage,
  RestoredCard,
  TopState,
} from "../src/bridge.ts";

let passed = 0;
function assert(cond: boolean, msg: string) {
  if (!cond) {
    console.error(`FAIL: ${msg}`);
    process.exit(1);
  }
  passed++;
  console.log(`ok ${passed} - ${msg}`);
}

const CFG: AppConfig = {
  kaomoji: { system: { idle: { face: "(´ω`)", motion: "still" } }, user: {} },
  setAutonomyDefaultTtlMs: 5000,
  viewScale: 1,
};
const TOP: TopState = { instances: [{ id: "a", name: "x", status: "idle" }], pendingNotifications: 0 };
const CTX: ContextMessage[] = [{ role: "user", content: "hi", ts: 1 }];
const CARDS: RestoredCard[] = [
  {
    component: { id: "c1", type: "text_card", title: "t", text: "x" },
    user_closed: false,
    layout: { direction: null, offset: null, manual: false },
  },
];

/** 最小假 Bridge：只实现 store 消费的读与事件面；写方法不需要 */
function fakeBridge(opts: { failConfig?: boolean; noCards?: boolean } = {}) {
  const cbs = {
    config: [] as ((c: AppConfig) => void)[],
    top: [] as ((s: TopState) => void)[],
    ctx: [] as ((m: ContextMessage[]) => void)[],
    cards: [] as (() => void)[],
  };
  let cards = CARDS;
  const bridge = {
    getConfig: () =>
      opts.failConfig ? Promise.reject(new Error("not ready")) : Promise.resolve(structuredClone(CFG)),
    getTopState: () => Promise.resolve(structuredClone(TOP)),
    getContext: () => Promise.resolve(structuredClone(CTX)),
    onTopStateChanged: (cb: (s: TopState) => void) => cbs.top.push(cb),
    onContextChanged: (cb: (m: ContextMessage[]) => void) => cbs.ctx.push(cb),
    onConfigChanged: (cb: (c: AppConfig) => void) => cbs.config.push(cb),
    onRenderComponent: () => {},
    pushEvent: () => {},
    appendUserMessage: () => {},
    ...(opts.noCards
      ? {}
      : {
          listCards: () => Promise.resolve(structuredClone(cards)),
          onCardsChanged: (cb: () => void) => cbs.cards.push(cb),
        }),
  } as unknown as Bridge;
  return {
    bridge,
    emitTop: (s: TopState) => cbs.top.forEach((cb) => cb(s)),
    emitCtx: (m: ContextMessage[]) => cbs.ctx.forEach((cb) => cb(m)),
    emitConfig: (c: AppConfig) => cbs.config.forEach((cb) => cb(c)),
    emitCards: (next: RestoredCard[]) => {
      cards = next;
      cbs.cards.forEach((cb) => cb());
    },
  };
}

// 1. 基线拉取
const f1 = fakeBridge();
const s1 = await Store.create(f1.bridge);
assert(s1.config?.viewScale === 1, "config 基线可读");
assert(s1.topState?.instances.length === 1, "top_state 基线可读");
assert(s1.context?.length === 1, "context 基线可读");
assert(s1.cards?.length === 1, "cards 基线可读");

// 2. 变化订阅（事件载荷直写）
let got: TopState | null = null;
s1.onTopState((s) => (got = s));
f1.emitTop({ instances: [], pendingNotifications: 2 });
assert(got !== null && (got as TopState).pendingNotifications === 2, "top_state 变化触发订阅");
assert(s1.topState?.pendingNotifications === 2, "top_state getter 同步最新");

let gotCtx: ContextMessage[] | null = null;
s1.onContext((m) => (gotCtx = m));
f1.emitCtx([...CTX, { role: "assistant", content: "yo", ts: 2 }]);
assert(gotCtx !== null && (gotCtx as ContextMessage[]).length === 2, "context 变化触发订阅");

let gotCfg: AppConfig | null = null;
s1.onConfig((c) => (gotCfg = c));
f1.emitConfig({ ...CFG, viewScale: 2 });
assert(gotCfg !== null && (gotCfg as AppConfig).viewScale === 2, "config 变化触发订阅");

// 3. cards 裸信号 → 重拉注册表
let gotCards: RestoredCard[] | null = null;
s1.onCards((c) => (gotCards = c));
f1.emitCards([]);
await new Promise((r) => setTimeout(r, 10));
assert(gotCards !== null && (gotCards as RestoredCard[]).length === 0, "cardsChanged 信号触发重拉并通知");
assert(s1.cards?.length === 0, "cards getter 同步最新");

// 4. 手动 refreshCards（写动作后的确定性刷新）
await s1.refreshCards();
assert(s1.cards !== null, "refreshCards 后可读");

// 5. 单类失败不拖垮其余（wait_state 竞态期）
const f2 = fakeBridge({ failConfig: true });
const s2 = await Store.create(f2.bridge);
assert(s2.config === null, "config 拉取失败 → null 兜底不抛");
assert(s2.topState?.instances.length === 1, "其余类基线不受失败影响");

// 6. 可选能力缺席（无 listCards/onCardsChanged 的桥）不炸
const f3 = fakeBridge({ noCards: true });
const s3 = await Store.create(f3.bridge);
assert(Array.isArray(s3.cards) && s3.cards.length === 0, "无 listCards → cards 空数组兜底");
await s3.refreshCards();
assert(s3.cards?.length === 0, "无 listCards 时 refreshCards 为空操作");

console.log(`\nstore 冒烟全过（${passed} 断言）`);
