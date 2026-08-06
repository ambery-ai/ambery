// Chat Panel（concepts §3a，设计 docs/chat-panel.md）：View 右键切换唤出/隐藏
// engine.place(Direction.sse) 定位，不重叠其他窗口

import type { Bridge, ContextMessage } from "../bridge";
import { attachDrag } from "../drag";
import { t, wireI18n } from "../i18n";
import type { PositioningEngine } from "../positioning/engine";
import type { Store } from "../store";
import { Direction } from "../positioning/types";

const PANEL_W = 320;
const PANEL_H = 380;

export class ChatPanel {
  private el: HTMLDivElement;
  private historyEl: HTMLDivElement;
  private inputEl: HTMLInputElement;
  private titleEl!: HTMLSpanElement;
  private visible = false;
  private loadingEl: HTMLDivElement | null = null;
  /** 用户意图关闭（docs/window-follow.md：窗口私有，与系统藏分离，单源语义） */
  userClosed = false;
  /** 流式（docs/streaming.md）：在飞的 assistant 气泡 + Thinking 气泡/累积文本 */
  private streamRow: HTMLDivElement | null = null;
  private thinkEl: HTMLDivElement | null = null;
  private thinkText = "";
  private thinkModal: HTMLDivElement | null = null;

  constructor(
    mount: HTMLElement,
    private bridge: Bridge,
    private store: Store,
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
    this.titleEl = document.createElement("span");
    const close = document.createElement("button");
    close.className = "chat-close";
    close.textContent = "×";
    // #26：× 与 toggle 关 / OS 关闭请求同走统一关闭 API（intentClose）
    close.addEventListener("click", () => this.intentClose());
    header.append(this.titleEl, close);

    this.historyEl = document.createElement("div");
    this.historyEl.className = "chat-history";

    this.inputEl = document.createElement("input");
    this.inputEl.className = "chat-input";
    this.relabel();
    // UI 语言切换即重渲染文案（docs/i18n.md；历史内容不翻译）；
    // pet 名称变化同样即时重贴（docs/view.md §名称：UI 读取当前名称）
    wireI18n(store, () => this.relabel());
    store.onConfig(() => this.relabel());
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

    // browser：DOM 拖拽（docs/window-follow.md §拖拽回写；Tauri 走 OS startDragging）
    if (engine) {
      attachDrag(this.el, ".chat-header", ".chat-close", (center) =>
        engine.updateCenter("chat-panel", center),
      );
    }

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

    store.onContext((msgs) => {
      this.dropLoading();
      this.renderHistory(msgs);
    });
  }

  showPanel() { this.el.hidden = false; this.visible = true; }
  isVisible() {
    return this.visible;
  }

  // ── 统一可见性 API（docs/window-follow.md §两路径统一：语义单源，分支只做事件翻译）──
  /** windowed 模式由 chat-window.ts 注入的统一关闭副作用（requestRelease + adapter.hide） */
  onIntentClose: (() => void) | null = null;

  /** 用户意图关（× / toggle 关 / OS 关闭请求，#26 统一收口）：
   *  置 userClosed + 隐藏 + 释放占区保留布局记忆（release 而非 remove——remove 是 dismiss） */
  intentClose() {
    this.userClosed = true;
    this.el.hidden = true;
    this.visible = false;
    this.engine?.release("chat-panel"); // browser 路径；windowed 由 onIntentClose 覆盖
    this.onIntentClose?.();
  }
  /** 用户意图开（toggle 开 / 唤出）：复位 userClosed；定位与显示由路径负责 */
  intentOpen() {
    this.userClosed = false;
  }
  /** 系统藏（pet 拖动/托盘）：只隐藏，不动 userClosed */
  systemHide() {
    this.el.hidden = true;
    this.visible = false;
  }
  /** 系统恢复判定（A 语义）：用户没主动关才恢复；定位与显示由路径负责 */
  systemRestore(): boolean {
    return !this.userClosed;
  }

  /** 按中心点定位并显示（系统恢复路径，docs/window-follow.md）。
   *  不做 clamp（§出屏与重叠：不压人 > 完全可见，部分出屏接受） */
  showAt(center: { x: number; y: number }) {
    this.el.style.left = `${center.x - PANEL_W / 2}px`;
    this.el.style.top = `${center.y - PANEL_H / 2}px`;
    this.showPanel();
  }

  /** 右键 toggle：engine.place(Direction.sse) 定位 */
  toggle() {
    if (this.visible) {
      this.intentClose();
    } else {
      this.intentOpen();
      this.el.hidden = false;
      this.visible = true;
      if (!this.engine) { console.error("[chat] toggle called without engine"); return; }
      const pos = this.engine.place(
        { id: "chat-panel", width: PANEL_W, height: PANEL_H },
        Direction.sse,
      );
      // 不做 clamp（docs/window-follow.md §出屏与重叠：不压人 > 完全可见）
      this.el.style.left = `${pos.x - PANEL_W / 2}px`;
      this.el.style.top = `${pos.y - PANEL_H / 2}px`;
      // 基线读 store（context_changed 事件保持新鲜）；core 未就绪时明示
      const msgs = this.store.context;
      if (msgs) {
        this.renderHistory(msgs);
      } else {
        const warn = document.createElement("div");
        warn.className = "chat-msg chat-system";
        warn.style.opacity = "0.7";
        warn.textContent = t("chat.offline");
        this.historyEl.replaceChildren(warn);
      }
      this.inputEl.focus();
    }
  }

  /** UI 文案（i18n）：标题 = pet 名称（Config 稳定身份值，docs/view.md §名称）；
   *  placeholder 跟随 UI 语言。切换语言只重贴文案，不动历史 */
  private relabel() {
    const name = this.store.config?.name ?? t("pet.default-name");
    this.titleEl.textContent = name;
    this.inputEl.placeholder = t("chat.placeholder", { name });
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
    this.thinkEl.className = "chat-msg chat-system chat-thinking";
    this.thinkEl.textContent = "…";
    this.thinkEl.title = t("chat.thinking-title");
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
    this.thinkModal.className = "think-overlay";
    const card = document.createElement("div");
    card.className = "think-card";
    const pre = document.createElement("pre");
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

