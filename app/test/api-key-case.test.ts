// provider key 输入（形态乙）前端 case：
// 经 RemoteBridge 真链路（HTTP+WS 连 case-runner 内嵌 core）验证：
// - renderApiKeyRow 三态渲染（本地端点无需 key / 未设置警示 / 已设置占位+清除）
// - 保存（填值）→ env 文件写入 → 状态刷新 → onChanged 回调
// - 清除 → env 文件删除 → 状态回到未设置
// 注意：共享单例 core（ambery-case frontend），env 文件在 AMBERY_CONFIG_DIR 沙盒——
// 不碰真实 ~/.config/ambery/env（case 不保存 env 到真实环境）。

import { beforeAll, expect, it, vi } from "vitest";
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

it("未设置 → 警示占位；保存 key → 已设置占位 + 来源提示 + 清除按钮；清除 → 回未设置", async () => {
  const bridge = await makeBridge();
  const provider = `t${Date.now()}`;
  const onChanged = vi.fn();

  // 远程端点（local=false）——用真实 core 的 set_api_key 写 env（隔离沙盒）
  const row = renderApiKeyRow(provider, "AMBERY_TEST_API_KEY", false, bridge, {
    readOnly: false,
    onChanged,
  });
  document.body.appendChild(row);

  const input = row.querySelector<HTMLInputElement>(".api-key-input")!;
  const hint = row.querySelector<HTMLElement>(".api-key-hint")!;

  // 初始：未设置（沙盒无该 key）
  await vi.waitFor(() => expect(hint.textContent).toContain("未设置"));

  // 填值保存
  input.value = "sk-test-123";
  const saveBtn = [...row.querySelectorAll("button")].find((b) => b.textContent === "保存")!;
  saveBtn.click();
  await vi.waitFor(() => expect(onChanged).toHaveBeenCalled());
  await vi.waitFor(() => expect(hint.textContent).toContain("已设置"));
  expect(input.placeholder).toContain("已设置");
  const clearBtn = [...row.querySelectorAll("button")].find((b) => b.textContent === "清除")!;
  expect(clearBtn.hidden).toBe(false);

  // 清除 → 回未设置
  clearBtn.click();
  await vi.waitFor(() => expect(onChanged).toHaveBeenCalledTimes(2));
  await vi.waitFor(() => expect(hint.textContent).toContain("未设置"));
  expect(input.placeholder).toContain("请输入");
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
