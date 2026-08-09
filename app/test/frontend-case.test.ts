// 前端进 case v2 首套（docs/case-runner.md §前端进 case）：
// 真前端模块（store / bridge TauriBridge / pet main / ChatPanel / ComponentManager）
// × 真 core（overseer-debug 沙盒子进程）× shim Tauri IPC × mock 窗口层。
// 断言对象：store 基线与回读、Queue 放行回读、#25 窗口接线、#26 统一关闭、
// windowed ComponentManager 不订全局流。

import { beforeAll, afterAll, expect, it, vi } from "vitest";
import {
  startCore,
  stopCore,
  setupShim,
  readEffects,
  windowLog,
  getMockWindow,
  type CoreHandle,
} from "./shim";
import { createBridge } from "../src/bridge";
import { Store } from "../src/store";
import { ChatPanel } from "../src/windows/chat";
import { ComponentManager } from "../src/components/component-manager";
import type { PositioningEngine } from "../src/positioning/engine";

let core: CoreHandle;

async function poll<T>(fn: () => T | null | undefined | false, what: string, ms = 8000): Promise<T> {
  const t0 = Date.now();
  for (;;) {
    const v = fn();
    if (v) return v;
    if (Date.now() - t0 > ms) throw new Error(`poll 超时: ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

beforeAll(async () => {
  core = await startCore();
  await setupShim(core);
  document.body.innerHTML = '<div id="app"></div>';
}, 60000);

afterAll(() => {
  stopCore(core);
});

it("T1 store 基线：config/top_state/context/cards 从真 core 拉取", async () => {
  const bridge = await createBridge(); // shim 在场 → 真生产桥 TauriBridge
  const store = await Store.create(bridge);
  expect(store.config?.viewScale).toBeTypeOf("number");
  expect(store.config?.kaomoji.system.idle).toBeTruthy();
  expect(store.topState?.instances).toEqual([]);
  expect(store.context).toEqual([]);
  expect(store.cards).toEqual([]);
});

it("T2 用户消息回读：appendUserMessage → Queue 放行 → context_changed → store", async () => {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  bridge.appendUserMessage("vitest 回读验证");
  await poll(
    () => store.context?.some((m) => m.role === "user" && m.content === "vitest 回读验证"),
    "context 含用户消息",
  );
});

it("T3 #25 接线：pet render/close 经 ensure/close 决策，一窗一卡可重建", async () => {
  const { main: petMain } = await import("../src/windows/pet");
  await petMain(); // pet 窗口主流程在 jsdom 全量启动
  const base = core.base;
  const spec = { id: "t1", type: "text_card", title: "T", text: "hello" };
  const emitEffect = (msg: unknown) =>
    fetch(`${base}/debug/effect`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(msg),
    });

  // render → ensure create（恰好一次）
  await emitEffect({ kind: "render_component", spec });
  await poll(() => windowLog.some((a) => a.action === "create" && a.label === "card-t1"), "card-t1 创建");
  // 同 id 再 render → reuse（不第二次 create；spec 重发到同一窗口）
  await emitEffect({ kind: "render_component", spec: { ...spec, text: "v2" } });
  await new Promise((r) => setTimeout(r, 300));
  expect(windowLog.filter((a) => a.action === "create" && a.label === "card-t1").length).toBe(1);
  // close → 统一关闭
  await emitEffect({ kind: "close_component", id: "t1" });
  await poll(() => windowLog.some((a) => a.action === "destroy" && a.label === "card-t1"), "card-t1 销毁");
  // 复活路径：关闭后 render → 干净重建（无将死窗口竞态）
  await emitEffect({ kind: "render_component", spec });
  await poll(
    () => windowLog.filter((a) => a.action === "create" && a.label === "card-t1").length === 2,
    "card-t1 关闭后重建",
  );
  // effect 流（沙盒 effect.jsonl）：window_opened / window_closed 各至少一条
  await poll(() => {
    const eff = readEffects(core);
    return eff.includes('"kind":"window_opened"') && eff.includes('"kind":"window_closed"');
  }, "effect.jsonl 含 window_opened/closed");
});

it("T4 #26：× 走 intentClose——userClosed + release（非 remove）+ 钩子", async () => {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  const mount = document.createElement("div");
  document.body.appendChild(mount);
  const release = vi.fn();
  const remove = vi.fn();
  const engine = {
    release,
    remove,
    place: () => ({ x: 100, y: 100 }),
  } as unknown as PositioningEngine;
  const panel = new ChatPanel(mount, bridge, store, engine);
  const hook = vi.fn();
  panel.onIntentClose = hook;

  panel.open(); // 唤出（engine.place 固定 sse）
  expect(panel.isVisible()).toBe(true);
  (mount.querySelector(".chat-close") as HTMLButtonElement).click(); // ×
  expect(panel.userClosed).toBe(true);
  expect(panel.isVisible()).toBe(false);
  expect(release).toHaveBeenCalledWith("chat-panel");
  expect(remove).not.toHaveBeenCalled(); // 不是 dismiss 语义
  expect(hook).toHaveBeenCalled(); // windowed 副作用钩子（requestRelease+adapter.hide）
});

it("T3b #25 压测：create→close→update 快速序列下不重复、不复活", async () => {
  const base = core.base;
  const spec = { id: "stress", type: "text_card", title: "S", text: "v1" };
  const emitEffect = (msg: unknown) =>
    fetch(`${base}/debug/effect`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(msg),
    });
  // 快速连续 create→close→create→close→create（#25 原始复现形态）
  for (let round = 0; round < 3; round++) {
    await emitEffect({ kind: "render_component", spec: { ...spec, text: `v${round}` } });
    await new Promise((r) => setTimeout(r, 150)); // 等 WS→renderCard→ensure 链落定（close 不得抢在 render 前）
    await emitEffect({ kind: "close_component", id: "stress" });
    await new Promise((r) => setTimeout(r, 150));
  }
  // 最终再 render 一次（复活路径：已关闭卡不得复活旧内容；同 id 应干净重建）
  await emitEffect({ kind: "render_component", spec: { ...spec, text: "final" } });
  await new Promise((r) => setTimeout(r, 600));
  const creates = windowLog.filter((a) => a.action === "create" && a.label === "card-stress").length;
  const destroys = windowLog.filter((a) => a.action === "destroy" && a.label === "card-stress").length;
  expect(creates).toBe(4); // 循环 3 次重建 + 最终干净再建，无一次漏判或重复
  expect(destroys).toBe(3); // 每次 close 都销毁
  // 任意时刻同 label 只有一个存活窗口（mock 注册表无重影）
  const alive = windowLog.filter((a) => a.label === "card-stress");
  const net = alive.filter((a) => a.action === "create").length - alive.filter((a) => a.action === "destroy").length;
  expect(net).toBe(1); // 恰好一窗存活
});

it("T5 #25 根因 A：windowed ComponentManager 不订阅全局 render 流", async () => {
  const bridge = await createBridge();
  const mountWin = document.createElement("div");
  const mountBrowser = document.createElement("div");
  document.body.append(mountWin, mountBrowser);
  new ComponentManager(mountWin, bridge, () => ({ x: 0, y: 0 }), true); // windowed
  new ComponentManager(mountBrowser, bridge, () => ({ x: 0, y: 0 }), false); // browser 对照
  await fetch(`${core.base}/debug/effect`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      kind: "render_component",
      spec: { id: "wm", type: "text_card", title: "W", text: "x" },
    }),
  });
  await poll(() => mountBrowser.querySelector(".component") !== null, "browser mgr 渲染");
  expect(mountWin.querySelector(".component")).toBeNull(); // windowed 不受全局流污染
  // windowed 仍渲染定向 spec（card:spec 路径 = 直接 render 调用）
  const cardWin = getMockWindow("card-wm");
  expect(cardWin).toBeTruthy(); // pet 的 renderCard 已建窗
});
