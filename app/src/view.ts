// View（concepts §3，设计 docs/view.md）：横向椭圆浮动窗口，窗内仅颜文字。
// 状态机：floating ⇄ docked(edge)。浏览器测试模式用 fixed DOM 元素模拟窗口。

import type { Motion } from "./bridge";

export type Edge = "top" | "right" | "bottom" | "left";

type ViewState = { mode: "floating" } | { mode: "docked"; edge: Edge };

const DOCK_MARGIN = 8;

export class View {
  readonly el: HTMLDivElement;
  private faceEl: HTMLSpanElement;
  private state: ViewState = { mode: "floating" };
  private drag: { dx: number; dy: number } | null = null;

  constructor(mount: HTMLElement) {
    this.el = document.createElement("div");
    this.el.id = "view";
    this.faceEl = document.createElement("span");
    this.faceEl.id = "face";
    this.el.appendChild(this.faceEl);
    mount.appendChild(this.el);
    this.el.style.left = "120px";
    this.el.style.top = "120px";

    this.el.addEventListener("pointerdown", this.onPointerDown);
    window.addEventListener("pointermove", this.onPointerMove);
    window.addEventListener("pointerup", this.onPointerUp);
    this.el.addEventListener("contextmenu", this.onContextMenu);
  }

  setExpression(e: { face: string; motion: Motion }) {
    this.faceEl.textContent = e.face;
    this.el.dataset.motion = e.motion;
  }

  center() {
    const r = this.el.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  }

  isDocked() {
    return this.state.mode === "docked";
  }

  dockEdge(): Edge | null {
    return this.state.mode === "docked" ? this.state.edge : null;
  }

  private onPointerDown = (ev: PointerEvent) => {
    if (ev.button !== 0) return;
    if (this.state.mode !== "floating") return; // docked 锁定拖拽
    const r = this.el.getBoundingClientRect();
    this.drag = { dx: ev.clientX - r.left, dy: ev.clientY - r.top };
  };

  private onPointerMove = (ev: PointerEvent) => {
    if (!this.drag) return;
    this.el.style.left = `${ev.clientX - this.drag.dx}px`;
    this.el.style.top = `${ev.clientY - this.drag.dy}px`;
  };

  private onPointerUp = () => {
    if (!this.drag) return;
    this.drag = null;
    this.dispatch("view:moved", this.center());
  };

  private onContextMenu = (ev: MouseEvent) => {
    ev.preventDefault();
    if (this.state.mode === "floating") this.dock();
    else this.undock();
  };

  /** 右键：吸附最近屏幕边缘（docs/view.md §状态机） */
  private dock() {
    const { x, y } = this.center();
    const dists: [Edge, number][] = [
      ["top", y],
      ["bottom", window.innerHeight - y],
      ["left", x],
      ["right", window.innerWidth - x],
    ];
    dists.sort((a, b) => a[1] - b[1]);
    const edge = dists[0][0];
    this.state = { mode: "docked", edge };
    this.el.dataset.docked = edge;

    // 水平边缘保持 x，垂直边缘保持 y（避免窗口跳动）
    const r = this.el.getBoundingClientRect();
    if (edge === "top") this.el.style.top = `${DOCK_MARGIN}px`;
    if (edge === "bottom")
      this.el.style.top = `${window.innerHeight - r.height - DOCK_MARGIN}px`;
    if (edge === "left") this.el.style.left = `${DOCK_MARGIN}px`;
    if (edge === "right")
      this.el.style.left = `${window.innerWidth - r.width - DOCK_MARGIN}px`;

    this.dispatch("view:docked", { edge });
  }

  /** 吸附态再次右键：解除吸附（docs/view.md 设计决定） */
  private undock() {
    this.state = { mode: "floating" };
    delete this.el.dataset.docked;
    this.dispatch("view:undocked", {});
  }

  private dispatch(name: string, detail: unknown) {
    this.el.dispatchEvent(new CustomEvent(name, { detail, bubbles: true }));
  }
}
