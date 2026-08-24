// windowed 单卡不变式 + card:spec label 过滤（#25 残留加固）：
// card 窗是 Tauri 专用窗（CDP 测不到），守卫逻辑抽纯函数/jsdom 单测覆盖。
// windowed ComponentManager 不订阅 bridge 流——BrowserMockBridge 仅作构造参数。

import { expect, it } from "vitest";
import { BrowserMockBridge, type ComponentSpec } from "../src/bridge";
import { ComponentManager } from "../src/components/component-manager";
import { acceptsSpec } from "../src/windows/card-window";

const textCard = (id: string, text: string): ComponentSpec => ({
  id,
  type: "text_card",
  title: id,
  text,
});

it("windowed ComponentManager：首张卡定身份，异 id spec 忽略不堆叠", () => {
  const mount = document.createElement("div");
  const mgr = new ComponentManager(mount, new BrowserMockBridge(), () => ({ x: 0, y: 0 }), true);

  mgr.render(textCard("a", "v1"));
  mgr.render(textCard("b", "异卡"));
  const cards = mount.querySelectorAll(".component");
  expect(cards.length).toBe(1); // 一窗一卡：异 id 不 appendChild
  expect(cards[0].textContent).toContain("v1");
  expect(cards[0].textContent).not.toContain("异卡");

  // 同 id 原地更新不受守卫影响（持续管理协议）
  mgr.render(textCard("a", "v2"));
  expect(mount.querySelectorAll(".component").length).toBe(1);
  expect(mount.querySelector(".component")!.textContent).toContain("v2");

  // 卡关闭后身份不释放：临终窗口期异 id 仍被拒
  mgr.closeById("a");
  mgr.render(textCard("b", "异卡"));
  expect(mount.querySelectorAll(".component").length).toBe(0);
});

it("acceptsSpec：label 与 spec.id 一致才放行", () => {
  expect(acceptsSpec("card-a", textCard("a", "x"))).toBe(true);
  expect(acceptsSpec("card-a", textCard("b", "x"))).toBe(false);
  // 嵌套 id（含 / 子目录）按整串匹配
  expect(acceptsSpec("card-proj/nested-1", textCard("proj/nested-1", "x"))).toBe(true);
  expect(acceptsSpec("card-proj/nested-1", textCard("proj", "x"))).toBe(false);
});
