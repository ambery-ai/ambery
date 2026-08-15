// ShelfPanel — Cards Shelf 的共享 DOM 面板（Tauri shelf 窗口与 browser pet 页共用，
// 同一 UI 真相）。Cards Shelf 是 pet 锚定的瞬时管理弹出层（不属于 Surface）：无标题栏、无 ×——关闭全靠中键 / 失焦 / 点面板外
// （由环境层接管）。环境差异收进 ShelfActions 回调：Tauri = IPC + pet 窗口动作；
// browser = mock bridge + DOM 卡片显隐。

import type { RestoredCard } from "../bridge";
import { t } from "../i18n";

/** 类型图标（五类 Component） */
const TYPE_ICON: Record<string, string> = {
  text_card: "📄",
  quick_jump: "↗️",
  git_display: "🌿",
  data_chart: "📊",
  todobox: "☑️",
};

export interface ShelfActions {
  /** 拉取存活卡片（component + user_closed） */
  list(): Promise<RestoredCard[]>;
  /** 显示选择切换（落 _meta.user_closed + 藏/开窗或 DOM 显隐；userClosed=true=隐藏） */
  setUserClosed(c: RestoredCard, userClosed: boolean): Promise<void>;
  /** dismiss（结束 Surface：closed_by_user 事件 + 出注册 + 销毁窗/DOM） */
  dismiss(c: RestoredCard, title: string): Promise<void>;
  /** Card 集合外部变化通知（agent render/close）→ 面板刷新 */
  onCardsChanged?(cb: () => void): void;
}

export class ShelfPanel {
  private body: HTMLElement;

  constructor(mount: HTMLElement, private actions: ShelfActions) {
    const root = document.createElement("div");
    root.id = "shelf-panel";
    root.innerHTML = `<div id="shelf-body">${t("shelf.loading")}</div>`;
    mount.appendChild(root);
    this.body = root.querySelector("#shelf-body")!;
    actions.onCardsChanged?.(() => void this.refresh());
  }

  async refresh() {
    const cards = await this.actions.list();
    if (cards.length === 0) {
      const empty = document.createElement("div");
      empty.className = "dim";
      empty.style.padding = "6px 8px";
      empty.textContent = t("shelf.empty");
      this.body.replaceChildren(empty);
      return;
    }
    this.body.innerHTML = "";
    for (const c of cards) {
      const id = c.component.id;
      const spec = c.component as { title?: string; label?: string };
      const title = spec.title ?? spec.label ?? id;
      const row = document.createElement("div");
      row.className = "shelf-row";
      // 行 = 类型图标 + 标题 + 显隐/删除两图标（极简单行；中键点任意位置关面板）
      row.innerHTML = `<span class="shelf-type" title="${escapeHtml(c.component.type)}">${TYPE_ICON[c.component.type] ?? "▢"}</span>
        <span class="shelf-title${c.user_closed ? " closed" : ""}"></span>
        <button class="shelf-vis" title="${c.user_closed ? t("shelf.show") : t("shelf.hide")}">${c.user_closed ? "👁" : "🙈"}</button>
        <button class="shelf-dismiss" title="${t("shelf.dismiss-title")}">✕</button>`;
      row.querySelector(".shelf-title")!.textContent = title;
      row.querySelector(".shelf-title")!.setAttribute("title", `${title} (${id})`);
      row.querySelector(".shelf-vis")!.addEventListener("click", () => void this.actions.setUserClosed(c, !c.user_closed));
      row.querySelector(".shelf-dismiss")!.addEventListener("click", () => void this.actions.dismiss(c, title));
      this.body.appendChild(row);
    }
  }
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"]/g, (ch) => `&#${ch.charCodeAt(0)};`);
}
