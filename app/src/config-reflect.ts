// 反射渲染共享模块（docs/config.md §Reflection / docs/llm-setup.md）：
// 单个 Config schema 节点的机械渲染（enum/bool/int/str/map/只读/desc/表情池移动）。
// 单一语义源：menu 设置面板与 LLM 引导 modal 共用同一渲染，不各自实现。
// 加字段零成本自动出现；写值经调用方注入的 applyValue（统一修改管道在 core）。
// 另有 renderApiKeyRow：provider key 输入行（形态乙——写应用级 env 文件，
// config.json 永不存 key；menu 与引导 modal 同一渲染源）。

import type { Bridge, ConfigSchemaNode } from "./bridge";
import { createCustomSelect, type CustomSelectOpts } from "./components/custom-select";
import { t } from "./i18n";

export type ConfigNode = ConfigSchemaNode;

export interface RenderNodeOpts {
  /** 只读降级模式：所有控件禁用 */
  readOnly: boolean;
  /** 表情两池当前值（kaomoji map 节点携带完整 map）——池间原子移动按钮用 */
  pools?: { system: Record<string, unknown>; user: Record<string, unknown> };
  /** 写值回调（menu 走 #panel-status + render；setup 走自身状态 + 重渲染） */
  applyValue: (path: string, value: unknown, control: HTMLElement) => void;
  /** enum（custom-select）控件扩展：按 path 注入 addMode 等（llm.active 加 provider） */
  enumAddons?: Record<string, Pick<CustomSelectOpts, "addMode">>;
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

  // name 的 hover 显示完整字段路径（长路径在面板内被截断，hover 看全）；
  // desc 已作为行下 hint 可见，hover 不再重复 desc
  const nameHtml = `<div class="name" title="${escapeHtml(n.path)}">${escapeHtml(label)}</div>`;
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
      control = createCustomSelect({
        options: n.type.options ?? [],
        value: String(n.value ?? ""),
        readOnly,
        onChange: (v) => opts.applyValue(n.path, v, control),
        ...opts.enumAddons?.[n.path],
      });
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

export interface ApiKeyRowOpts {
  /** 只读降级模式：输入与按钮禁用 */
  readOnly: boolean;
  /** 保存/清除成功后回调（宿主刷新：引导 modal 重跑 test_llm；menu 刷状态） */
  onChanged: () => void;
}

/**
 * provider key 输入行（形态乙）：
 * - `local` = 本地端点（base_url 指向本机）→ 无需 key，显示提示。
 * - 否则渲染密码框 + 保存：状态按 getApiKeyStatus 判定
 *   （未设置 → 警示占位"请输入 API key"；已设置 → 占位"留空则不改动" + 来源提示）。
 * - 保存：留空 = 不改动；填值 = upsert（覆盖）。成功后刷新状态并回调 onChanged
 *   （引导 modal 宿主自动重跑 test_llm；menu 刷状态）。
 * - 无清除：key 只增不删，覆盖即更新。
 */
export function renderApiKeyRow(
  provider: string,
  apiKeyEnv: string | null | undefined,
  local: boolean,
  bridge: Bridge,
  opts: ApiKeyRowOpts,
): HTMLElement {
  const row = document.createElement("div");
  row.className = "cfg-row api-key-row";
  row.dataset.provider = provider;

  if (local) {
    row.innerHTML = `<div class="cfg-line"><div class="name">${escapeHtml(
      provider,
    )} · key</div><span class="dim">${t("setup.key-not-needed")}</span></div>`;
    return row;
  }

  const envName = apiKeyEnv ?? "";
  const nameHtml = `<div class="name" title="${escapeHtml(envName)}">${escapeHtml(
    provider,
  )} · key</div>`;
  const input = document.createElement("input");
  input.type = "password";
  input.className = "api-key-input";
  input.placeholder = t("setup.key-placeholder-unset");
  input.disabled = opts.readOnly;

  const save = document.createElement("button");
  save.type = "button";
  save.className = "api-key-save";
  save.textContent = t("setup.key-save");
  save.disabled = opts.readOnly;

  const line = document.createElement("div");
  line.className = "cfg-line";
  line.innerHTML = nameHtml;
  line.appendChild(input);
  line.appendChild(save);
  row.appendChild(line);

  const hint = document.createElement("div");
  hint.className = "desc api-key-hint";
  row.appendChild(hint);

  const refresh = async () => {
    let source: string | null = null;
    let isSet = false;
    try {
      const r = await bridge.getApiKeyStatus!(provider);
      isSet = r.set;
      source = r.source;
    } catch {
      /* 状态查询失败 = 按未设置显示（保存动作仍可试） */
    }
    if (isSet) {
      input.placeholder = t("setup.key-placeholder-set");
      hint.textContent = t("setup.key-set-hint", { source: source ?? "" });
      hint.className = "desc api-key-hint ok";
    } else {
      input.placeholder = t("setup.key-placeholder-unset");
      hint.textContent = t("setup.key-unset-hint");
      hint.className = "desc api-key-hint warn";
    }
  };

  const saving = (busy: boolean) => {
    input.disabled = opts.readOnly || busy;
    save.disabled = opts.readOnly || busy;
  };

  save.addEventListener("click", async () => {
    const value = input.value.trim();
    if (value === "") return; // 留空 = 不改动
    saving(true);
    try {
      const r = await bridge.setApiKey!(provider, value);
      if (!r.ok) {
        hint.textContent = t("setup.key-save-fail", { error: r.error ?? "" });
        hint.className = "desc api-key-hint warn";
      } else {
        input.value = "";
        await refresh();
        opts.onChanged(); // 引导 modal：自动重跑连通测试
      }
    } catch {
      hint.textContent = t("setup.key-save-fail", { error: "?" });
      hint.className = "desc api-key-hint warn";
    } finally {
      saving(false);
    }
  });

  void refresh();
  return row;
}
