// theme 模块前端 case（docs/theme.md）：store 事件 → applyTheme → token 表覆写语义。
// 另含 KNOWN_TOKENS ↔ styles.css :root 的 parity 守卫（node fs 直读样式表）。

import { beforeAll, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { waitCore, coreBase } from "./shim";
import { createBridge } from "../src/bridge";
import { Store } from "../src/store";
import { applyTheme, wireTheme, KNOWN_TOKENS } from "../src/theme";

beforeAll(async () => {
  await waitCore(); // case-runner 内嵌 core（overseer-case frontend 拉起本进程）
}, 60000);

it("parity：KNOWN_TOKENS == styles.css :root 的 --ov-* 定义", () => {
  const css = readFileSync(join(__dirname, "../src/styles.css"), "utf8");
  const rootBlock = css.slice(css.indexOf(":root"), css.indexOf("/* ── 透明窗口"));
  const defined = [...rootBlock.matchAll(/(--ov-[a-z0-9-]+)\s*:/g)].map((m) =>
    m[1].replace(/^--ov-/, ""),
  );
  expect([...KNOWN_TOKENS].sort()).toEqual(defined.sort());
});

it("主题切换即写 token 表；切回 dark 清覆写回内置默认", async () => {
  const bridge = await createBridge();
  const store = await Store.create(bridge);
  wireTheme(store); // 窗口接线的同一入口

  // 初始 dark：无内联覆写
  expect(document.documentElement.style.getPropertyValue("--ov-panel-bg")).toBe("");

  // 注册自定义主题并切换（经统一修改管道：POST /config）
  const post = (path: string, value: unknown) =>
    fetch(`${coreBase()}/config`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path, value }),
    }).then((r) => r.json());
  let r = await post("themes.contrast", { "panel-bg": "rgba(1, 2, 3, 0.9)", "text": "#abcdef" });
  expect(r.ok).toBe(true);
  r = await post("theme", "contrast");
  expect(r.ok).toBe(true);

  // config 变化事件 → store → wireTheme → 内联覆写生效
  await vi.waitFor(() => {
    expect(document.documentElement.style.getPropertyValue("--ov-panel-bg")).toBe("rgba(1, 2, 3, 0.9)");
    expect(document.documentElement.style.getPropertyValue("--ov-text")).toBe("#abcdef");
  });

  // 切回 dark（空覆写表）→ 内联清除
  r = await post("theme", "dark");
  expect(r.ok).toBe(true);
  await vi.waitFor(() => {
    expect(document.documentElement.style.getPropertyValue("--ov-panel-bg")).toBe("");
    expect(document.documentElement.style.getPropertyValue("--ov-text")).toBe("");
  });
});

it("动态 enum 校验：theme 必须是 themes 的 key（原子拒绝）", async () => {
  const r = await fetch(`${coreBase()}/config`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path: "theme", value: "no-such-theme" }),
  }).then((r) => r.json());
  expect(r.ok).toBe(false);
});

it("主题表校验：非法 token 名 / 注入值原子拒绝", async () => {
  const r = await fetch(`${coreBase()}/config`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path: "themes.evil", value: { "BadToken": "x", "ok": "a;}/*" } }),
  }).then((r) => r.json());
  expect(r.ok).toBe(false);
});

it("主题切换是纯视觉变更：不写 DOM 结构/输入内容", async () => {
  const marker = document.createElement("div");
  marker.id = "keep-me";
  document.body.appendChild(marker);
  const before = document.body.innerHTML;
  applyTheme({ theme: "dark", themes: { dark: {} } } as never);
  expect(document.body.innerHTML).toBe(before);
  expect(document.getElementById("keep-me")).toBeTruthy();
});
