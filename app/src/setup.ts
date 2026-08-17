// LLM 首启配置引导 modal（docs/llm-setup.md）：
// 从 Chat 打开；反射渲染 llm 相关 schema 节点（与设置面板同一 get_config_schema 投影），
// 非手写表单；提供 provider 选择、key 状态检测（test_llm 结果当状态）、测试连通。
// menu 本身不变——引导不在 menu 里，也不改变 menu 行为。

import type { Bridge, ConfigSchemaNode } from "./bridge";
import { t } from "./i18n";

/** 打开引导 modal（overlay + 面板）；返回关闭函数（宿主可在 ChatPanel 关闭时一并收起） */
export function openSetupModal(bridge: Bridge): () => void {
  const overlay = document.createElement("div");
  overlay.className = "setup-overlay";

  const panel = document.createElement("div");
  panel.className = "setup-modal";

  const head = document.createElement("div");
  head.className = "setup-head";
  const title = document.createElement("span");
  title.textContent = t("setup.title");
  const close = document.createElement("button");
  close.className = "setup-close";
  close.textContent = "×";
  head.append(title, close);

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

  // active 选择（enum select）
  const activeNode = llmNodes.find((n) => n.path === "llm.active");
  if (activeNode) body.appendChild(nodeRow(activeNode, bridge));

  // provider 字段（当前 active 对应的 providers.<name>.* 节点）
  const active = String(activeNode?.value ?? "");
  const provPrefix = `llm.providers.${active}.`;
  if (active && active !== "unconfigured" && active !== "debug") {
    const provNodes = llmNodes.filter((n) => n.path.startsWith(provPrefix));
    for (const n of provNodes) body.appendChild(nodeRow(n, bridge));
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

/** 单节点反射渲染（简化版 renderNode：只处理 llm 用到的 enum/str/int，写走 setConfig） */
function nodeRow(n: ConfigSchemaNode, bridge: Bridge): HTMLElement {
  const row = document.createElement("div");
  row.className = "cfg-row";
  const label = n.path.split(".").slice(1).join(".") || n.path;
  const name = document.createElement("div");
  name.className = "name";
  name.textContent = label;
  if (n.desc) name.title = n.desc;
  row.appendChild(name);

  let control: HTMLElement;
  switch (n.type.kind) {
    case "enum": {
      const c = document.createElement("select");
      for (const o of n.type.options ?? []) {
        const opt = document.createElement("option");
        opt.value = o;
        opt.textContent = o;
        if (o === n.value) opt.selected = true;
        c.appendChild(opt);
      }
      c.onchange = () => void bridge.setConfig!(n.path, c.value).then(() => void render(bridge, row.parentElement!));
      control = c;
      break;
    }
    case "int":
    case "float": {
      const c = document.createElement("input");
      c.type = "number";
      c.value = String(n.value);
      c.onchange = () =>
        void bridge.setConfig!(
          n.path,
          n.type.kind === "int" ? parseInt(c.value, 10) : parseFloat(c.value),
        );
      control = c;
      break;
    }
    default: {
      // str（含 api_key_env 变量名）与只读值：文本输入/只读展示
      if (n.type.kind === "str") {
        const c = document.createElement("input");
        c.type = "text";
        c.value = String(n.value ?? "");
        c.onchange = () => void bridge.setConfig!(n.path, c.value);
        control = c;
      } else {
        const c = document.createElement("code");
        c.className = "readonly";
        c.textContent = JSON.stringify(n.value);
        control = c;
      }
    }
  }
  row.appendChild(control);
  return row;
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
