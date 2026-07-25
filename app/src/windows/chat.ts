// Chat Panel（concepts §3a，设计 docs/chat-panel.md）：View 右键切换唤出/隐藏
// engine.place(Direction.sse) 定位，不重叠其他窗口

import type { Bridge, QueueMessage } from "../bridge";
import type { PositioningEngine } from "../positioning/engine";
import { Direction } from "../positioning/types";

const PANEL_W = 320;
const PANEL_H = 380;

export class ChatPanel {
  private el: HTMLDivElement;
  private historyEl: HTMLDivElement;
  private inputEl: HTMLInputElement;
  private visible = false;

  constructor(
    mount: HTMLElement,
    private bridge: Bridge,
    private engine: PositioningEngine,
    /** multi-window 模式：跳过 DOM 定位，面板填充窗口 */
    public windowed = false,
  ) {
    this.el = document.createElement("div");
    this.el.id = "chat-panel";
    this.el.hidden = true;
    if (windowed) mount.classList.add("chat-mode");

    const header = document.createElement("div");
    header.className = "chat-header";
    const title = document.createElement("span");
    title.textContent = "ペット";
    const close = document.createElement("button");
    close.className = "chat-close";
    close.textContent = "×";
    close.addEventListener("click", () => this.hide());
    header.append(title, close);

    this.historyEl = document.createElement("div");
    this.historyEl.className = "chat-history";

    this.inputEl = document.createElement("input");
    this.inputEl.className = "chat-input";
    this.inputEl.placeholder = "和ペット说话…";
    this.inputEl.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && this.inputEl.value.trim()) {
        this.bridge.appendUserMessage(this.inputEl.value.trim());
        this.inputEl.value = "";
      }
    });

    this.el.append(header, this.historyEl, this.inputEl);
    mount.appendChild(this.el);

    bridge.onQueueChanged((msgs) => this.renderHistory(msgs));
  }

  isVisible() {
    return this.visible;
  }

  hide() {
    this.visible = false;
    this.el.hidden = true;
    this.engine.remove("chat-panel");
  }

  /** 右键 toggle：engine.place(Direction.sse) 定位 */
  toggle(view: { center(): { x: number; y: number } }) {
    if (this.visible) {
      this.hide();
    } else {
      this.el.hidden = false;
      this.visible = true;
      const c = view.center();
      const r = (document.getElementById("view") as HTMLElement)?.getBoundingClientRect();
      const pos = this.engine.place(
        { id: "chat-panel", width: PANEL_W, height: PANEL_H },
        Direction.sse,
        c,
        { w: r?.width ?? 72, h: r?.height ?? 40 },
      );
      this.el.style.left = `${clamp(pos.x - PANEL_W / 2, 8, window.innerWidth - PANEL_W - 8)}px`;
      this.el.style.top = `${clamp(pos.y - PANEL_H / 2, 8, window.innerHeight - PANEL_H - 8)}px`;
      void this.bridge.getQueue().then((msgs) => this.renderHistory(msgs));
      this.inputEl.focus();
    }
  }

  private renderHistory(msgs: QueueMessage[]) {
    this.historyEl.replaceChildren();
    for (const m of msgs) {
      if (m.role !== "user" && m.role !== "assistant") continue;
      if (!m.content) continue;
      const row = document.createElement("div");
      row.className = `chat-msg chat-${m.role}`;
      row.textContent = m.content;
      this.historyEl.append(row);
    }
    this.historyEl.scrollTop = this.historyEl.scrollHeight;
  }
}

function clamp(v: number, min: number, max: number) {
  return Math.max(min, Math.min(v, Math.max(min, max)));
}
