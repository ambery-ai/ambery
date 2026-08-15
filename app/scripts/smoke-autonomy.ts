// Autonomy 冒烟（node --experimental-transform-types scripts/smoke-autonomy.ts）：
// #27 语义——apply 源（derive/set_autonomy/revert）与「未变不 emit」去重。
// （autonomy.ts 用 parameter properties，需 transform-types 而非 strip-types）

import { Autonomy, type AutonomySource, type Expression } from "../src/autonomy.ts";
import type { Store } from "../src/store.ts";
import type { AppConfig, TopState } from "../src/bridge.ts";

// autonomy.ts 运行期使用 window.setTimeout（node 无 window → 最小 shim）
(globalThis as Record<string, unknown>).window = {
  setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
  clearTimeout: (id: number) => clearTimeout(id),
};

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
  kaomoji: {
    system: {
      idle: { face: "(´ω`)", motion: "still" },
      processing: { face: "(ˇωˇ」∠)_", motion: "float" },
    },
    user: {},
  },
  setAutonomyDefaultTtlMs: 5000,
  viewScale: 1,
};

function fakeStore(top: TopState) {
  const cbs: ((s: TopState) => void)[] = [];
  const store = {
    config: structuredClone(CFG),
    topState: structuredClone(top),
    onTopState: (cb: (s: TopState) => void) => cbs.push(cb),
  } as unknown as Store;
  return { store, emitTop: (s: TopState) => cbs.forEach((cb) => cb(s)) };
}

const IDLE: TopState = { instances: [], pendingNotifications: 0 };
const PROC: TopState = { instances: [{ id: "a", name: "x", status: "processing" }], pendingNotifications: 0 };

const seen: { e: Expression; source: AutonomySource }[] = [];
const f = fakeStore(IDLE);
const autonomy = new Autonomy(f.store, (e, source) => seen.push({ e, source }));

// 1. init → derive（idle 表情）
autonomy.init();
assert(seen.length === 1 && seen[0].source === "derive" && seen[0].e.face === "(´ω`)", "init 推导发出 derive/idle");

// 2. updateConfig（同表情）→ 去重不 emit
autonomy.updateConfig(structuredClone(CFG));
assert(seen.length === 1, "同表情重推导去重（不 emit）");

// 3. set_autonomy 覆盖 motion → emit set_autonomy
autonomy.setAutonomy({ motion: "bounce", ttlMs: 60000 });
assert(seen.length === 2 && seen[1].source === "set_autonomy" && seen[1].e.motion === "bounce", "覆盖 emit set_autonomy/bounce");

// 4. 同值覆盖 → 去重
autonomy.setAutonomy({ motion: "bounce", ttlMs: 60000 });
assert(seen.length === 2, "同值覆盖去重（不 emit）");

// 5. 覆盖期间 topState 变化不中断覆盖
f.emitTop(PROC);
assert(seen.length === 2, "覆盖期间 topState 变化不 emit");

// 6. TTL 到期 → revert 回落默认（此时 topState=processing → processing 表情）
autonomy.setAutonomy({ motion: "shake", ttlMs: 20 });
assert(seen.length === 3 && seen[2].e.motion === "shake", "shake 覆盖 emit");
await new Promise((r) => setTimeout(r, 50));
assert(seen.length === 4 && seen[3].source === "revert" && seen[3].e.face === "(ˇωˇ」∠)_", "TTL 回落 emit revert/processing");

// 7. topState 回 idle → derive
f.emitTop(IDLE);
assert(seen.length === 5 && seen[4].source === "derive" && seen[4].e.face === "(´ω`)", "topState 变化 emit derive");

// 8. 全空清除（无覆盖时同表情，去重）
autonomy.setAutonomy({});
assert(seen.length === 5, "无覆盖时全空清除同表情去重");

console.log(`\nautonomy 冒烟全过（${passed} 断言）`);
