// chat 交互原则前端 case：
// 滚动意图态机 / IME 守卫 / 自增长 / 发送按钮 / 失败保文重试 / 排队状态翻译 / 回应提示。
// jsdom 无布局——滚动几何用 defineProperty 打桩（scrollHeight/clientHeight/scrollTop）。

import { beforeAll, expect, it, vi } from "vitest";
import { waitCore, coreBase, readEffects } from "./shim";
import { createBridge, type Bridge } from "../src/bridge";
import { Store } from "../src/store";
import { ChatPanel } from "../src/windows/chat";
import type { PositioningEngine } from "../src/positioning/engine";

beforeAll(async () => {
  await waitCore();

  document.body.innerHTML = '<div id="app"></div>';
}, 60000);

const fakeEngine = {
  release: () => {},
  remove: () => {},
  place: () => ({ x: 100, y: 100 }),
} as unknown as PositioningEngine;

function stubScroll(el: HTMLElement, scrollHeight: number, clientHeight: number, scrollTop: number) {
  Object.defineProperty(el, "scrollHeight", { value: scrollHeight, configurable: true });
  Object.defineProperty(el, "clientHeight", { value: clientHeight, configurable: true });
  el.scrollTop = scrollTop;
}

async function makePanel(): Promise<{ panel: ChatPanel; mount: HTMLElement; bridge: Bridge; store: Store }> {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  const mount = document.createElement("div");
  document.body.appendChild(mount);
  const panel = new ChatPanel(mount, bridge, store, fakeEngine);
  panel.open();
  return { panel, mount, bridge, store };
}

it("IME 组合输入中 Enter 只确认候选不误发送；Shift+Enter 换行；发送按钮与 Enter 同语义", async () => {
  const { mount } = await makePanel();
  const input = mount.querySelector<HTMLTextAreaElement>(".chat-input")!;
  const sendBtn = mount.querySelector<HTMLButtonElement>(".chat-send")!;
  // 共享 core（ambery-case frontend 单例）：context 可能已有其他文件的消息——断言用相对计数
  const bubbles = () => mount.querySelectorAll(".chat-user").length;
  const base = bubbles();
  input.value = "にほんご";
  // IME 组合中 Enter：不发送
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true }));
  expect(bubbles()).toBe(base);
  expect(input.value).toBe("にほんご"); // 文字不动
  // Shift+Enter：换行不发送
  input.value = "第一行";
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true, cancelable: true }));
  expect(bubbles()).toBe(base);
  expect(input.value).toBe("第一行");
  // Enter 发送
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: false, bubbles: true, cancelable: true }));
  await vi.waitFor(() =>
    expect([...mount.querySelectorAll(".chat-user")].some((u) => u.textContent === "第一行")).toBe(true),
  );
  expect(input.value).toBe(""); // 发送后清空
  // 发送按钮同语义：空白禁用
  expect(sendBtn.disabled).toBe(true);
  input.value = "第二行";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  expect(sendBtn.disabled).toBe(false);
  sendBtn.click();
  await vi.waitFor(() => expect(bubbles()).toBe(base + 2));
});

it("发送失败：文字退回输入框 + 错误行 + 重试路径", async () => {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  // 制造失败（不调真方法——否则会真写 Queue，context 回流重渲会把错误行冲掉）
  const orig = bridge.appendUserMessage.bind(bridge);
  bridge.appendUserMessage = async () => false;
  const mount = document.createElement("div");
  document.body.appendChild(mount);
  new ChatPanel(mount, bridge, store, fakeEngine).open();
  const input = mount.querySelector<HTMLTextAreaElement>(".chat-input")!;
  input.value = "会失败的消息";
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  await vi.waitFor(() => expect(mount.querySelector(".chat-send-failed")).toBeTruthy());
  // Context 全量重渲不冲掉失败提示（界面瞬态保留到用户处理）
  const msgs = [...(store.context ?? [])];
  (store as unknown as { contextData: unknown }).contextData = msgs;
  for (const cb of (store as unknown as { contextListeners: Set<(m: unknown) => void> }).contextListeners) cb(msgs);
  expect(mount.querySelector(".chat-send-failed")).toBeTruthy();
  expect(input.value).toBe("会失败的消息"); // 文字不丢（继续编辑路径）
  // 重试路径：恢复成功后点重试
  bridge.appendUserMessage = orig;
  mount.querySelector<HTMLButtonElement>(".chat-send-failed button")!.click();
  await vi.waitFor(() => {
    const users = [...mount.querySelectorAll(".chat-user")];
    expect(users.some((u) => u.textContent === "会失败的消息")).toBe(true);
  });
  expect(mount.querySelector(".chat-send-failed")).toBeNull();
});

it("滚动意图：跟随贴底；滚离后新消息只提示不抢视口；点提示回底清零", async () => {
  // 共享 core（ambery-case frontend 单例）：先归一 UI 语言为 zh，防文件序影响
  await fetch(`${coreBase()}/config`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path: "ui_language", value: "zh" }),
  });
  const { mount, store } = await makePanel();
  // open() 的 scrollToBottom 把 suppressScroll 挂到下一宏任务——先越过，再模拟用户滚动
  await new Promise((r) => setTimeout(r, 20));
  const history = mount.querySelector<HTMLElement>(".chat-history")!;
  stubScroll(history, 1000, 300, 700); // 贴底
  history.dispatchEvent(new Event("scroll")); // 确认跟随
  expect((mount.querySelector(".chat-pill") as HTMLElement).hidden).toBe(true);

  // 用户滚离底部 → 阅读历史
  stubScroll(history, 1000, 300, 200);
  history.dispatchEvent(new Event("scroll"));
  // 新消息到达（经 store 注入 context 变化）
  const base = [...(store.context ?? [])];
  const msgs = [...base, { role: "assistant", content: "新回复一", ts: Date.now() }, { role: "assistant", content: "新回复二", ts: Date.now() }];
  (store as unknown as { contextData: unknown }).contextData = msgs;
  for (const cb of (store as unknown as { contextListeners: Set<(m: unknown) => void> }).contextListeners) cb(msgs);
  await vi.waitFor(() => expect((mount.querySelector(".chat-pill") as HTMLElement).hidden).toBe(false));
  expect(mount.querySelector(".chat-pill")!.textContent).toMatch(/↓ 2 条新消息/);
  expect(history.scrollTop).toBe(200); // 视口不被抢
  // 点击提示 → 回底 + 清零 + 恢复跟随
  (mount.querySelector(".chat-pill") as HTMLElement).click();
  expect((mount.querySelector(".chat-pill") as HTMLElement).hidden).toBe(true);
  expect(history.scrollTop).toBe(1000); // scrollToBottom 设 scrollTop=scrollHeight
});

it("回应提示：发送后出现「…」，delta 到达即消失；排队状态翻译", async () => {
  const { mount, bridge } = await makePanel();
  const input = mount.querySelector<HTMLTextAreaElement>(".chat-input")!;
  const sendOne = async (text: string) => {
    input.value = text;
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(mount.querySelector(`.chat-user`)).toBeTruthy());
  };
  // 连发两条：awaitingReply=2 → 第一条正在回应（…），第二条已排队（1）
  await sendOne("第一条");
  expect(mount.querySelector(".chat-replying")).toBeTruthy(); // 回应提示
  await sendOne("第二条");
  await vi.waitFor(() => expect((mount.querySelector(".chat-queue-status") as HTMLElement).hidden).toBe(false));
  expect(mount.querySelector(".chat-queue-status")!.textContent).toContain("1");
  // delta 到达：回应提示消失、streaming 开始
  (bridge as unknown as { deltaListeners?: ((d: { content?: string }) => void)[] });
  // 经 shim effect 总线注入 delta/done
  await fetch(`${coreBase()}/debug/effect`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ kind: "assistant_delta", content: "回答中" }),
  });
  await vi.waitFor(() => expect(mount.querySelector(".chat-replying")).toBeNull());
  await fetch(`${coreBase()}/debug/effect`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ kind: "assistant_done" }),
  });
  await fetch(`${coreBase()}/debug/effect`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ kind: "assistant_done" }),
  });
  await vi.waitFor(() => expect((mount.querySelector(".chat-queue-status") as HTMLElement).hidden).toBe(true));
});

it("UI 动作记录：渲染用户气泡/错误气泡/错误 banner 时 effect.jsonl 有对应记录", async () => {
  // effect 语义：记录动作不驱动渲染——前端渲染 UI 单元后经上报通道记录
  const { mount } = await makePanel();
  const input = mount.querySelector<HTMLTextAreaElement>(".chat-input")!;

  // 用户气泡 → user_bubble
  const baseUser = (readEffects().match(/"user_bubble"/g) ?? []).length;
  input.value = "气泡记录测试";
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  await vi.waitFor(() =>
    expect((readEffects().match(/"user_bubble"/g) ?? []).length).toBeGreaterThan(baseUser),
  );
  expect(readEffects()).toContain("气泡记录测试");

  // 错误通知按 retention 路由（错误即通知模型）：transient → 气泡（不开 banner）；
  // persistent → banner（不开气泡）。经 shim effect 总线注入 error 事件驱动
  const { panel } = await makePanel();
  panel.onOpenSetup = () => {};
  const baseErr = (readEffects().match(/"error_bubble"/g) ?? []).length;
  const baseBanner = (readEffects().match(/"setup_banner"/g) ?? []).length;
  const errMsg = "LLM 调用失败：连接超时";
  await fetch(`${coreBase()}/debug/effect`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ kind: "error", message: errMsg, retention: "transient" }),
  });
  await vi.waitFor(() =>
    expect((readEffects().match(/"error_bubble"/g) ?? []).length).toBeGreaterThan(baseErr),
  );
  expect(readEffects()).toContain(errMsg);
  // transient 只气泡：banner 不增（retention 是唯一路由轴）
  expect((readEffects().match(/"setup_banner"/g) ?? []).length).toBe(baseBanner);

  // persistent + action=setup → banner（文案 = 后端 message 进 DOM；记录带 state）
  const bannerMsg = "provider「x」初始化失败：环境变量未设置";
  await fetch(`${coreBase()}/debug/effect`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ kind: "error", message: bannerMsg, retention: "persistent", action: "setup" }),
  });
  await vi.waitFor(() =>
    expect((readEffects().match(/"setup_banner"/g) ?? []).length).toBeGreaterThan(baseBanner),
  );
  // banner 文案 = 后端 message（本面板的 banner 元素）
  const banner = [...document.querySelectorAll(".chat-setup-banner")].find((b) =>
    b.textContent?.includes(bannerMsg),
  );
  expect(banner).toBeTruthy();
});
