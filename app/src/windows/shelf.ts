// Cards Shelf 窗口（docs/view.md：Card 集合的管理 Surface）——pet 中键唤出。
// 每张 Card 一行：标题/类型 + 显隐切换 + dismiss；显示选择与布局的持久真相在
// .card.json（docs/components.md §Card 文件），本窗口只做管理与转发：
// - 显隐：setCardUserClosed 落文件 + shelf:visibility 事件让 pet 开窗/藏窗
// - dismiss：pushEvent（closed_by_user 双行事件 + 删文件）+ shelf:dismiss 让 pet 销毁窗口
// 显隐/恢复/拖拽语义与 chat 同款（engine 占区：系统藏保留、用户藏释放留布局记忆）。

import { createBridge, type Bridge, type RestoredCard } from "../bridge";
import { requestPlace, requestRelease, reportMoved } from "../positioning/tauri-server";
import { Direction } from "../positioning/types";
import * as actions from "../tauri_runtime_actions";

const PANEL_W = 380;
const PANEL_H = 560;

let bridge: Bridge;
let cards: RestoredCard[] = [];
/** 运行期显示选择（与 chat 同款：不跨重启，重启归隐藏） */
let userClosed = false;
let isVisible = false;

export async function main() {
  if (!("__TAURI_INTERNALS__" in window)) return; // Shelf 只存在于 Tauri（无浏览器模拟）
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const { listen } = await import("@tauri-apps/api/event");
  const win = getCurrentWindow();
  bridge = await createBridge();

  document.body.innerHTML = `<div id="shelf-panel">
    <div id="shelf-head"><span>🗂 Cards Shelf</span><button id="shelf-close" title="关闭">✕</button></div>
    <div id="shelf-body">加载中…</div>
  </div>`;

  // 显隐切换（pet 中键）：toggle = 用户意图
  await listen("shelf:toggle", async () => {
    if (userClosed || !isVisible) {
      userClosed = false;
      await showShelf();
    } else {
      closeShelf();
    }
  });
  // 系统藏（pet 拖拽/托盘连坐）：只藏不动 userClosed
  await listen("shelf:hide", () => {
    isVisible = false;
    void actions.hideWindow(win);
  });
  // 系统恢复：userClosed 不回弹
  await listen<{ x: number; y: number }>("shelf:show", async (ev) => {
    if (userClosed) return;
    await setPos(ev.payload.x, ev.payload.y);
    await actions.showWindow(win);
    isVisible = true;
  });
  // Card 集合变化（agent 渲染/关闭）→ 重拉
  await listen("effect", (ev) => {
    const kind = (ev.payload as { kind?: string })?.kind;
    if (kind === "render_component" || kind === "close_component") void refresh();
  });

  document.getElementById("shelf-close")!.onclick = () => closeShelf();

  // 头部拖拽（与 chat 同款语义；拖拽结束回写 engine 偏移基准）
  document.addEventListener("mousedown", (e) => {
    const t = e.target as HTMLElement;
    if (t.closest("#shelf-head") && !t.closest("#shelf-close")) {
      void actions.startDragging(win);
    }
  });
  let moveTimer: number | undefined;
  await win.onMoved(() => {
    clearTimeout(moveTimer);
    moveTimer = window.setTimeout(async () => {
      const pos = await win.outerPosition();
      await reportMoved("cards-shelf", { x: pos.x + PANEL_W / 2, y: pos.y + PANEL_H / 2 });
    }, 250);
  });

  await refresh();
}

function closeShelf() {
  userClosed = true;
  isVisible = false;
  void requestRelease("cards-shelf"); // 用户隐藏：释放占区、布局入记忆
  void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
    void actions.hideWindow(getCurrentWindow());
  });
}

async function showShelf() {
  const pos = await requestPlace(
    "cards-shelf",
    { id: "cards-shelf", width: PANEL_W, height: PANEL_H },
    Direction.sse,
  );
  await setPos(pos.x, pos.y);
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await actions.showWindow(getCurrentWindow());
  isVisible = true;
}

async function setPos(cx: number, cy: number) {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await actions.moveWindow(getCurrentWindow(), Math.round(cx - PANEL_W / 2), Math.round(cy - PANEL_H / 2));
}

async function refresh() {
  cards = (await bridge.listCards?.()) ?? [];
  render();
}

function render() {
  const body = document.getElementById("shelf-body");
  if (!body) return;
  if (cards.length === 0) {
    body.innerHTML = `<div class="dim" style="padding:12px">（无存活卡片）</div>`;
    return;
  }
  body.innerHTML = "";
  for (const c of cards) {
    const id = c.component.id;
    const spec = c.component as { title?: string; label?: string };
    const title = spec.title ?? spec.label ?? id;
    const row = document.createElement("div");
    row.className = "shelf-row";
    const visBtn = c.user_closed ? "显示" : "隐藏";
    row.innerHTML = `<div class="shelf-info"><div class="shelf-title"></div>
      <div class="shelf-sub">${escapeHtml(c.component.type)} · ${escapeHtml(id)}${c.user_closed ? " · 已隐藏" : ""}</div></div>
      <button class="shelf-vis">${visBtn}</button>
      <button class="shelf-dismiss" title="dismiss（结束 Surface，忘记布局）">✕</button>`;
    row.querySelector(".shelf-title")!.textContent = title;
    row.querySelector(".shelf-vis")!.addEventListener("click", () => void setVisibility(c, !c.user_closed));
    row.querySelector(".shelf-dismiss")!.addEventListener("click", () => void dismissCard(c, title));
    body.appendChild(row);
  }
}

/** 显隐切换：落文件（显示选择）+ 通知 pet 开窗/藏窗 */
async function setVisibility(c: RestoredCard, visible: boolean) {
  const id = c.component.id;
  const resp = await bridge.setCardUserClosed?.(id, !visible);
  if (resp && !resp.ok) {
    console.warn("[shelf] set_card_user_closed 失败", resp);
    return;
  }
  await actions.emitEvent("shelf:visibility", { id, visible, spec: c.component }, "pet");
  c.user_closed = !visible;
  render();
}

/** dismiss：closed_by_user 双行事件 + 删 .card.json（core 侧 push_event 路径）
 *  + pet 销毁窗口（shelf:dismiss） */
async function dismissCard(c: RestoredCard, title: string) {
  bridge.pushEvent(`用户关闭了 ${c.component.type}「${title}」(${c.component.id})`, { cardId: c.component.id });
  await actions.emitEvent("shelf:dismiss", { id: c.component.id }, "pet");
  await refresh();
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"]/g, (ch) => `&#${ch.charCodeAt(0)};`);
}
