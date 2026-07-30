// Component（concepts §5，设计 docs/components.md）：
// ペット经 call_component 调用，以 View 为中心向合适方位偏移弹出；
// 用户交互写 Event Buffer（bridge.pushEvent），不写 Queue user role。
//
// multi-window 模式（windowed=true）：卡片在窗口内自然流式布局，窗口外部定位；
// 浏览器/maximized 模式（默认）：卡片 position:fixed 在 View 锚点周围弹出。

import type { Bridge, ComponentSpec, Direction } from "../bridge";
import type { PositioningEngine } from "../positioning/engine";
import { directionFromName, type Point } from "../positioning/types";

const GAP = 12;
const VIEW_RADIUS_X = 36;
const VIEW_RADIUS_Y = 20;

type Anchor = () => { x: number; y: number };

export class ComponentManager {
  private layer: HTMLDivElement;
  private cards = new Map<string, HTMLDivElement>();

  constructor(
    mount: HTMLElement,
    private bridge: Bridge,
    private anchor: Anchor,
    /** multi-window 模式：跳过 DOM 定位，卡片在窗口内自然流式布局 */
    public windowed = false,
    /** browser 模式：卡片纳入 engine 语义（place 注册 / follow 重排，docs/window-follow.md） */
    private engine?: PositioningEngine,
  ) {
    this.layer = document.createElement("div");
    this.layer.id = "components";
    if (windowed) mount.classList.add("cards-mode");
    mount.appendChild(this.layer);
    bridge.onRenderComponent((spec) => this.render(spec));
  }

  render(spec: ComponentSpec) {
    // 同 id 重复调用 = toggle 关闭（docs/components.md）
    const existing = this.cards.get(spec.id);
    if (existing) {
      existing.remove();
      this.cards.delete(spec.id);
      this.engine?.remove(`card-${spec.id}`);
      return;
    }
    const card = this.buildCard(spec);
    if (spec.direction) card.dataset.direction = spec.direction;
    this.layer.appendChild(card);
    this.cards.set(spec.id, card);
    if (!this.windowed) this.place(card, spec.direction ?? "auto");
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
      this.bridge.pushEvent(`用户关闭了 ${spec.type}「${title.textContent}」`);
      card.remove();
      this.cards.delete(spec.id);
    });
    header.append(title, close);
    const body = this.buildBody(spec);
    card.append(header, body);
    // #20 高度 cap（单源：Tauri/browser 共用渲染路径）——body 限高 = 屏高×0.5 − header，
    // 超长走 .cmp-body 滚动（styles.css overflow-y:auto），内容不截断
    const cap = Math.max(window.screen.availHeight * 0.5 - (header.offsetHeight || 40), 120);
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
        copy.textContent = "复制";
        copy.addEventListener("click", () => {
          void navigator.clipboard?.writeText(spec.text ?? "");
          this.bridge.pushEvent(`用户复制了 text_card「${spec.title}」的内容`);
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
          this.bridge.pushEvent(`用户点击 quick_jump 跳转到「${spec.target}」`);
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
          summary.textContent = "展开 diff";
          const pre = document.createElement("pre");
          pre.textContent = spec.diff;
          details.append(summary, pre);
          details.addEventListener("toggle", () => {
            if (details.open)
              this.bridge.pushEvent(`用户展开了 git_display「${spec.title}」的 diff`);
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
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
    svg.setAttribute("width", String(W));
    svg.setAttribute("height", String(H));

    const { kind, labels, series } = spec.chart;
    const flat = series.flatMap((s) => s.data);
    const max = Math.max(...flat, 1);
    const colors = ["#89b4fa", "#a6e3a1", "#f9e2af", "#f38ba8"];

    if (kind === "line") {
      series.forEach((s, si) => {
        const step = W / Math.max(s.data.length - 1, 1);
        const points = s.data
          .map((v, i) => `${i * step},${H - (v / max) * (H - 16)}`)
          .join(" ");
        const pl = document.createElementNS(svg.namespaceURI, "polyline");
        pl.setAttribute("points", points);
        pl.setAttribute("fill", "none");
        pl.setAttribute("stroke", colors[si % colors.length]);
        pl.setAttribute("stroke-width", "2");
        svg.append(pl);
        s.data.forEach((v, i) => {
          const c = document.createElementNS(svg.namespaceURI, "circle");
          c.setAttribute("cx", String(i * step));
          c.setAttribute("cy", String(H - (v / max) * (H - 16)));
          c.setAttribute("r", "3");
          c.setAttribute("fill", colors[si % colors.length]);
          const tip = document.createElementNS(svg.namespaceURI, "title");
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
          const r = document.createElementNS(svg.namespaceURI, "rect");
          const h = (v / max) * (H - 16);
          r.setAttribute("x", String((i * series.length + si) * bw));
          r.setAttribute("y", String(H - h));
          r.setAttribute("width", String(bw - 2));
          r.setAttribute("height", String(h));
          r.setAttribute("fill", colors[si % colors.length]);
          const tip = document.createElementNS(svg.namespaceURI, "title");
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
        const path = document.createElementNS(svg.namespaceURI, "path");
        path.setAttribute(
          "d",
          `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${large} 1 ${x2} ${y2} Z`,
        );
        path.setAttribute("fill", colors[i % colors.length]);
        const tip = document.createElementNS(svg.namespaceURI, "title");
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
        this.bridge.pushEvent(
          `用户${cb.checked ? "勾选" : "取消勾选"}了 todobox 条目「${text}」`,
        );
      });
      const span = document.createElement("span");
      span.textContent = text;
      li.append(cb, span);
      ul.append(li);
    };
    for (const item of spec.items) addItem(item.text, item.done);

    const input = document.createElement("input");
    input.placeholder = "新增条目…";
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && input.value.trim()) {
        const text = input.value.trim();
        addItem(text, false);
        this.bridge.pushEvent(`用户新增 todobox 条目「${text}」`);
        input.value = "";
      }
    });
    wrap.append(ul, input);
    return wrap;
  }

  /** 方位几何（docs/components.md）：engine 优先（单源语义），无 engine 时本地锚点偏移。
   *  不做 clamp（docs/window-follow.md §出屏与重叠：不压人 > 完全可见） */
  private place(card: HTMLDivElement, dir: Direction) {
    const d =
      dir === "auto"
        ? this.autoDirection()
        : dir;
    const cw = card.offsetWidth || 260;
    const ch = card.offsetHeight || 140;
    if (this.engine) {
      // engine 语义：注册进 occupied（跟随/恢复由 engine 管）
      const specId = card.dataset.id!;
      const edir = directionFromName(d) ?? 7; // 默认 sse（16 方位枚举值）
      const pos = this.engine.place({ id: `card-${specId}`, width: cw, height: ch }, edir);
      card.style.left = `${pos.x - cw / 2}px`;
      card.style.top = `${pos.y - ch / 2}px`;
      return;
    }
    const { x, y } = this.anchor();
    let left = x - cw / 2;
    let top = y - ch / 2;
    if (d.includes("left")) left = x - VIEW_RADIUS_X - GAP - cw;
    if (d.includes("right")) left = x + VIEW_RADIUS_X + GAP;
    if (d.includes("top")) top = y - VIEW_RADIUS_Y - GAP - ch;
    if (d.includes("bottom")) top = y + VIEW_RADIUS_Y + GAP;
    if (d === "left" || d === "right") top = y - ch / 2;
    if (d === "top" || d === "bottom") left = x - cw / 2;
    card.style.left = `${left}px`;
    card.style.top = `${top}px`;
  }

  /** auto = 屏幕剩余空间最大的方位（以 View 中心划分四象限比较） */
  private autoDirection(): Exclude<Direction, "auto"> {
    const { x, y } = this.anchor();
    const spaces: [Exclude<Direction, "auto">, number][] = [
      ["left", x],
      ["right", window.innerWidth - x],
      ["top", y],
      ["bottom", window.innerHeight - y],
    ];
    spaces.sort((a, b) => b[1] - a[1]);
    return spaces[0][0];
  }
}

