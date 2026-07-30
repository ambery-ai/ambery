// Chat Panel（concepts §3a，设计 docs/chat-panel.md）：View 右键切换唤出/隐藏
// engine.place(Direction.sse) 定位，不重叠其他窗口

import type { Bridge, ContextMessage } from "../bridge";
import type { PositioningEngine } from "../positioning/engine";
import { Direction } from "../positioning/types";

const PANEL_W = 320;
const PANEL_H = 380;

export class ChatPanel {
  private el: HTMLDivElement;
  private historyEl: HTMLDivElement;
  private inputEl: HTMLInputElement;
  private visible = false;
  private loadingEl: HTMLDivElement | null = null;
  /** 流式（docs/streaming.md）：在飞的 assistant 气泡 + Thinking 气泡/累积文本 */
  private streamRow: HTMLDivElement | null = null;
  private thinkEl: HTMLDivElement | null = null;
  private thinkText = "";
  private thinkModal: HTMLDivElement | null = null;

  constructor(
    mount: HTMLElement,
    private bridge: Bridge,
    private engine?: PositioningEngine,
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
        const text = this.inputEl.value.trim();
        // 立即回显用户消息
        const userRow = document.createElement("div");
        userRow.className = "chat-msg chat-user";
        userRow.textContent = text;
        this.historyEl.append(userRow);
        // loading 指示器（首个 delta 或 done 时移除）
        this.loadingEl = document.createElement("div");
        this.loadingEl.className = "chat-msg chat-system";
        this.loadingEl.textContent = "…";
        this.historyEl.append(this.loadingEl);
        autoScroll(this.historyEl);
        this.bridge.appendUserMessage(text);
        this.inputEl.value = "";
      }
    });

    this.el.append(header, this.historyEl, this.inputEl);
    mount.appendChild(this.el);

    // 流式增量（docs/streaming.md §前端行为）：
    // reasoning → ThinkingBubble「…」+ ThinkingModal 累积；content → 气泡逐片追加
    bridge.onAssistantDelta?.((d) => {
      this.dropLoading();
      if (d.reasoning_content) {
        this.thinkText += d.reasoning_content;
        if (!this.thinkEl) this.showThinkingBubble();
        if (this.thinkModal) this.thinkModal.querySelector("pre")!.textContent = this.thinkText;
        autoScroll(this.historyEl);
      }
      if (d.content) {
        this.clearThinking(); // thinking 阶段结束
        if (!this.streamRow) {
          this.streamRow = document.createElement("div");
          this.streamRow.className = "chat-msg chat-assistant";
          this.historyEl.append(this.streamRow);
        }
        this.streamRow.textContent += d.content;
        autoScroll(this.historyEl);
      }
    });
    bridge.onAssistantDone?.(() => {
      this.dropLoading();
      this.clearThinking();
      this.streamRow = null;
      this.thinkText = "";
    });

    bridge.onContextChanged((msgs) => {
      this.dropLoading();
      this.renderHistory(msgs);
    });
  }

  showPanel() { this.el.hidden = false; this.visible = true; }
  hidePanel() { this.engine?.remove("chat-panel"); this.el.hidden = true; this.visible = false; }
  isVisible() {
    return this.visible;
  }

  hide() { this.visible = false; this.el.hidden = true; this.engine?.remove("chat-panel"); }

  /** 右键 toggle：engine.place(Direction.sse) 定位 */
  toggle() {
    if (this.visible) {
      this.hide();
    } else {
      this.el.hidden = false;
      this.visible = true;
      if (!this.engine) { console.error("[chat] toggle called without engine"); return; }
      const pos = this.engine.place(
        { id: "chat-panel", width: PANEL_W, height: PANEL_H },
        Direction.sse,
      );
      this.el.style.left = `${clamp(pos.x - PANEL_W / 2, 8, window.innerWidth - PANEL_W - 8)}px`;
      this.el.style.top = `${clamp(pos.y - PANEL_H / 2, 8, window.innerHeight - PANEL_H - 8)}px`;
      void this.bridge.getContext()
        .then((msgs) => this.renderHistory(msgs))
        .catch(() => {
          this.historyEl.innerHTML = `<div class="chat-msg chat-system" style="opacity:0.7">⚠ 未连接到 core</div>`;
        });
      this.inputEl.focus();
    }
  }

  private renderHistory(msgs: ContextMessage[]) {
    this.historyEl.replaceChildren();
    for (const m of msgs) {
      if (m.role !== "user" && m.role !== "assistant") continue;
      if (!m.content) continue;
      const row = document.createElement("div");
      row.className = `chat-msg chat-${m.role}`;
      row.textContent = m.content;
      this.historyEl.append(row);
    }
    // 全量重渲不冲掉在飞的流式气泡（他实例事件触发的 context_changed）
    if (this.streamRow) this.historyEl.append(this.streamRow);
    if (this.thinkEl) this.historyEl.append(this.thinkEl);
    autoScroll(this.historyEl);
  }

  private dropLoading() {
    if (this.loadingEl) {
      this.loadingEl.remove();
      this.loadingEl = null;
    }
  }

  /** ThinkingBubble：透明气泡「…」，点击展开 ThinkingModal 看 reasoning 全文 */
  private showThinkingBubble() {
    this.thinkEl = document.createElement("div");
    this.thinkEl.className = "chat-msg chat-system";
    this.thinkEl.style.cssText =
      "opacity:0.6;border:1px dashed rgba(255,255,255,0.35);border-radius:8px;cursor:default";
    this.thinkEl.textContent = "…";
    this.thinkEl.title = "思考中（点击看思维链）";
    this.thinkEl.addEventListener("click", () => this.toggleThinkingModal());
    this.historyEl.append(this.thinkEl);
  }

  private toggleThinkingModal() {
    if (this.thinkModal) {
      this.thinkModal.remove();
      this.thinkModal = null;
      return;
    }
    this.thinkModal = document.createElement("div");
    this.thinkModal.style.cssText =
      "position:fixed;inset:0;background:rgba(0,0,0,0.6);display:flex;align-items:center;justify-content:center;z-index:9999";
    const card = document.createElement("div");
    card.style.cssText =
      "max-width:480px;max-height:60vh;overflow:auto;background:#1e1e28;color:#dde;padding:16px;border-radius:10px;font-size:12px";
    const pre = document.createElement("pre");
    pre.style.cssText = "white-space:pre-wrap;margin:0;font-family:inherit";
    pre.textContent = this.thinkText;
    card.appendChild(pre);
    this.thinkModal.appendChild(card);
    this.thinkModal.addEventListener("click", () => {
      this.thinkModal?.remove();
      this.thinkModal = null;
    });
    document.body.appendChild(this.thinkModal);
  }

  private clearThinking() {
    if (this.thinkEl) {
      this.thinkEl.remove();
      this.thinkEl = null;
    }
    if (this.thinkModal) {
      this.thinkModal.remove();
      this.thinkModal = null;
    }
  }
}

function autoScroll(el: HTMLElement) {
  if (el.scrollHeight - el.scrollTop - el.clientHeight < 50) {
    el.scrollTop = el.scrollHeight;
  }
}

function clamp(v: number, min: number, max: number) {
  return Math.max(min, Math.min(v, Math.max(min, max)));
}
