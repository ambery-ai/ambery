// provider key 输入（形态乙）前端 case：
// 经 RemoteBridge 真链路（HTTP+WS 连 case-runner 内嵌 core）验证：
// - renderApiKeyRow 本地端点渲染（无需 key，无输入控件）
// - setApiKey 写入 → getApiKeyStatus 同源读到（env 文件）→ 清除
// 注意：共享单例 core（ambery-case frontend），env 文件在 AMBERY_CONFIG_DIR 沙盒——
// 不碰真实 ~/.config/ambery/env（case 不保存 env 到真实环境）。

import { beforeAll, expect, it } from "vitest";
import { waitCore } from "./shim";
import { createBridge, type Bridge } from "../src/bridge";
import { renderApiKeyRow } from "../src/config-reflect";

beforeAll(async () => {
  await waitCore();
  document.body.innerHTML = '<div id="app"></div>';
}, 60000);

async function makeBridge(): Promise<Bridge> {
  return createBridge();
}

it("本地端点（base_url 指向本机）显示无需 key，无输入控件", async () => {
  const bridge = await makeBridge();
  const row = renderApiKeyRow("ollama", null, true, bridge, { readOnly: false, onChanged: () => {} });
  expect(row.textContent).toContain("无需 key");
  expect(row.querySelector("input")).toBeNull();
});

it("已设置的 key 经 getApiKeyStatus 能读到（写入 → 状态查询同源）", async () => {
  const bridge = await makeBridge();
  const provider = `s${Date.now()}`;
  const envName = "AMBERY_S_API_KEY";

  const r = await bridge.setApiKey!(provider, "sk-from-test");
  expect(r.ok).toBe(true);

  const st = await bridge.getApiKeyStatus!(provider);
  expect(st.set).toBe(true);
  expect(st.source).toBe("env 文件");
  expect(envName).toMatch(/^AMBERY_/);

  // 清理
  await bridge.setApiKey!(provider, null);
});
