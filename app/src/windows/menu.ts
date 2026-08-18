// 托盘设置面板：schema 驱动的声明式 config UI。
// 本文件不认识 Config——只是 GET /config/schema 节点的薄渲染器，
// 加字段零成本自动出现；改值 → set_config（验证/热生效/广播都在 core）。
// 读取走 bridge 方法、写入走动作层（invoke 收口规则）。

import { createBridge, type Bridge, type ConfigSchemaNode, type ConfigSchemaResp } from "../bridge";
import { renderConfigNode, renderApiKeyRow } from "../config-reflect";
import { Store } from "../store";
import { t, wireI18n } from "../i18n";
import { wireTheme } from "../theme";
import * as actions from "../tauri_runtime_actions";

type ConfigNode = ConfigSchemaNode;
type SchemaResp = ConfigSchemaResp;

/** 模块级 bridge（main 初始化后供 render/apply 使用） */
let bridge: Bridge;

export async function main() {
  bridge = await createBridge();
  const store = await Store.create(bridge);
  // 设置面板同样采用当前主题：UI 语言切换即重渲染
  wireTheme(store);
  wireI18n(store, () => void render());
  document.body.innerHTML = `<div id="menu-panel">
    <div id="panel-head">
      <span>${t("menu.title")}</span>
      <span id="panel-head-right"><span id="panel-status"></span><button id="btn-close" title="${t("menu.close-title")}">✕</button></span>
    </div>
    <div id="panel-body">${t("menu.loading")}</div>
    <div id="panel-foot">
      <button id="btn-toggle">${t("menu.toggle-pet")}</button>
      <button id="btn-quit">${t("menu.quit")}</button>
    </div>
  </div>`;
  document.getElementById("btn-toggle")!.onclick = () => void actions.togglePet();
  document.getElementById("btn-quit")!.onclick = () => void actions.quitApp();
  document.getElementById("btn-close")!.onclick = async () => {
    if ("__TAURI_INTERNALS__" in window) {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const { hideWindow, tauriWindowLike } = await import("../tauri_runtime_actions");
      await hideWindow(tauriWindowLike(getCurrentWindow()));
    }
  };
  // 外部自动载入 / 其他入口写入 → core 广播 config effect，面板重渲（错误横幅/值/pending 刷新）
  if ("__TAURI_INTERNALS__" in window) {
    const { listen } = await import("@tauri-apps/api/event");
    await listen("effect", (ev) => {
      if ((ev.payload as { kind?: string })?.kind === "config") void render();
    });
  }
  await render();
}

async function render() {
  const body = document.getElementById("panel-body")!;
  let resp: SchemaResp;
  try {
    resp = await bridge.getConfigSchema!();
  } catch {
    const err = document.createElement("div");
    err.className = "err";
    err.textContent = t("menu.offline");
    body.replaceChildren(err);
    return;
  }
  body.innerHTML = "";
  if (resp.readOnly) {
    body.insertAdjacentHTML(
      "beforeend",
      `<div class="warn">${escapeHtml(t("menu.readonly"))}</div>`,
    );
  }
  // 外部自动载入错误（保持 live Config，UI 显示具体错误）
  if (resp.loadError) {
    body.insertAdjacentHTML(
      "beforeend",
      `<div class="err">${escapeHtml(t("menu.load-error", { error: resp.loadError }))}</div>`,
    );
  }
  // 待重启状态（保存值与运行值不同）
  if (resp.restartRequired?.length) {
    body.insertAdjacentHTML(
      "beforeend",
      `<div class="warn">${escapeHtml(t("menu.restart-banner", { paths: resp.restartRequired.join(", ") }))}</div>`,
    );
  }
  // 按 path 前缀分组：顶层标量一组（__top），llm.* / kaomoji.* 各一组
  const groups = new Map<string, ConfigNode[]>();
  for (const n of resp.nodes) {
    const g = n.path.includes(".") ? n.path.split(".")[0] : "__top";
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g)!.push(n);
  }
  const ordered = [...groups.entries()].sort(([a], [b]) =>
    a === "__top" ? -1 : b === "__top" ? 1 : a.localeCompare(b),
  );
  // 表情两池当前值（map 节点携带完整 map）：供池间原子移动构造整节点写入
  const pools = {
    system: { ...((resp.nodes.find((n) => n.path === "kaomoji.system")?.value ?? {}) as Record<string, unknown>) },
    user: { ...((resp.nodes.find((n) => n.path === "kaomoji.user")?.value ?? {}) as Record<string, unknown>) },
  };
  for (const [name, nodes] of ordered) {
    if (name !== "__top") {
      body.insertAdjacentHTML("beforeend", `<div class="group">${name}</div>`);
    }
    for (const n of nodes) {
      body.appendChild(
        renderConfigNode(n, {
          readOnly: resp.readOnly,
          pools,
          applyValue: (path, value, control) => void apply(path, value, control),
        }),
      );
    }
    // provider key 输入行（形态乙）：llm.providers.<name>.api_key_env 节点后插一行。
    // 单渲染源：与引导 modal 同一 renderApiKeyRow；变更后重渲面板（config effect 广播同款）。
    // 本地端点判定用 base_url（远程 provider 清除 key 后 api_key_env 也是 null——不能用它判）。
    if (name === "llm") {
      const envNodes = nodes.filter((n) =>
        /^llm\.providers\.[^.]+\.api_key_env$/.test(n.path),
      );
      for (const n of envNodes) {
        const provider = n.path.split(".")[2];
        const baseUrlNode = nodes.find(
          (m) => m.path === `llm.providers.${provider}.base_url`,
        );
        const baseUrl = String(baseUrlNode?.value ?? "");
        const local = /localhost|127\.0\.0\.1|::1/i.test(baseUrl);
        body.appendChild(
          renderApiKeyRow(provider, n.value as string | null | undefined, local, bridge, {
            readOnly: resp.readOnly,
            onChanged: () => void render(),
          }),
        );
      }
    }
  }
  // 主题分享：导出到 config_root/themes/，按文件名导入
  if (!resp.readOnly) {
    const currentTheme = String(resp.nodes.find((n) => n.path === "theme")?.value ?? "dark");
    body.insertAdjacentHTML("beforeend", `<div class="group">${t("menu.theme-group")}</div>`);
    const share = document.createElement("div");
    share.className = "cfg-row";
    share.innerHTML = `<div class="cfg-line">
      <button type="button" data-act="export">${t("menu.theme-export")}</button>
      <input type="text" data-role="file" placeholder="${t("menu.theme-file-placeholder")}" />
      <button type="button" data-act="import">${t("menu.theme-import")}</button>
    </div>`;
    const status = document.getElementById("panel-status")!;
    share.querySelector('[data-act="export"]')!.addEventListener("click", async () => {
      status.textContent = "…";
      const r = await actions.exportTheme(currentTheme);
      status.textContent = r.ok ? t("menu.exported", { path: r.path ?? "" }) : `✗ ${r.error}`;
      status.className = r.ok ? "ok" : "err";
    });
    share.querySelector('[data-act="import"]')!.addEventListener("click", async () => {
      const file = (share.querySelector('[data-role="file"]') as HTMLInputElement).value.trim();
      if (!file) return;
      status.textContent = "…";
      const r = await actions.importTheme(file);
      status.textContent = r.ok ? t("menu.imported", { name: r.name ?? "" }) : `✗ ${r.error}`;
      status.className = r.ok ? "ok" : "err";
      if (r.ok) setTimeout(() => void render(), 300);
    });
    body.appendChild(share);
  }
}


async function apply(path: string, value: unknown, control: HTMLElement) {
  const status = document.getElementById("panel-status")!;
  status.textContent = "…";
  try {
    const resp = await bridge.setConfig!(path, value);
    if (resp.ok) {
      const rr = resp.restartRequired as string[];
      status.textContent = rr?.length ? t("menu.need-restart", { paths: rr.join(",") }) : "✓";
      status.className = rr?.length ? "warn" : "ok";
      control.classList.remove("bad");
      setTimeout(() => void render(), 300); // 热刷新（值归一后重渲染）
    } else {
      status.textContent = `✗ ${resp.error}`;
      status.className = "err";
      control.classList.add("bad");
    }
  } catch (e) {
    status.textContent = `✗ ${e}`;
    status.className = "err";
  }
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
}
