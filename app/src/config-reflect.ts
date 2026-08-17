// 反射渲染共享模块（docs/config.md §Reflection / docs/llm-setup.md）：
// 单个 Config schema 节点的机械渲染（enum/bool/int/str/map/只读/desc/表情池移动）。
// 单一语义源：menu 设置面板与 LLM 引导 modal 共用同一渲染，不各自实现。
// 加字段零成本自动出现；写值经调用方注入的 applyValue（统一修改管道在 core）。

import type { ConfigSchemaNode } from "./bridge";
import { t } from "./i18n";

export type ConfigNode = ConfigSchemaNode;

export interface RenderNodeOpts {
  /** 只读降级模式：所有控件禁用 */
  readOnly: boolean;
  /** 表情两池当前值（kaomoji map 节点携带完整 map）——池间原子移动按钮用 */
  pools?: { system: Record<string, unknown>; user: Record<string, unknown> };
  /** 写值回调（menu 走 #panel-status + render；setup 走自身状态 + 重渲染） */
  applyValue: (path: string, value: unknown, control: HTMLElement) => void;
}

/** 渲染一个 schema 节点为配置行（控件 + 名称 + desc） */
export function renderConfigNode(n: ConfigNode, opts: RenderNodeOpts): HTMLElement {
  const { readOnly, pools } = opts;
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
      c.onchange = () => opts.applyValue(n.path, c.checked, c);
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
      c.onchange = () => opts.applyValue(n.path, c.value, c);
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
        opts.applyValue(
          n.path,
          n.type.kind === "int" ? parseInt(c.value, 10) : parseFloat(c.value),
          c,
        );
      control = c;
      break;
    }
    case "str": {
      const c = document.createElement("input");
      c.type = "text";
      c.value = String(n.value ?? "");
      c.onchange = () => opts.applyValue(n.path, c.value, c);
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
  if (m && pools) {
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
      opts.applyValue("kaomoji", next, btn);
    };
    line.appendChild(btn);
  }
  row.appendChild(line);
  if (hint) row.insertAdjacentHTML("beforeend", hint);
  return row;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
}
