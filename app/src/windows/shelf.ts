// Cards Shelf 窗口（Card 集合的瞬时管理弹出层，不属于 Surface）。
// 瞬时语义：尺寸 = pet 物理尺寸 ×3（钳制 180–480 × 120–240，toggle 载荷带来 pet
// 中心与宽高，打开时现算）；位置 = 左下角落在 pet 中心、向右上延伸；中键点 pet 或
// shelf 任意位置直接关闭；失焦即关（600ms 武装延迟）；pet 拖拽/托盘连坐关。
// 面板内容 = 共享 ShelfPanel（同 browser）。

import { createBridge, type Bridge } from "../bridge";
import { Store } from "../store";
import { wireI18n } from "../i18n";
import { wireTheme } from "../theme";
import * as actions from "../tauri_runtime_actions";
import { ShelfPanel } from "./shelf-panel";

const MIN_W = 180;
const MAX_W = 480;
const MIN_H = 120;
const MAX_H = 240;
/** 失焦关闭的武装延迟：显示后 600ms 内的失焦事件忽略（焦点接力失败不秒杀） */
const FOCUS_ARM_MS = 600;

let shownAt = 0;
const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);

export async function main() {
  if (!("__TAURI_INTERNALS__" in window)) return; // Shelf 面板在 browser 由 pet 页内嵌
  const { getCurrentWindow, currentMonitor } = await import("@tauri-apps/api/window");
  const { listen } = await import("@tauri-apps/api/event");
  const win = getCurrentWindow();
  const bridge: Bridge = await createBridge();
  // 前端 store：cards 注册表经 store 读
  const store = await Store.create(bridge);
  wireTheme(store); // 瞬时弹出层同样采用当前主题
  wireI18n(store, () => void panel.refresh()); // 语言切换即重渲染

  const panel = new ShelfPanel(document.body, {
    list: async () => store.cards ?? [],
    setUserClosed: async (c, userClosed) => {
      const id = c.component.id;
      const resp = await bridge.setCardUserClosed?.(id, userClosed);
      if (resp && !resp.ok) {
        console.warn("[shelf] set_card_user_closed 失败", resp);
        return;
      }
      await actions.emitEvent("shelf:visibility", { id, visible: !userClosed, spec: c.component }, "pet");
      await store.refreshCards();
      await panel.refresh();
    },
    dismiss: async (c, title) => {
      void title; // 文本由 core 现写（lifecycle 单源）
      bridge.pushEvent({ action: "dismiss", cardId: c.component.id });
      await actions.emitEvent("shelf:dismiss", { id: c.component.id }, "pet");
      await store.refreshCards();
      await panel.refresh();
    },
    onCardsChanged: (cb) => store.onCards(cb),
  });

  const close = () => void actions.hideWindow(win);

  // 中键 toggle（pet 或 shelf 任意位置中键都直接关闭）：pet 发来中心与物理宽高——
  // 尺寸 = pet ×3（钳制），左下角落在 pet 中心、向右上延伸（屏边界钳制）
  await listen<{ x: number; y: number; w: number; h: number }>("shelf:toggle", async (ev) => {
    if (await win.isVisible()) {
      close();
      return;
    }
    const w = clamp(Math.round(ev.payload.w * 3), MIN_W, MAX_W);
    const h = clamp(Math.round(ev.payload.h * 3), MIN_H, MAX_H);
    await actions.resizeWindow(win, w, h);
    const mon = await currentMonitor();
    const sx = mon ? mon.position.x + mon.size.width : Infinity;
    const x = Math.min(Math.round(ev.payload.x), sx - w - 8);
    const y = Math.max(8, Math.round(ev.payload.y) - h);
    await actions.moveWindow(win, x, y);
    shownAt = Date.now();
    await actions.showWindow(win);
    await panel.refresh();
  });
  // 系统藏（pet 拖拽/托盘连坐）：瞬时面板直接关
  await listen("shelf:hide", close);

  // 中键点 shelf 任意位置 = 关闭（行内按钮保持左键语义）
  document.addEventListener("auxclick", (e) => {
    if ((e as MouseEvent).button === 1) {
      e.preventDefault();
      close();
    }
  });

  // 失焦即关（瞬时语义；武装延迟防显示瞬间焦点接力失败秒杀）
  await win.onFocusChanged((ev) => {
    if (!ev.payload && Date.now() - shownAt > FOCUS_ARM_MS) {
      close();
    }
  });
}
