// View（concepts §3，设计 docs/view.md）：横向椭圆浮动窗口，窗内仅颜文字。
// Tauri 模式：native 窗口拖拽（startDragging）；浏览器模式：DOM pointer 事件。
// 手势（docs/view.md §手势与 Chat 唤出）：右键 = 唤出/关闭 Chat（chat:toggle，
// pet 原地不动——无吸附态，2026-07-26 否决）；左键拖拽恒可用。

import type { Motion } from "./bridge";

export class View {
  readonly el: HTMLDivElement;
  private faceEl: HTMLSpanElement;
  private drag: { dx: number; dy: number } | null = null;
  /** 浏览器 wrapper 模式：拖拽目标改为父容器，默认自身。构造器内赋值以保证 this.el 已存在 */
  dragTarget!: HTMLElement;
  /** Tauri 模式：pet.ts 注入 startDragging 闭包 */
  tauriStartDrag: (() => void) | null = null;

  constructor(mount: HTMLElement) {
    this.el = document.createElement("div");
    this.el.id = "view";
    this.faceEl = document.createElement("span");
    this.faceEl.id = "face";
    this.el.appendChild(this.faceEl);
    mount.appendChild(this.el);
    this.dragTarget = this.el;
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

  private onPointerDown = (ev: PointerEvent) => {
    if (ev.button !== 0) return;
    // Tauri 模式：native 窗口拖拽
    if (this.tauriStartDrag) {
      this.tauriStartDrag();
      this.dispatch("view:moved", this.center());
      return;
    }
    // 浏览器模式：DOM 拖拽（dragTarget 可能是 wrapper）
    this.dispatch("view:drag-start", {});
    const r = this.dragTarget.getBoundingClientRect();
    this.drag = { dx: ev.clientX - r.left, dy: ev.clientY - r.top };
  };

  private onPointerMove = (ev: PointerEvent) => {
    if (!this.drag) return;
    this.dragTarget.style.left = `${ev.clientX - this.drag.dx}px`;
    this.dragTarget.style.top = `${ev.clientY - this.drag.dy}px`;
  };

  private onPointerUp = () => {
    if (!this.drag) return;
    this.drag = null;
    this.dispatch("view:moved", this.center());
  };

  private onContextMenu = (ev: MouseEvent) => {
    ev.preventDefault();
    // 右键 = 唤出/关闭 Chat（docs/view.md §手势与 Chat 唤出；pet 原地不动）
    this.dispatch("chat:toggle", {});
  };

  private dispatch(name: string, detail: unknown) {
    this.el.dispatchEvent(new CustomEvent(name, { detail, bubbles: true }));
  }
}
