// Component（concepts §5，设计 docs/components.md）：
// pet 经 call_component 调用，以 View 为中心向合适方位偏移弹出；
// 用户交互写 Event Buffer（bridge.pushEvent），不写 Queue user role。
//
// multi-window 模式（windowed=true）：卡片在窗口内自然流式布局，窗口外部定位；
// 浏览器/maximized 模式（默认）：卡片 position:fixed 在 View 锚点周围弹出。

import type { Bridge, ComponentSpec, Direction } from "../bridge";
import { attachDrag } from "../drag";
import { onLanguageChange, t } from "../i18n";
import type { PositioningEngine } from "../positioning/engine";
import { directionFromName, type Point } from "../positioning/types";

const GAP = 12;
const VIEW_RADIUS_X = 36;
const VIEW_RADIUS_Y = 20;

type Anchor = () => { x: number; y: number };

export class ComponentManager {
  private layer: HTMLDivElement;
  private cards = new Map<string, HTMLDivElement>();

  /** 屏幕逻辑高度缓存（#20 高度 cap 取口） */
  private screenH: number | null = null;

  constructor(
    mount: HTMLElement,
    private bridge: Bridge,
    private anchor: Anchor,
    /** multi-window 模式：跳过 DOM 定位，卡片在窗口内自然流式布局 */
    public windowed = false,
    /** browser 模式：卡片纳入 engine 语义（place 注册 / follow 重排，docs/window-follow.md） */
    private engine?: PositioningEngine,
    /** 屏幕高度提供方（docs/window-follow.md §显示器几何：消费通道统一走 adapter 取口；
     *  缺省回落 window.screen——browser 单屏调试可接受） */
    private screenHeightProvider?: () => Promise<number>,
  ) {
    void this.screenHeightProvider?.().then((h) => (this.screenH = h));
    this.layer = document.createElement("div");
    this.layer.id = "components";
    if (windowed) mount.classList.add("cards-mode");
    mount.appendChild(this.layer);
    // windowed（card 窗口）：不订阅全局 render 流——本窗只渲染 card:spec 定向事件
    // （#25 根因 A：全局广播使每个 card 窗口渲染所有卡 → 一窗多卡堆叠 + 复活已关闭卡）
    if (!windowed) {
      bridge.onRenderComponent((spec) => this.render(spec));
      // 显式关闭（持续管理协议：agent close action）——browser 模式无窗口层，
      // close_component 由本管理器直接落 DOM（与 render 订阅同 guard）
      bridge.onCloseComponent?.((id) => this.closeById(id));
    }
    // UI 语言切换：原地重贴 chrome 文案（docs/i18n.md；不重建 DOM——todobox 勾选态
    // 只在 DOM，重建会丢交互状态）
    onLanguageChange(() => this.relabel());
  }

  /** chrome 文案原地重贴（复制按钮 / diff 摘要 / todo 输入占位；卡片内容不翻译） */
  private relabel() {
    for (const card of this.cards.values()) {
      const body = card.querySelector(".cmp-body");
      if (!body) continue;
      const type = card.dataset.type;
      if (type === "text_card") {
        const btn = body.querySelector(":scope > button");
        if (btn) btn.textContent = t("card.copy");
      } else if (type === "git_display") {
        const summary = body.querySelector("details > summary");
        if (summary) summary.textContent = t("card.expand-diff");
      } else if (type === "todobox") {
        const input = body.querySelector(":scope > input");
        if (input) (input as HTMLInputElement).placeholder = t("card.todo-placeholder");
      }
    }
  }

  render(spec: ComponentSpec) {
    // 持续管理协议（docs/components.md）：同 id = **原地更新**（重建内容，不销毁窗口/DOM）；
    // 关闭只走显式 close（closeById / 用户 ×），render 永不关
    const existing = this.cards.get(spec.id);
    if (existing) {
      const updated = this.buildCard(spec);
      existing.replaceChildren(...updated.childNodes);
      if (spec.direction) existing.dataset.direction = spec.direction;
      return;
    }
    void this.screenHeightProvider?.().then((h) => (this.screenH = h));
    const card = this.buildCard(spec);
    if (spec.direction) card.dataset.direction = spec.direction;
    // browser：DOM 拖拽（docs/window-follow.md §拖拽回写；Tauri 走 OS startDragging）
    if (this.engine) {
      const specId = spec.id;
      attachDrag(card, ".cmp-header", ".cmp-close", (center) =>
        this.engine!.updateCenter(`card-${specId}`, center),
      );
    }
    this.layer.appendChild(card);
    this.cards.set(spec.id, card);
    if (!this.windowed) this.place(card, spec.direction ?? "auto");
  }

  /** 显式关闭（持续管理协议：agent close action / 用户 ×） */
  closeById(id: string) {
    const existing = this.cards.get(id);
    if (!existing) return;
    existing.remove();
    this.cards.delete(id);
    this.engine?.remove(`card-${id}`);
  }

  /** pet 移动后重排（browser）：按 restorePositions 的结果重定位卡片 DOM */
  followRestore(restored: { id: string; center: Point }[]) {
    for (const r of restored) {
      if (!r.id.startsWith("card-")) continue;
      const card = this.cards.get(r.id.slice(5));
      if (!card) continue;
      card.style.left = `${r.center.x - card.offsetWidth / 2}px`;
      card.style.top = `${r.center.y - card.offsetHeight / 2}px`;
    }
  }

  /** 系统藏/恢复（pet 拖动，docs/window-follow.md：整层，不逐卡改状态） */
  systemHideAll() {
    this.layer.hidden = true;
  }
  systemShowAll() {
    this.layer.hidden = false;
  }

  /** Shelf 显隐（browser）：user_closed = 只藏不销（DOM 原位即布局记忆；dismiss 走 closeById） */
  setHidden(id: string, hidden: boolean) {
    const el = this.cards.get(id);
    if (el) el.style.display = hidden ? "none" : "";
  }

  private buildCard(spec: ComponentSpec): HTMLDivElement {
    const card = document.createElement("div");
    card.className = "component";
    card.dataset.type = spec.type;
    card.dataset.id = spec.id;

    const header = document.createElement("div");
    header.className = "cmp-header";
    const title = document.createElement("span");
    title.className = "cmp-title";
    title.textContent = "title" in spec ? spec.title : spec.label;
    const close = document.createElement("button");
    close.className = "cmp-close";
    close.textContent = "×";
    close.addEventListener("click", () => {
      // closed_by_user 双行生命周期事件（docs/components.md）：结构化事实，
      // 文本由 core 按 Harness 语言现写（lifecycle 单源，docs/i18n.md）
      this.bridge.pushEvent({ action: "dismiss", cardId: spec.id });
      this.closeById(spec.id);
    });
    header.append(title, close);
    const body = this.buildBody(spec);
    card.append(header, body);
    // #20 高度 cap（单源：Tauri/browser 共用渲染路径）——body 限高 = 屏高×0.5 − header，
    // 屏高经 adapter 取口（docs/window-follow.md §显示器几何：分支不各自 window.screen），
    // 超长走 .cmp-body 滚动（styles.css overflow-y:auto），内容不截断
    const screenH = this.screenH ?? window.screen.availHeight;
    const cap = Math.max(screenH * 0.5 - (header.offsetHeight || 40), 120);
    body.style.maxHeight = `${cap}px`;
    return card;
  }

  private buildBody(spec: ComponentSpec): HTMLElement {
    const body = document.createElement("div");
    body.className = "cmp-body";
    switch (spec.type) {
      case "text_card": {
        const p = document.createElement("p");
        p.textContent = spec.text;
        const copy = document.createElement("button");
        copy.textContent = t("card.copy");
        copy.addEventListener("click", () => {
          void navigator.clipboard?.writeText(spec.text ?? "");
          this.bridge.pushEvent({ action: "copy", cardType: "text_card", title: spec.title });
        });
        body.append(p, copy);
        break;
      }
      case "quick_jump": {
        const btn = document.createElement("button");
        btn.className = "cmp-jump";
        btn.textContent = `→ ${spec.target}`;
        btn.addEventListener("click", () => {
          // 真实切换 WT 标签页待 C# sidecar 接入（docs/components.md）
          this.bridge.pushEvent({ action: "jump", cardType: "quick_jump", target: spec.target });
        });
        body.append(btn);
        break;
      }
      case "git_display": {
        const ul = document.createElement("ul");
        ul.className = "cmp-git-log";
        for (const e of spec.entries) {
          const li = document.createElement("li");
          li.textContent = `${e.hash} ${e.msg} (${e.time})`;
          ul.append(li);
        }
        body.append(ul);
        if (spec.diff) {
          const details = document.createElement("details");
          const summary = document.createElement("summary");
          summary.textContent = t("card.expand-diff");
          const pre = document.createElement("pre");
          pre.textContent = spec.diff;
          details.append(summary, pre);
          details.addEventListener("toggle", () => {
            if (details.open)
              this.bridge.pushEvent({ action: "expand_diff", cardType: "git_display", title: spec.title });
          });
          body.append(details);
        }
        break;
      }
      case "data_chart": {
        body.append(this.buildChart(spec));
        break;
      }
      case "todobox": {
        body.append(this.buildTodo(spec));
        break;
      }
    }
    return body;
  }

  private buildChart(
    spec: Extract<ComponentSpec, { type: "data_chart" }>,
  ): HTMLElement {
    const wrap = document.createElement("div");
    wrap.className = "cmp-chart";
    const W = 220;
    const H = 120;
    const NS = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(NS, "svg");
    svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
    svg.setAttribute("width", String(W));
    svg.setAttribute("height", String(H));

    const { kind, labels, series } = spec.chart;
    const flat = series.flatMap((s) => s.data);
    const max = Math.max(...flat, 1);
    // 调色板走设计 token（styles.css --ov-chart-*）：SVG paint 属性不认 var()，
    // 经 style 属性引用（fill/stroke 是 CSS 属性，var() 在文档内解析）
    const colors = ["var(--ov-chart-1)", "var(--ov-chart-2)", "var(--ov-chart-3)", "var(--ov-chart-4)"];

    if (kind === "line") {
      series.forEach((s, si) => {
        const step = W / Math.max(s.data.length - 1, 1);
        const points = s.data
          .map((v, i) => `${i * step},${H - (v / max) * (H - 16)}`)
          .join(" ");
        const pl = document.createElementNS(NS, "polyline");
        pl.setAttribute("points", points);
        pl.style.fill = "none";
        pl.style.stroke = colors[si % colors.length];
        pl.setAttribute("stroke-width", "2");
        svg.append(pl);
        s.data.forEach((v, i) => {
          const c = document.createElementNS(NS, "circle");
          c.setAttribute("cx", String(i * step));
          c.setAttribute("cy", String(H - (v / max) * (H - 16)));
          c.setAttribute("r", "3");
          c.style.fill = colors[si % colors.length];
          const tip = document.createElementNS(NS, "title");
          tip.textContent = `${s.name} ${labels[i] ?? i}: ${v}`;
          c.append(tip);
          svg.append(c);
        });
      });
    } else if (kind === "bar") {
      const groups = series[0]?.data.length ?? 0;
      const bw = W / Math.max(groups * series.length, 1);
      series.forEach((s, si) => {
        s.data.forEach((v, i) => {
          const r = document.createElementNS(NS, "rect");
          const h = (v / max) * (H - 16);
          r.setAttribute("x", String((i * series.length + si) * bw));
          r.setAttribute("y", String(H - h));
          r.setAttribute("width", String(bw - 2));
          r.setAttribute("height", String(h));
          r.style.fill = colors[si % colors.length];
          const tip = document.createElementNS(NS, "title");
          tip.textContent = `${s.name} ${labels[i] ?? i}: ${v}`;
          r.append(tip);
          svg.append(r);
        });
      });
    } else {
      // pie
      const total = series[0]?.data.reduce((a, b) => a + b, 0) || 1;
      let angle = -Math.PI / 2;
      const cx = W / 2;
      const cy = H / 2;
      const r = Math.min(W, H) / 2 - 6;
      series[0]?.data.forEach((v, i) => {
        const sweep = (v / total) * Math.PI * 2;
        const large = sweep > Math.PI ? 1 : 0;
        const x1 = cx + r * Math.cos(angle);
        const y1 = cy + r * Math.sin(angle);
        const x2 = cx + r * Math.cos(angle + sweep);
        const y2 = cy + r * Math.sin(angle + sweep);
        const path = document.createElementNS(NS, "path");
        path.setAttribute(
          "d",
          `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${large} 1 ${x2} ${y2} Z`,
        );
        path.style.fill = colors[i % colors.length];
        const tip = document.createElementNS(NS, "title");
        tip.textContent = `${labels[i] ?? i}: ${v}`;
        path.append(tip);
        svg.append(path);
        angle += sweep;
      });
    }
    wrap.append(svg);
    return wrap;
  }

  private buildTodo(
    spec: Extract<ComponentSpec, { type: "todobox" }>,
  ): HTMLElement {
    const wrap = document.createElement("div");
    wrap.className = "cmp-todo";
    const ul = document.createElement("ul");

    const addItem = (text: string, done: boolean) => {
      const li = document.createElement("li");
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.checked = done;
      cb.addEventListener("change", () => {
        // 双载荷（docs/harness.md）：该 card 当前完整 items 快照（同 card 去重合并）；
        // 文本由 core 现写（lifecycle 单源）
        this.bridge.pushEvent({
          action: "todo_toggle",
          cardId: spec.id,
          cardType: "todobox",
          text,
          checked: cb.checked,
          state: { id: spec.id, type: "todobox", items: collectItems() },
        });
      });
      const span = document.createElement("span");
      span.textContent = text;
      li.append(cb, span);
      ul.append(li);
    };
    const collectItems = (): { text: string; done: boolean }[] =>
      [...ul.querySelectorAll("li")].map((li) => ({
        text: (li.querySelector("span")?.textContent ?? "").trim(),
        done: (li.querySelector("input") as HTMLInputElement | null)?.checked ?? false,
      }));
    for (const item of spec.items) addItem(item.text, item.done);

    const input = document.createElement("input");
    input.placeholder = t("card.todo-placeholder");
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && input.value.trim()) {
        const text = input.value.trim();
        addItem(text, false);
        this.bridge.pushEvent({
          action: "todo_add",
          cardId: spec.id,
          cardType: "todobox",
          text,
          state: { id: spec.id, type: "todobox", items: collectItems() },
        });
        input.value = "";
      }
    });
    wrap.append(ul, input);
    return wrap;
  }

  /** 方位几何（docs/components.md）：engine 优先（单源语义），无 engine 时本地锚点偏移。
   *  auto 在两路径各自现算最大剩余空间（engine 与本地四象限同一语义）。
   *  不做 clamp（docs/window-follow.md §出屏与重叠：不压人 > 完全可见） */
  private place(card: HTMLDivElement, dir: Direction) {
    const cw = card.offsetWidth || 260;
    const ch = card.offsetHeight || 140;
    if (this.engine) {
      // engine 语义：注册进 occupied（跟随/恢复由 engine 管）；auto 透传（engine 现算最大剩余空间）
      const specId = card.dataset.id!;
      const edir = dir === "auto" ? "auto" as const : directionFromName(dir) ?? 7; // 默认 sse（16 方位枚举值）
      const pos = this.engine.place({ id: `card-${specId}`, width: cw, height: ch }, edir);
      card.style.left = `${pos.x - cw / 2}px`;
      card.style.top = `${pos.y - ch / 2}px`;
      return;
    }
    const d =
      dir === "auto"
        ? this.autoDirection()
        : dir;
    const { x, y } = this.anchor();
    // 方位几何（docs/components.md）：锚点 ± (View 半径 + 12px 间距 + 卡片半尺寸)；
    // 斜方位 = 两轴分别偏移
    const ox = VIEW_RADIUS_X + GAP + cw / 2;
    const oy = VIEW_RADIUS_Y + GAP + ch / 2;
    const cx = x + (d.includes("e") ? ox : d.includes("w") ? -ox : 0);
    const cy = y + (d.includes("s") ? oy : d.includes("n") ? -oy : 0);
    card.style.left = `${cx - cw / 2}px`;
    card.style.top = `${cy - ch / 2}px`;
  }

  /** auto = 屏幕剩余空间最大的方位（以 View 中心划分四象限比较） */
  private autoDirection(): Exclude<Direction, "auto"> {
    const { x, y } = this.anchor();
    const spaces: [Exclude<Direction, "auto">, number][] = [
      ["w", x],
      ["e", window.innerWidth - x],
      ["n", y],
      ["s", window.innerHeight - y],
    ];
    spaces.sort((a, b) => b[1] - a[1]);
    return spaces[0][0];
  }
}

