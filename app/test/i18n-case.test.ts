// i18n 模块前端 case：ui_language 字段校验 + 切换即重渲染 + 机器契约不译。

import { beforeAll, expect, it, vi } from "vitest";
import { waitCore, coreBase } from "./shim";
import { createBridge } from "../src/bridge";
import { Store } from "../src/store";
import { t, uiLanguage } from "../src/i18n";
import { ChatPanel } from "../src/windows/chat";
import { ComponentManager } from "../src/components/component-manager";

beforeAll(async () => {
  await waitCore();

  document.body.innerHTML = '<div id="app"></div>';
}, 60000);

const postConfig = (path: string, value: unknown) =>
  fetch(`${coreBase()}/config`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path, value }),
  }).then((r) => r.json());

it("字段默认与校验：harness_language 默认 zh；非法语言原子拒绝", async () => {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  expect(["zh", "en"]).toContain(store.config?.uiLanguage);
  // harness_language 不进前端投影（core 内部）；经 schema 验证其默认与枚举
  const schema = (await (await fetch(`${coreBase()}/config/schema`)).json()) as {
    nodes: { path: string; value: unknown }[];
  };
  const harness = schema.nodes.find((n) => n.path === "harness_language");
  expect(harness?.value).toBe("zh");
  // OneOf 校验：fr 原子拒绝
  const r = await postConfig("ui_language", "fr");
  expect(r.ok).toBe(false);
});

it("切换 ui_language 即重渲染：card chrome 跟随", async () => {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  const mount = document.createElement("div");
  document.body.appendChild(mount);
  const panel = new ChatPanel(mount, bridge, store);
  const mgr = new ComponentManager(mount, bridge, () => ({ x: 0, y: 0 }), false);
  mgr.render({ id: "i1", type: "text_card", title: "T", text: "x" });

  const lang0 = store.config?.uiLanguage ?? "zh";
  const lang1 = lang0 === "zh" ? "en" : "zh";
  if (lang0 !== "zh") {
    // 先归一到 zh 做基线（机器 locale 影响首启默认）
    expect((await postConfig("ui_language", "zh")).ok).toBe(true);
    await vi.waitFor(() => expect(uiLanguage()).toBe("zh"));
  }
  const input = mount.querySelector(".chat-input") as HTMLInputElement;
  expect(input.placeholder).toBe("");
  expect(mount.querySelector(".cmp-body button")?.textContent).toBe("复制");

  expect((await postConfig("ui_language", lang1)).ok).toBe(true);
  await vi.waitFor(() => expect(uiLanguage()).toBe(lang1));
  // card chrome 原地重贴（DOM 不重建）
  const expectedCopy = lang1 === "en" ? "Copy" : "复制";
  await vi.waitFor(() =>
    expect(mount.querySelector(".cmp-body button")?.textContent).toBe(expectedCopy),
  );
  expect(panel).toBeTruthy();

  // 机器契约不译：Config path 原样（t() 不涉及 path）；名称不参与翻译
  expect(t("pet.default-name")).toBe("pet");

  // 共享 core（ambery-case frontend 单例）：恢复初始语言，防跨文件污染
  await postConfig("ui_language", lang0);
  await vi.waitFor(() => expect(uiLanguage()).toBe(lang0));
});

it("插值工作", async () => {
  expect(t("chat.queued", { n: 3 })).toContain("3");
});

it("pet 名称：Config name 流入 chat 标题，改名即重贴（名称）", async () => {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  const mount = document.createElement("div");
  document.body.appendChild(mount);
  new ChatPanel(mount, bridge, store);
  const title = mount.querySelector(".chat-header span")!;
  // 默认名（改名轮定案）：Ambery
  expect(title.textContent).toBe("Ambery");
  // 改名 → 标题即当前名称
  expect((await postConfig("name", "監督ちゃん")).ok).toBe(true);
  await vi.waitFor(() => expect(title.textContent).toBe("監督ちゃん"));
  // 校验：空名原子拒绝
  expect((await postConfig("name", " ")).ok).toBe(false);
});
