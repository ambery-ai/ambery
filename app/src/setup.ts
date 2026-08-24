// LLM 首启配置引导 modal（docs/llm-setup.md）：
// 从 Chat 打开；反射渲染 llm 相关 schema 节点（与设置面板同一 get_config_schema 投影），
// 非手写表单；提供 provider 选择、key 输入（形态乙——写应用级 env 文件）、测试连通。
// menu 本身不变——引导不在 menu 里，也不改变 menu 行为。

import type { Bridge } from "./bridge";
import { renderConfigNode, renderApiKeyRow } from "./config-reflect";
import { t } from "./i18n";

/** 打开引导 modal（overlay + 面板）；返回关闭函数（宿主可在 ChatPanel 关闭时一并收起） */
export function openSetupModal(bridge: Bridge): () => void {
  const overlay = document.createElement("div");
  overlay.className = "setup-overlay";

  const panel = document.createElement("div");
  panel.className = "setup-modal";

  const head = document.createElement("div");
  head.className = "setup-head";
  const close = document.createElement("button");
  close.className = "setup-close";
  close.textContent = "×";
  head.append(close);

  const body = document.createElement("div");
  body.className = "setup-body";
  body.textContent = t("menu.loading");

  panel.append(head, body);
  overlay.append(panel);
  document.body.append(overlay);

  const dismiss = () => overlay.remove();
  close.addEventListener("click", dismiss);
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) dismiss();
  });

  void render(bridge, body);
  return dismiss;
}

/** 渲染 llm 相关节点（过滤 schema 的 llm. 前缀）+ key 状态 + 测试连通 */
async function render(bridge: Bridge, body: HTMLElement) {
  let resp: Awaited<ReturnType<NonNullable<Bridge["getConfigSchema"]>>>;
  try {
    resp = await bridge.getConfigSchema!();
  } catch {
    body.textContent = t("menu.offline");
    return;
  }

  const llmNodes = resp.nodes.filter((n) => n.path === "llm" || n.path.startsWith("llm."));
  body.textContent = "";

  // 写值回调：setConfig + 重渲染（控件值变化后 schema 值归一，重绘 llm 区）
  const applyValue = (path: string, value: unknown) => {
    void bridge.setConfig!(path, value).then(() => void render(bridge, body));
  };

  // active 选择（enum select）+ 新增 provider（custom-select addMode，组件内实现）
  const activeNode = llmNodes.find((n) => n.path === "llm.active");
  if (activeNode) {
    body.appendChild(
      renderConfigNode(activeNode, {
        readOnly: resp.readOnly,
        applyValue: (p, v) => applyValue(p, v),
        enumAddons: {
          "llm.active": {
            addMode: {
              addLabel: "新增 provider（小写字母开头，仅小写/数字/_/-）",
              onAdd: async (name) => {
                const r = await bridge.setConfig!(`llm.providers.${name}`, {
                  base_url: "",
                  model: "",
                  // 统一 key 变量名约定（llm.rs:833）：AMBERY_<PROVIDER>_API_KEY（大写）
                  api_key_env: `AMBERY_${name.toUpperCase()}_API_KEY`,
                });
                if (r.ok) void render(bridge, body); // 重渲染：新 provider 进下拉选项
                return r;
              },
            },
          },
        },
      }),
    );
  }

  // provider 字段（当前 active 对应的 providers.<name>.* 节点）+ key 输入行
  const active = String(activeNode?.value ?? "");
  const provPrefix = `llm.providers.${active}.`;
  if (active && active !== "unconfigured" && active !== "debug") {
    const provNodes = llmNodes.filter((n) => n.path.startsWith(provPrefix));
    for (const n of provNodes) {
      body.appendChild(
        renderConfigNode(n, {
          readOnly: resp.readOnly,
          applyValue: (p, v) => applyValue(p, v),
        }),
      );
    }
    // key 输入行（形态乙）：本地端点（base_url 指向本机）无需 key；
    // 远程端点渲染密码框。保存/清除成功后自动重跑 test_llm（Q6a 定案）。
    const envNode = llmNodes.find((n) => n.path === `${provPrefix}api_key_env`);
    const envValue = envNode?.value as string | null | undefined;
    const baseUrlNode = llmNodes.find((n) => n.path === `${provPrefix}base_url`);
    const baseUrl = String(baseUrlNode?.value ?? "");
    const local = /localhost|127\.0\.0\.1|::1/i.test(baseUrl);
    body.appendChild(
      renderApiKeyRow(active, envValue, local, bridge, {
        readOnly: resp.readOnly,
        onChanged: () => void runTest(bridge, statusEl, testBtn),
      }),
    );
  }

  // key 状态 + 测试连通
  const statusEl = document.createElement("div");
  statusEl.className = "setup-test-status";
  body.appendChild(statusEl);

  const testBtn = document.createElement("button");
  testBtn.className = "setup-test-btn";
  testBtn.textContent = t("setup.test");
  testBtn.addEventListener("click", () => void runTest(bridge, statusEl, testBtn));
  body.appendChild(testBtn);

  // 打开即自动检测一次（key 状态 = test_llm 结果）
  void runTest(bridge, statusEl, testBtn);
}


async function runTest(bridge: Bridge, statusEl: HTMLElement, btn: HTMLButtonElement) {
  btn.disabled = true;
  statusEl.textContent = "…";
  try {
    const r = await bridge.testLlm!();
    if (r.ok) {
      statusEl.textContent = t("setup.ok");
      statusEl.className = "setup-test-status ok";
    } else {
      statusEl.textContent = t("setup.fail", { error: r.error ?? "" });
      statusEl.className = "setup-test-status fail";
    }
  } catch {
    statusEl.textContent = t("setup.fail", { error: "?" });
    statusEl.className = "setup-test-status fail";
  } finally {
    btn.disabled = false;
  }
}
