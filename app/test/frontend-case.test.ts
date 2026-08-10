// 前端进 case 首套（docs/case-runner.md §前端进 case，壳类比落地形态）：
// 真前端模块（store / RemoteBridge / pet main 浏览器分支 / ChatPanel / ComponentManager）
// × case-runner 内嵌真 core（overseer-case frontend 拉起本进程，RemoteBridge 连）。
// 断言对象：store 基线与回读、Queue 放行回读、#25 同 id 不重复/不复活（DOM 层）、
// #26 统一关闭、windowed ComponentManager 不订全局流。
// shell 窗口决策（ensure/close_card_window，Rust 权威注册表）由壳 cargo 测试覆盖；
// 本环境无 __TAURI_INTERNALS__，pet 跑浏览器分支——卡片经 ComponentManager DOM 模式。

import { beforeAll, expect, it, vi } from "vitest";
import { waitCore, coreBase } from "./shim";
import { createBridge } from "../src/bridge";
import { Store } from "../src/store";
import { ChatPanel } from "../src/windows/chat";
import { ComponentManager } from "../src/components/component-manager";
import type { PositioningEngine } from "../src/positioning/engine";

async function poll<T>(fn: () => T | null | undefined | false, what: string, ms = 8000): Promise<T> {
  const t0 = Date.now();
  for (;;) {
    const v = fn();
    if (v) return v;
    if (Date.now() - t0 > ms) throw new Error(`poll 超时: ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

const emitEffect = (msg: unknown) =>
  fetch(`${coreBase()}/debug/effect`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(msg),
  });

const cardsById = (id: string) =>
  [...document.querySelectorAll(".component")].filter(
    (el) => (el as HTMLElement).dataset.id === id,
  );

beforeAll(async () => {
  await waitCore();
  document.body.innerHTML = '<div id="app"></div>';
}, 60000);

it("T1 store 基线：config/top_state/context/cards 经 RemoteBridge 从真 core 拉取", async () => {
  const bridge = await createBridge(); // 无 __TAURI_INTERNALS__ → RemoteBridge（真 HTTP+WS）
  const store = await Store.create(bridge);
  expect(store.config?.viewScale).toBeTypeOf("number");
  expect(store.config?.kaomoji.system.idle).toBeTruthy();
  // 共享 core（case-runner 单例，其他文件已写入）：只断言通路形态，不断言空基线
  expect(Array.isArray(store.topState?.instances)).toBe(true);
  expect(Array.isArray(store.context)).toBe(true);
  expect(Array.isArray(store.cards)).toBe(true);
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

it("T3 #25 前端语义：同 id render 原地更新不重复，close 移除后可干净重建", async () => {
  const { main: petMain } = await import("../src/windows/pet");
  await petMain(); // pet 浏览器分支在 jsdom 全量启动（ComponentManager DOM 模式）
  const spec = { id: "t1", type: "text_card", title: "T", text: "hello" };

  // render → DOM 建卡
  await emitEffect({ kind: "render_component", spec });
  await poll(() => cardsById("t1").length === 1, "card t1 渲染");
  // 同 id 再 render → 原地更新（docs/components.md 持续管理协议），不复制
  await emitEffect({ kind: "render_component", spec: { ...spec, text: "v2" } });
  await poll(
    () => (cardsById("t1")[0] as HTMLElement).textContent?.includes("v2"),
    "card t1 原地更新 v2",
  );
  expect(cardsById("t1").length).toBe(1);
  // close → DOM 移除
  await emitEffect({ kind: "close_component", id: "t1" });
  await poll(() => cardsById("t1").length === 0, "card t1 移除");
  // 复活路径：关闭后 render → 干净重建
  await emitEffect({ kind: "render_component", spec });
  await poll(() => cardsById("t1").length === 1, "card t1 关闭后重建");
  // 注：/debug/effect 是驱动注入（不经 LLM/execute_tool 创建点），不落 effect.jsonl——
  // 本测试断言 DOM 层语义；effect 记录链路由 core 单测覆盖
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

it("T3b #25 压测：create→close→update 快速序列下同 id 不重复、不复活", async () => {
  const spec = { id: "stress", type: "text_card", title: "S", text: "v1" };
  // 快速连续 create→close→create→close→create（#25 原始复现形态）
  for (let round = 0; round < 3; round++) {
    await emitEffect({ kind: "render_component", spec: { ...spec, text: `v${round}` } });
    await new Promise((r) => setTimeout(r, 150)); // 等 WS→render 链落定（close 不得抢在 render 前）
    await emitEffect({ kind: "close_component", id: "stress" });
    await new Promise((r) => setTimeout(r, 150));
  }
  // 最终再 render 一次（复活路径：已关闭卡不得复活旧内容；同 id 应干净重建）
  await emitEffect({ kind: "render_component", spec: { ...spec, text: "final" } });
  await poll(() => cardsById("stress").length === 1, "stress 最终恰好一张");
  expect((cardsById("stress")[0] as HTMLElement).textContent).toContain("final");
  expect(cardsById("stress").length).toBe(1); // 无重影
});

it("T5 #25 根因 A：windowed ComponentManager 不订阅全局 render 流", async () => {
  const bridge = await createBridge();
  const mountWin = document.createElement("div");
  const mountBrowser = document.createElement("div");
  document.body.append(mountWin, mountBrowser);
  new ComponentManager(mountWin, bridge, () => ({ x: 0, y: 0 }), true); // windowed
  new ComponentManager(mountBrowser, bridge, () => ({ x: 0, y: 0 }), false); // browser 对照
  await emitEffect({
    kind: "render_component",
    spec: { id: "wm", type: "text_card", title: "W", text: "x" },
  });
  await poll(() => mountBrowser.querySelector(".component") !== null, "browser mgr 渲染");
  expect(mountWin.querySelector(".component")).toBeNull(); // windowed 不受全局流污染
  // windowed 的窗口创建决策（ensure/close_card_window）是壳侧 Rust 逻辑——壳 cargo 测试覆盖
});
