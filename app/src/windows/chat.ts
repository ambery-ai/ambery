// Chat Panel（concepts §3a，设计 docs/chat-panel.md）：
// View 右键吸附时唤出，Queue 的视图投影——只渲染 user/assistant，输入写 Queue user role。
//
// multi-window 模式（windowed=true）：面板填充整个窗口（窗口外部定位）；
// 浏览器/maximized 模式（默认）：面板 position:fixed 在 View 锚点周围弹出。

import type { Bridge, QueueMessage } from "../bridge";

type Edge = "top" | "right" | "bottom" | "left";

const PANEL_W = 320;
const PANEL_H = 380;
const MARGIN = 12;

export class ChatPanel {
  private el: HTMLDivElement;
  private historyEl: HTMLDivElement;
  private inputEl: HTMLInputElement;
  private visible = false;

  constructor(
    mount: HTMLElement,
    private bridge: Bridge,
    private viewCenter: () => { x: number; y: number },
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
        // 用户输入写 Queue 的 user role（concepts §3a）
        this.bridge.appendUserMessage(this.inputEl.value.trim());
        this.inputEl.value = "";
      }
    });

    this.el.append(header, this.historyEl, this.inputEl);
    mount.appendChild(this.el);

    bridge.onQueueChanged((msgs) => this.renderHistory(msgs));
  }

  /** docked 唤出；edge 决定面板向屏幕内侧展开的方向 */
  show(edge: Edge) {
    this.visible = true;
    this.el.hidden = false;
    if (!this.windowed) this.place(edge);
    void this.bridge.getQueue().then((msgs) => this.renderHistory(msgs));
    this.inputEl.focus();
  }

  hide() {
    this.visible = false;
    this.el.hidden = true;
  }

  isVisible() {
    return this.visible;
  }

  /** 右键 toggle：可见 → 隐藏，不可见 → 显示在 pet 右侧 */
  toggle(view: { center(): { x: number; y: number } }) {
    if (this.visible) {
      this.hide();
    } else {
      const { x, y } = view.center();
      this.el.hidden = false;
      this.visible = true;
      this.el.style.left = `${clamp(x + 50 + MARGIN, 8, window.innerWidth - PANEL_W - 8)}px`;
      this.el.style.top = `${clamp(y - PANEL_H / 2, 8, window.innerHeight - PANEL_H - 8)}px`;
      void this.bridge.getQueue().then((msgs) => this.renderHistory(msgs));
      this.inputEl.focus();
    }
  }

  private place(edge: Edge) {
    const { x, y } = this.viewCenter();
    let left: number;
    let top: number;
    if (edge === "top") {
      left = x - PANEL_W / 2;
      top = y + 20 + MARGIN; // View 半径(纵) + 间距
    } else if (edge === "bottom") {
      left = x - PANEL_W / 2;
      top = y - 20 - MARGIN - PANEL_H;
    } else if (edge === "left") {
      left = x + 36 + MARGIN; // View 半径(横)
      top = y - PANEL_H / 2;
    } else {
      left = x - 36 - MARGIN - PANEL_W;
      top = y - PANEL_H / 2;
    }
    this.el.style.left = `${clamp(left, 8, window.innerWidth - PANEL_W - 8)}px`;
    this.el.style.top = `${clamp(top, 8, window.innerHeight - PANEL_H - 8)}px`;
  }

  /** 只渲染 user/assistant 的非空 content（system/tool 是运行态消息；
   *  assistant 的 tool_calls 载体消息 content 为空，面板无需感知——docs/chat-panel.md） */
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
