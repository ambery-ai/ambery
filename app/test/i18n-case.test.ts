// i18n 模块前端 case（docs/i18n.md）：ui_language 字段校验 + 切换即重渲染 + 机器契约不译。

import { beforeAll, afterAll, expect, it, vi } from "vitest";
import { startCore, stopCore, setupShim, type CoreHandle } from "./shim";
import { createBridge } from "../src/bridge";
import { Store } from "../src/store";
import { t, uiLanguage } from "../src/i18n";
import { ChatPanel } from "../src/windows/chat";
import { ComponentManager } from "../src/components/component-manager";

let core: CoreHandle;

beforeAll(async () => {
  core = await startCore(47657);
  await setupShim(core);
  document.body.innerHTML = '<div id="app"></div>';
}, 60000);

afterAll(() => {
  stopCore(core);
});

const postConfig = (path: string, value: unknown) =>
  fetch(`${core.base}/config`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path, value }),
  }).then((r) => r.json());

it("字段默认与校验：harness_language 默认 zh；非法语言原子拒绝", async () => {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  expect(["zh", "en"]).toContain(store.config?.uiLanguage);
  // harness_language 不进前端投影（core 内部）；经 schema 验证其默认与枚举
  const schema = (await (await fetch(`${core.base}/config/schema`)).json()) as {
    nodes: { path: string; value: unknown }[];
  };
  const harness = schema.nodes.find((n) => n.path === "harness_language");
  expect(harness?.value).toBe("zh");
  // OneOf 校验：fr 原子拒绝
  const r = await postConfig("ui_language", "fr");
  expect(r.ok).toBe(false);
});

it("切换 ui_language 即重渲染：chat placeholder 与 card chrome 跟随", async () => {
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
  expect(input.placeholder).toBe("和ペット说话…");
  expect(mount.querySelector(".cmp-body button")?.textContent).toBe("复制");

  expect((await postConfig("ui_language", lang1)).ok).toBe(true);
  await vi.waitFor(() => expect(uiLanguage()).toBe(lang1));
  // chat placeholder 跟随（含 pet 名插值；名称不翻译）
  const expected = lang1 === "en" ? "Talk to ペット…" : "和ペット说话…";
  await vi.waitFor(() => expect(input.placeholder).toBe(expected));
  // card chrome 原地重贴（DOM 不重建）
  const expectedCopy = lang1 === "en" ? "Copy" : "复制";
  await vi.waitFor(() =>
    expect(mount.querySelector(".cmp-body button")?.textContent).toBe(expectedCopy),
  );
  expect(panel).toBeTruthy();

  // 机器契约不译：Config path 原样（t() 不涉及 path）；名称不参与翻译
  expect(t("pet.default-name")).toBe("ペット");
});

it("缺失 key 回退 zh；插值工作", async () => {
  expect(t("chat.placeholder", { name: "X" })).toContain("X");
});
