// Card 窗口入口 — 每个 card 一个独立 Tauri 窗口，由 pet 动态创建。
// 收到 "card:spec" 事件后渲染、定位、跟随 pet 移动。
import { createBridge, type ComponentSpec } from "../bridge";
import { ComponentManager } from "../components/component-manager";
import { createTauriAdapter, type WindowAdapter } from "../window-adapter";
import { requestPlace, requestRemove, reportMoved } from "../positioning/tauri-server";
import { Store } from "../store";
import { wireI18n } from "../i18n";
import { wireTheme } from "../theme";
import * as actions from "../tauri_runtime_actions";
import { Direction } from "../positioning/types";

let adapter: WindowAdapter | null = null;
let lastPw = 260, lastPh = 140;
let dpr = 1;

export async function main() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const win = getCurrentWindow();
  dpr = window.devicePixelRatio || 1;
  adapter = await createTauriAdapter(document.body, dpr);

  const bridge = await createBridge();
  //  + ：新卡窗随当前主题与 UI 语言，切换即生效
  const store = await Store.create(bridge);
  wireTheme(store);
  wireI18n(store);
  const mount = document.getElementById("app")!;
  mount.classList.add("cards-mode");
  // 屏高 cap 取口统一走 adapter
  const mgr = new ComponentManager(mount, bridge, () => ({ x: 0, y: 0 }), true, undefined, () => adapter!.getScreenHeight());

  const { listen } = await import("@tauri-apps/api/event");

  // #8 拖拽：card header mousedown → 开始移动窗口
  document.addEventListener("mousedown", (e) => {
    if ((e.target as HTMLElement).closest(".cmp-header") && !(e.target as HTMLElement).closest(".cmp-close")) {
      void actions.startDragging(actions.tauriWindowLike(win));
    }
  });

  // #12/#15/#8①：拖拽结束（onMoved 防抖）→ 回写 OS 真实位置为新的跟随基准
  let moveTimer: number | undefined;
  await win.onMoved(() => {
    clearTimeout(moveTimer);
    moveTimer = window.setTimeout(async () => {
      const pos = await win.outerPosition();
      await reportMoved(win.label, { x: pos.x + lastPw / 2, y: pos.y + lastPh / 2 });
    }, 250);
  });

  // 接收 pet 发来的组件 spec：定向防线——spec.id 必须匹配本窗 label，
  // 异 id = 后端误发/广播，忽略并报出（数据完整性可见）
  await listen<ComponentSpec>("card:spec", async (ev) => {
    if (!acceptsSpec(win.label, ev.payload)) {
      console.error(`[card] 忽略异 id spec（本窗 ${win.label}，来 ${ev.payload?.id}）`);
      return;
    }
    mgr.render(ev.payload);
    await new Promise((r) => setTimeout(r, 50));

    const card = document.querySelector(".component") as HTMLElement | null;
    if (!card) return;
    const dir = cardDirection(card);
    const pw = Math.ceil((card.offsetWidth || 260) * dpr);
    const ph = Math.ceil((card.offsetHeight || 140) * dpr);
    lastPw = pw;
    lastPh = ph;
    const label = win.label; // e.g., "card-notify-ft"
    const pos = await requestPlace(label, { id: label, width: pw, height: ph }, dir);
    // chrome 规则（styles.css）：测量值已含 border，窗口恰好包裹内容（阴影留边已废弃）
    await adapter?.setSize(pw, ph);
    await adapter?.setPosition(Math.round(pos.x - pw / 2), Math.round(pos.y - ph / 2));
    await adapter?.show();
  });

  // 拖拽 hide/restore
  await listen("cards:hide", () => adapter?.hide());
  await listen<{ id: string; x: number; y: number }>("cards:show", async (ev) => {
    const { id, x, y } = ev.payload;
    if (id !== win.label) return;
    try {
      await adapter?.setPosition(Math.round(x - lastPw / 2), Math.round(y - lastPh / 2));
    } catch (e) {
      console.warn("[card] restore setPosition failed", e);
    }
    await adapter?.show();
  });

  // 卡片关闭：× 按钮清除 DOM → 清理引擎 + 统一关闭收口（Rust destroy 同步出注册表）
  const observer = new MutationObserver(() => {
    const card = document.querySelector(".component");
    (window as any).__diag_mutation = { ts: Date.now(), hasCard: !!card };
    if (!card) {
      requestRemove(win.label);
      void actions.closeCardWindow(win.label.slice("card-".length));
    }
  });
  observer.observe(mount, { childList: true, subtree: true });

  // OS 级关闭请求：同样收口（destroy 不触发本事件，无回环）
  win.onCloseRequested((ev) => {
    ev.preventDefault();
    requestRemove(win.label);
    void actions.closeCardWindow(win.label.slice("card-".length));
  });
}

/** card:spec 定向判定：spec.id 必须等于窗口 label 去 "card-" 前缀 */
export function acceptsSpec(label: string, spec: ComponentSpec): boolean {
  return spec?.id === label.slice("card-".length);
}

function cardDirection(el: HTMLElement): Direction | "auto" {
  const d = el.dataset.direction;
  if (d && d !== "auto") {
    const dir = Direction[d as keyof typeof Direction];
    if (dir !== undefined) return dir;
  }
  // auto/省略：engine 按「屏幕剩余空间最大的方位」现算
  return "auto";
}
