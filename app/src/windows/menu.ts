// 托盘设置面板（docs/config.md）：schema 驱动的声明式 config UI。
// 本文件不认识 Config——只是 GET /config/schema 节点的薄渲染器，
// 加字段零成本自动出现；改值 → set_config（验证/热生效/广播都在 core）。

import { invoke } from "@tauri-apps/api/core";

interface NodeType {
  kind: "bool" | "int" | "float" | "str" | "enum" | "map" | "other";
  min?: number;
  max?: number;
  options?: string[];
}
interface ConfigNode {
  path: string;
  type: NodeType;
  desc?: string;
  value: unknown;
}
interface SchemaResp {
  version: number;
  readOnly: boolean;
  restartRequired?: string[];
  loadError?: string | null;
  nodes: ConfigNode[];
}

export async function main() {
  document.body.innerHTML = `<div id="menu-panel">
    <div id="panel-head">
      <span>⚙ 设置</span>
      <span id="panel-head-right"><span id="panel-status"></span><button id="btn-close" title="关闭">✕</button></span>
    </div>
    <div id="panel-body">加载中…</div>
    <div id="panel-foot">
      <button id="btn-toggle">显示/隐藏</button>
      <button id="btn-quit">退出</button>
    </div>
  </div>`;
  document.getElementById("btn-toggle")!.onclick = () => invoke("toggle_pet");
  document.getElementById("btn-quit")!.onclick = () => invoke("quit_app");
  document.getElementById("btn-close")!.onclick = async () => {
    if ("__TAURI_INTERNALS__" in window) {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const { hideWindow } = await import("../tauri_runtime_actions");
      await hideWindow(getCurrentWindow());
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
    resp = await invoke<SchemaResp>("get_config_schema");
  } catch {
    body.innerHTML = `<div class="err">连不上 core</div>`;
    return;
  }
  body.innerHTML = "";
  if (resp.readOnly) {
    body.insertAdjacentHTML(
      "beforeend",
      `<div class="warn">只读降级模式：备份文件加载中，修改被拒绝（docs/config.md）</div>`,
    );
  }
  // 外部自动载入错误（docs/config.md §外部文件自动载入：保持 live Config，UI 显示具体错误）
  if (resp.loadError) {
    body.insertAdjacentHTML(
      "beforeend",
      `<div class="err">配置文件外部载入失败：${escapeHtml(resp.loadError)}（当前仍使用已加载配置；修复文件后自动重试）</div>`,
    );
  }
  // 待重启状态（docs/config.md §待重启状态：保存值与运行值不同）
  if (resp.restartRequired?.length) {
    body.insertAdjacentHTML(
      "beforeend",
      `<div class="warn">以下字段已保存，重启应用后生效：${escapeHtml(resp.restartRequired.join(", "))}</div>`,
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
    row.innerHTML = `<div class="map-head" ${title}>${escapeHtml(label)} <span class="dim">(map)</span></div>`;
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
  // 单次 kaomoji 整节点写入，统一管道保证原子性与两池校验（docs/config.md §表情池）
  const m = n.path.match(/^kaomoji\.(system|user)\.([^.]+)\.face$/);
  if (m) {
    const from = m[1] as "system" | "user";
    const key = m[2];
    const btn = document.createElement("button");
    btn.textContent = from === "system" ? "→user" : "→system";
    btn.title = `把「${key}」移到 ${from === "system" ? "user" : "system"} 池（原子移动）`;
    btn.disabled = readOnly;
    btn.onclick = () => {
      const to = from === "system" ? "user" : "system";
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
    const resp = await invoke<{ ok: boolean; restartRequired?: string[]; error?: string }>(
      "set_config",
      { path, value },
    );
    if (resp.ok) {
      const rr = resp.restartRequired as string[];
      status.textContent = rr?.length ? `⚠ ${rr.join(",")} 需重启` : "✓";
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
