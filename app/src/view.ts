// View（concepts §3，设计 docs/view.md）：横向椭圆浮动窗口，窗内仅颜文字。
// Tauri 模式：native 窗口拖拽（startDragging）；浏览器模式：DOM pointer 事件。
// 状态机（docs/view.md §状态机）：floating ⇄ docked(edge)——右键吸附/解除，
// 吸附锁定拖拽；几何与窗口移动在环境层（pet.ts），本类只管手势与状态。

import type { Motion } from "./bridge";

/** 吸附边缘（docs/view.md：top/right/bottom/left，取欧氏距离最小者） */
export type DockEdge = "top" | "right" | "bottom" | "left";

export class View {
  readonly el: HTMLDivElement;
  private faceEl: HTMLSpanElement;
  private drag: { dx: number; dy: number } | null = null;
  /** 浏览器 wrapper 模式：拖拽目标改为父容器，默认自身。构造器内赋值以保证 this.el 已存在 */
  dragTarget!: HTMLElement;
  /** Tauri 模式：pet.ts 注入 startDragging 闭包 */
  tauriStartDrag: (() => void) | null = null;
  /** 吸附态（docs/view.md §状态机）；null = floating */
  docked: DockEdge | null = null;

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
    // 吸附态（docs/view.md §状态机）：锁定拖拽；左键单击 = 重新唤出 Chat
    // （chat-panel.md §唤出与关闭：× 关闭后在 View 上左键单击唤出）
    if (this.docked) {
      this.dispatch("view:summon", { edge: this.docked });
      return;
    }
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

  /** 环境层在移动到位后回写吸附态（派发契约事件 view:docked / view:undocked） */
  setDock(edge: DockEdge | null) {
    this.docked = edge;
    if (edge) {
      this.dispatch("view:docked", { edge });
    } else {
      this.dispatch("view:undocked", {});
    }
  }

  private onContextMenu = (ev: MouseEvent) => {
    ev.preventDefault();
    // 右键 = 吸附/解除吸附（docs/view.md §状态机；chat 唤出随 view:docked，chat-panel.md §唤出）
    if (this.docked) {
      this.setDock(null);
    } else {
      // 几何（最近边缘、目标位置）在环境层现算，到位后 setDock 回写
      this.dispatch("view:dock-request", {});
    }
  };

  private dispatch(name: string, detail: unknown) {
    this.el.dispatchEvent(new CustomEvent(name, { detail, bubbles: true }));
  }
}
