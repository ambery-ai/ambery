// drag — DOM 窗口拖拽助手：
// Tauri 走 OS startDragging；browser 走这里——同一语义：拖动 DOM，松手把中心点回写 engine。

import type { Point } from "./positioning/types";

/**
 * 让 DOM 元素可拖拽（handle 内 mousedown 启动，exclude 内不启动）。
 * 松手时回调元素中心点（调用方换算偏移/回写 engine）。
 */
export function attachDrag(
  el: HTMLElement,
  handleSelector: string,
  excludeSelector: string,
  onDrop: (center: Point) => void,
): void {
  el.addEventListener("pointerdown", (e) => {
    const t = e.target as HTMLElement;
    if (!t.closest(handleSelector) || t.closest(excludeSelector)) return;
    e.preventDefault();
    const startX = e.clientX;
    const startY = e.clientY;
    const startLeft = parseFloat(el.style.left || `${el.getBoundingClientRect().left}`);
    const startTop = parseFloat(el.style.top || `${el.getBoundingClientRect().top}`);
    const move = (ev: PointerEvent) => {
      el.style.left = `${startLeft + ev.clientX - startX}px`;
      el.style.top = `${startTop + ev.clientY - startY}px`;
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (ev.clientX === startX && ev.clientY === startY) return; // 没拖动不算
      const r = el.getBoundingClientRect();
      onDrop({ x: r.left + r.width / 2, y: r.top + r.height / 2 });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });
}
