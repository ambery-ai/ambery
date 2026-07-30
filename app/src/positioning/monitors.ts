// positioning/monitors — 显示器缓存表（docs/window-follow.md §显示器几何）：
// 一次读取、逻辑像素、出界自愈刷新。出屏判定/高度 cap 的唯一直径。

import type { Point } from "./types";

export interface MonitorRect {
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
}

let cache: MonitorRect[] | null = null;

/** 读取/重读全显示器（Tauri=availableMonitors 逻辑换算；浏览器=window.screen 单屏语义） */
export async function refreshMonitors(): Promise<MonitorRect[]> {
  if ("__TAURI_INTERNALS__" in window) {
    const { availableMonitors } = await import("@tauri-apps/api/window");
    const ms = await availableMonitors();
    cache = ms.map((m) => ({
      x: m.position.x / m.scaleFactor,
      y: m.position.y / m.scaleFactor,
      width: m.size.width / m.scaleFactor,
      height: m.size.height / m.scaleFactor,
      scaleFactor: m.scaleFactor,
    }));
  } else {
    cache = [{
      x: 0,
      y: 0,
      width: window.screen.availWidth,
      height: window.screen.availHeight,
      scaleFactor: window.devicePixelRatio || 1,
    }];
  }
  return cache;
}

function current(): MonitorRect[] {
  if (!cache) void refreshMonitors();
  return (
    cache ?? [{
      x: 0, y: 0,
      width: window.screen.availWidth,
      height: window.screen.availHeight,
      scaleFactor: window.devicePixelRatio || 1,
    }]
  );
}

/** 点所在屏；不在任何缓存矩形内 = 拓扑变了 → 自愈重读（本拍回退全屏并集） */
export function monitorOf(p: Point): MonitorRect {
  const list = current();
  const hit = list.find(
    (m) => p.x >= m.x && p.x < m.x + m.width && p.y >= m.y && p.y < m.y + m.height,
  );
  if (hit) return hit;
  void refreshMonitors();
  return list.reduce((acc, m) => ({
    x: Math.min(acc.x, m.x),
    y: Math.min(acc.y, m.y),
    width: Math.max(acc.x + acc.width, m.x + m.width) - Math.min(acc.x, m.x),
    height: Math.max(acc.y + acc.height, m.y + m.height) - Math.min(acc.y, m.y),
    scaleFactor: 1,
  }));
}
