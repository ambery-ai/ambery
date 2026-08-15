// 托盘设置面板：schema 驱动的声明式 config UI。
// 本文件不认识 Config——只是 GET /config/schema 节点的薄渲染器，
// 加字段零成本自动出现；改值 → set_config（验证/热生效/广播都在 core）。
// 读取走 bridge 方法、写入走动作层（invoke 收口规则）。

import { createBridge, type Bridge, type ConfigSchemaNode, type ConfigSchemaResp } from "../bridge";
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
      body.appendChild(renderNode(n, resp.readOnly, pools));
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

function renderNode(
  n: ConfigNode,
  readOnly: boolean,
  pools: { system: Record<string, unknown>; user: Record<string, unknown> },
): HTMLElement {
  const row = document.createElement("div");
  row.className = "cfg-row";
  const label = n.path.split(".").slice(1).join(".") || n.path;
  const title = n.desc ? `title="${escapeHtml(n.desc)}"` : "";
  const hint = n.desc ? `<div class="desc">${escapeHtml(n.desc)}</div>` : "";

  if (n.type.kind === "map") {
    // map 节点只作分组标记（已有条目已展开为独立子节点）
    row.innerHTML = `<div class="map-head" ${title}>${escapeHtml(label)} <span class="dim">${t("menu.map-tag")}</span></div>`;
    return row;
  }

  const nameHtml = `<div class="name" ${title}>${escapeHtml(label)}</div>`;
  let control: HTMLElement;

  switch (n.type.kind) {
    case "bool": {
      const c = document.createElement("input");
      c.type = "checkbox";
      c.checked = n.value === true;
      c.onchange = () => apply(n.path, c.checked, c);
      control = c;
      break;
    }
    case "enum": {
      const c = document.createElement("select");
      for (const o of n.type.options ?? []) {
        const opt = document.createElement("option");
        opt.value = o;
        opt.textContent = o;
        if (o === n.value) opt.selected = true;
        c.appendChild(opt);
      }
      c.onchange = () => apply(n.path, c.value, c);
      control = c;
      break;
    }
    case "int":
    case "float": {
      const c = document.createElement("input");
      c.type = "number";
      c.value = String(n.value);
      if (n.type.min !== undefined) c.min = String(n.type.min);
      if (n.type.max !== undefined) c.max = String(n.type.max);
      if (n.type.kind === "float") c.step = "0.1";
      c.onchange = () =>
        apply(n.path, n.type.kind === "int" ? parseInt(c.value, 10) : parseFloat(c.value), c);
      control = c;
      break;
    }
    case "str": {
      const c = document.createElement("input");
      c.type = "text";
      c.value = String(n.value ?? "");
      c.onchange = () => apply(n.path, c.value, c);
      control = c;
      break;
    }
    default: {
      const c = document.createElement("code");
      c.className = "readonly";
      c.textContent = JSON.stringify(n.value);
      control = c;
    }
  }

  if (readOnly) (control as HTMLInputElement).disabled = true;
  const line = document.createElement("div");
  line.className = "cfg-line";
  line.innerHTML = nameHtml;
  line.appendChild(control);
  // 表情池条目（kaomoji.{system|user}.<key>.face 行）：池间原子移动按钮——
  // 单次 kaomoji 整节点写入，统一管道保证原子性与两池校验
  const m = n.path.match(/^kaomoji\.(system|user)\.([^.]+)\.face$/);
  if (m) {
    const from = m[1] as "system" | "user";
    const key = m[2];
    const btn = document.createElement("button");
    const to = from === "system" ? "user" : "system";
    btn.textContent = `→${to}`;
    btn.title = t("menu.move-title", { key, to });
    btn.disabled = readOnly;
    btn.onclick = () => {
      const next = {
        system: { ...pools.system },
        user: { ...pools.user },
      };
      const entry = next[from][key];
      delete next[from][key];
      next[to][key] = entry;
      void apply("kaomoji", next, btn);
    };
    line.appendChild(btn);
  }
  row.appendChild(line);
  if (hint) row.insertAdjacentHTML("beforeend", hint);
  return row;
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
