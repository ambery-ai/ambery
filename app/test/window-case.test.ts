// 前端窗口 effect 接入 headless case：
// MockWindow 经 tauri_runtime_actions 产生 window_* effect，经 RemoteBridge
// POST /effect 落入沙盒 effect.jsonl（origin=frontend）。
// 断言面 = windowLog（内存记录面）+ 沙盒 effect.jsonl（动作流观测面）。

import { beforeAll, expect, it } from "vitest";
import {
  waitCore,
  resetWindowWorld,
  setCurrentWindow,
  getMockWindow,
  readEffects,
  windowLog,
} from "./shim";
import {
  hideWindow,
  moveWindow,
  resizeWindow,
  showWindow,
  closeWindow,
} from "../src/tauri_runtime_actions";

function effects(): string {
  return readEffects();
}

async function poll<T>(fn: () => T | null | undefined | false, what: string, ms = 8000): Promise<T> {
  const t0 = Date.now();
  for (;;) {
    const v = fn();
    if (v) return v;
    if (Date.now() - t0 > ms) throw new Error(`poll 超时: ${what}\n当前 effects:\n${effects()}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

beforeAll(async () => {
  await waitCore();
  resetWindowWorld();
  setCurrentWindow("card-x");
}, 60000);

it("W1 mock 窗口动作经动作层入 effect.jsonl（resize/move/show/hide）", async () => {
  const win = getMockWindow("card-x")!;
  await resizeWindow(win, 320, 200);
  await moveWindow(win, 12, 34);
  await showWindow(win);
  await hideWindow(win);

  // 高频 move/resize 250ms 打包后落盘；show/hide 即时
  await poll(
    () => effects().includes('"kind":"window_visible"'),
    "window_visible 落盘",
  );
  await poll(
    () => effects().includes('"kind":"window_hidden"'),
    "window_hidden 落盘",
  );
  await poll(
    () => effects().includes('"kind":"window_resized"'),
    "window_resized 落盘",
  );
  await poll(
    () => effects().includes('"kind":"window_moved"'),
    "window_moved 落盘",
  );

  const raw = effects();
  expect(raw).toContain('"origin":"frontend"');
  expect(raw).toContain('"window":"card-x"');
  // 内存记录面同步保留（多窗口测试的窗口动作记录）
  expect(windowLog.some((a) => a.action === "setSize")).toBe(true);
  expect(windowLog.some((a) => a.action === "show")).toBe(true);
});

it("W2 close 经动作层记录 window_closed", async () => {
  setCurrentWindow("card-y");
  const win = getMockWindow("card-y")!;
  await showWindow(win);
  await closeWindow(win);
  await poll(
    () => effects().includes('"kind":"window_closed"'),
    "window_closed 落盘",
  );
  expect(windowLog.some((a) => a.action === "destroy" && a.label === "card-y")).toBe(true);
});
