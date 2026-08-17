// Chat Panel（设计）：用户与 pet 对话的产品界面。
// 滚动由用户意图驱动（跟随最新 / 阅读历史两态），输入服务于「组织一段话并决定发送」，
// 内部状态（Queue/Context/流式增量/窗口尺寸）必须翻译为用户可理解的反馈，不得抢视口。

import type { Bridge, ContextMessage } from "../bridge";
import { attachDrag } from "../drag";
import { reportEffect } from "../effects";
import { t, wireI18n } from "../i18n";
import type { PositioningEngine } from "../positioning/engine";
import type { Store } from "../store";
import { Direction } from "../positioning/types";

const PANEL_W = 320;
const PANEL_H = 380;
/** 输入区自增长上限（px）：超过后输入区自身滚动，不挤没消息历史 */
const INPUT_MAX_H = 110;
/** 贴底判定余量（px） */
const AT_BOTTOM = 8;

export class ChatPanel {
  private el: HTMLDivElement;
  private historyEl: HTMLDivElement;
  private inputEl: HTMLTextAreaElement;
  private sendEl: HTMLButtonElement;
  private pillEl: HTMLDivElement;
  private queueEl: HTMLDivElement;
  private titleEl!: HTMLSpanElement;
  private visible = false;
  /** 回应提示（…）：assistant 未输出正文时显示，开始输出/完成/失败即消失 */
  private replyingEl: HTMLDivElement | null = null;
  /** 用户意图关闭（窗口私有，与系统藏分离，单源语义） */
  userClosed = false;
  /** 流式：在飞的 assistant 气泡 + Thinking 气泡/累积文本 */
  private streamRow: HTMLDivElement | null = null;
  private thinkEl: HTMLDivElement | null = null;
  private thinkText = "";
  private thinkModal: HTMLDivElement | null = null;

  // ── 滚动意图态机──
  /** true=跟随最新（贴底）；false=阅读历史（不抢视口，累积新消息提示） */
  private follow = true;
  /** 阅读历史时累积的新消息计数（一个在飞的流式回复只计一条） */
  private pendingNew = 0;
  private streamCounted = false;
  /** 程序化滚动期间抑制 scroll 事件的意图判读 */
  private suppressScroll = false;
  /** 已发送但尚未得到完整回复的消息数（Queue 串行处理的翻译：正在回复/已排队） */
  private awaitingReply = 0;
  private streaming = false;
  private lastRenderedCount = 0;
  /** 发送失败提示行（界面瞬态，Context 全量重渲不冲掉——失败说明必须留到用户处理） */
  private sendErrorEl: HTMLDivElement | null = null;
  /** 未配置 banner（chat 顶部横幅；LLM 未配置时显示，点击打开引导；配置完成由宿主清除） */
  private setupBannerEl: HTMLDivElement | null = null;
  /** LLM 未配置（宿主检测注入；true 时发送被拦截为错误气泡） */
  unconfigured = false;
  /** 打开配置引导 modal 的回调（由宿主注入；未配置/连接失败是同一 modal 的两种状态） */
  onOpenSetup?: () => void;

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
    // 用户滚离底部 → 阅读历史；滚回底部 → 跟随最新 + 清新消息计数
    this.historyEl.addEventListener("scroll", () => {
      if (this.suppressScroll) return;
      const atBottom =
        this.historyEl.scrollHeight - this.historyEl.scrollTop - this.historyEl.clientHeight < AT_BOTTOM;
      if (atBottom) {
        this.follow = true;
        this.pendingNew = 0;
        this.updatePill();
      } else {
        this.follow = false;
      }
    });
    // 窗口/面板尺寸变化不改变阅读意图：跟随者仍贴底；阅读者以锚点恢复
    // （jsdom 无 ResizeObserver——测试环境跳过，生产必有）
    if (typeof ResizeObserver !== "undefined") {
      new ResizeObserver(() => {
        if (this.el.hidden) return;
        if (this.follow) {
          this.scrollToBottom();
        } else {
          const anchor = this.captureAnchor();
          if (anchor) this.restoreAnchor(anchor);
        }
      }).observe(this.historyEl);
    }

    // 「↓ N 条新消息」提示：点击 = 滚到底部 + 清零 + 恢复跟随
    this.pillEl = document.createElement("div");
    this.pillEl.className = "chat-pill";
    this.pillEl.hidden = true;
    this.pillEl.addEventListener("click", () => {
      this.follow = true;
      this.pendingNew = 0;
      this.updatePill();
      this.scrollToBottom();
    });

    this.queueEl = document.createElement("div");
    this.queueEl.className = "chat-queue-status";
    this.queueEl.hidden = true;

    // 输入区：可自增长多行（默认一行，长到上限后自身滚动）+ 发送按钮（与 Enter 同语义）
    const inputRow = document.createElement("div");
    inputRow.className = "chat-input-row";
    this.inputEl = document.createElement("textarea");
    this.inputEl.className = "chat-input";
    this.inputEl.rows = 1;
    this.inputEl.addEventListener("input", () => this.autoGrow());
    this.inputEl.addEventListener("keydown", (ev) => {
      // Enter 发送；Shift+Enter 换行；输入法组合输入未确认时 Enter 只确认候选（不误发送）
      if (ev.key === "Enter" && !ev.shiftKey && !ev.isComposing) {
        ev.preventDefault();
        void this.send();
      }
    });
    this.sendEl = document.createElement("button");
    this.sendEl.className = "chat-send";
    this.sendEl.disabled = true; // 初始空内容不可用
    this.sendEl.addEventListener("click", () => void this.send());
    inputRow.append(this.inputEl, this.sendEl);

    this.el.append(header, this.historyEl, this.pillEl, this.queueEl, inputRow);
    mount.appendChild(this.el);
    this.relabel();
    // UI 语言切换即重渲染文案（历史内容不翻译）；
    // pet 名称变化同样即时重贴（UI 读取当前名称）
    wireI18n(store, () => this.relabel());
    store.onConfig(() => this.relabel());

    // browser：DOM 拖拽（Tauri 走 OS startDragging）
    if (engine) {
      attachDrag(this.el, ".chat-header", ".chat-close", (center) =>
        engine.updateCenter("chat-panel", center),
      );
    }

    // 流式增量：
    // reasoning → ThinkingBubble「…」+ ThinkingModal 累积；content → 气泡逐片追加
    bridge.onAssistantDelta?.((d) => {
      this.dropReplying();
      this.streaming = true;
      if (d.reasoning_content) {
        this.thinkText += d.reasoning_content;
        if (!this.thinkEl) this.showThinkingBubble();
        if (this.thinkModal) this.thinkModal.querySelector("pre")!.textContent = this.thinkText;
        this.onNewContent();
      }
      if (d.content) {
        this.clearThinking(); // thinking 阶段结束
        if (!this.streamRow) {
          this.streamRow = document.createElement("div");
          this.streamRow.className = "chat-msg chat-assistant";
          this.historyEl.append(this.streamRow);
        }
        this.streamRow.textContent += d.content;
        this.onNewContent();
      }
    });
    bridge.onAssistantDone?.(() => {
      this.dropReplying();
      this.clearThinking();
      this.streamRow = null;
      this.thinkText = "";
      this.streaming = false;
      this.streamCounted = false;
      this.awaitingReply = Math.max(0, this.awaitingReply - 1);
      this.updateQueueStatus();
    });

    // LLM 连接错误：消息流错误气泡（区分原因）+ 输入框上方 banner（可关闭，带打开配置）
    bridge.onLlmError?.((message) => {
      this.showLlmError(message);
    });

    store.onContext((msgs) => {
      this.dropReplying();
      this.renderHistory(msgs);
    });
  }

  // ── 输入与发送──

  private autoGrow() {
    this.inputEl.style.height = "auto";
    const h = Math.min(this.inputEl.scrollHeight, INPUT_MAX_H);
    this.inputEl.style.height = `${h}px`;
    this.inputEl.style.overflowY = this.inputEl.scrollHeight > INPUT_MAX_H ? "auto" : "hidden";
    this.sendEl.disabled = !this.inputEl.value.trim();
  }

  /** 发送 = 等待后续内容的意图：无论此前是否在阅读历史，都无条件滚到底部并恢复跟随 */
  private async send() {
    const text = this.inputEl.value.trim();
    if (!text) return;
    // LLM 未配置：发消息不静默——显示错误气泡（不真正发送；banner 已提示引导入口）
    if (this.unconfigured) {
      this.showUnconfiguredError();
      return;
    }
    this.follow = true;
    this.pendingNew = 0;
    this.updatePill();
    // 立即以用户气泡出现；输入区清空、保留焦点
    const userRow = document.createElement("div");
    userRow.className = "chat-msg chat-user";
    userRow.textContent = text;
    this.historyEl.append(userRow);
    // 动作记录：前端渲染了用户气泡（docs/storage.md effect——记录不驱动渲染）
    reportEffect("user_bubble", { text });
    this.showReplying();
    this.scrollToBottom();
    this.inputEl.value = "";
    this.autoGrow();
    this.inputEl.focus();
    this.awaitingReply++;
    this.updateQueueStatus();
    // 上一次失败提示在新一轮发送时清除（用户已处理：继续编辑或重试）
    this.clearSendError();
    const ok = await this.bridge.appendUserMessage(text);
    if (!ok) {
      // 发送失败：文字不丢——退回输入区（继续编辑路径）+ 明确说明 + 重试路径
      this.awaitingReply--;
      this.updateQueueStatus();
      this.dropReplying();
      userRow.remove();
      this.inputEl.value = text;
      this.autoGrow();
      this.showSendError(text);
    }
  }

  /** LLM 未配置：发消息被拦截——错误气泡（输入框文字保留，可先配置再发） */
  private showUnconfiguredError() {
    const row = document.createElement("div");
    row.className = "chat-msg chat-system chat-llm-error";
    row.textContent = t("chat.unconfigured-error");
    this.historyEl.append(row);
    this.scrollToBottom();
  }

  private showSendError(text: string) {
    const row = document.createElement("div");
    row.className = "chat-msg chat-system chat-send-failed";
    row.textContent = t("chat.send-failed") + " ";
    const retry = document.createElement("button");
    retry.textContent = t("chat.retry");
    retry.addEventListener("click", () => {
      this.clearSendError();
      this.inputEl.value = text;
      void this.send();
    });
    row.appendChild(retry);
    this.sendErrorEl = row;
    this.historyEl.append(row);
    this.scrollToBottom();
  }

  private clearSendError() {
    if (this.sendErrorEl) {
      this.sendErrorEl.remove();
      this.sendErrorEl = null;
    }
  }

  /** LLM 连接错误：消息流错误气泡（区分原因；失败当轮回退 DebugAgent 仍出回复，须注明降级）。
   *  界面瞬态（非 Context 内容）：renderHistory 全量重渲时保留（与 sendErrorEl 同模式） */
  private llmErrorEl: HTMLDivElement | null = null;

  private showLlmError(message: string) {
    const bubble = document.createElement("div");
    bubble.className = "chat-msg chat-system chat-llm-error";
    bubble.textContent = `${t("chat.llm-error")}: ${message}（${t("chat.degraded-reply")}）`;
    this.historyEl.append(bubble);
    this.llmErrorEl = bubble;
    this.scrollToBottom();
    // 动作记录：前端渲染了错误气泡（docs/storage.md effect——记录不驱动渲染）
    reportEffect("error_bubble", { message });
  }

  /** 未配置 banner：LLM 未配置时显示（点击打开引导 modal）；可关闭（仅隐藏当前） */
  showSetupBanner() {
    if (this.setupBannerEl) return;
    const banner = document.createElement("div");
    banner.className = "chat-setup-banner";
    const text = document.createElement("span");
    text.textContent = t("chat.setup-banner");
    banner.appendChild(text);
    banner.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).classList.contains("chat-setup-banner-close")) return;
      this.onOpenSetup?.();
    });
    const close = document.createElement("button");
    close.className = "chat-setup-banner-close";
    close.textContent = "×";
    close.addEventListener("click", () => this.clearSetupBanner());
    banner.appendChild(close);
    this.historyEl.before(banner);
    this.setupBannerEl = banner;
    // 动作记录：前端显示了未配置 banner（docs/storage.md effect——记录不驱动渲染）
    reportEffect("setup_banner");
  }

  clearSetupBanner() {
    if (this.setupBannerEl) {
      this.setupBannerEl.remove();
      this.setupBannerEl = null;
    }
  }

  /** 「正在回复」与「已排队等待处理」的用户可理解翻译（不伪装已排队为已被读取） */
  private updateQueueStatus() {
    const queued = Math.max(0, this.awaitingReply - (this.streaming || this.replyingEl ? 1 : 0));
    if (queued > 0) {
      this.queueEl.textContent = t("chat.queued", { n: String(queued) });
      this.queueEl.hidden = false;
    } else {
      this.queueEl.hidden = true;
    }
  }

  /** 回应提示「…」：只表达正在回应，不暴露内部状态 */
  private showReplying() {
    this.dropReplying();
    this.replyingEl = document.createElement("div");
    this.replyingEl.className = "chat-msg chat-system chat-replying";
    this.replyingEl.textContent = "…";
    this.historyEl.append(this.replyingEl);
  }

  private dropReplying() {
    if (this.replyingEl) {
      this.replyingEl.remove();
      this.replyingEl = null;
    }
  }

  // ── 滚动意图态机 ──

  /** 新内容到达（delta / 全量重渲共用）：跟随者贴底；阅读者累积提示（流式只计一条） */
  private onNewContent() {
    if (this.follow) {
      this.scrollToBottom();
      return;
    }
    if (!this.streamCounted) {
      this.streamCounted = true;
      this.pendingNew++;
      this.updatePill();
    }
  }

  private scrollToBottom() {
    this.suppressScroll = true;
    this.historyEl.scrollTop = this.historyEl.scrollHeight;
    // 滚动事件异步触发，下一拍再恢复意图判读
    setTimeout(() => {
      this.suppressScroll = false;
    }, 0);
  }

  private updatePill() {
    if (!this.follow && this.pendingNew > 0) {
      this.pillEl.textContent = t("chat.new-messages", { n: String(this.pendingNew) });
      this.pillEl.hidden = false;
    } else {
      this.pillEl.hidden = true;
    }
  }

  /** 锚点 = 刷新前第一条可见消息（索引 + 其相对容器顶的偏移），不机械复用 scrollTop */
  private captureAnchor(): { index: number; offset: number } | null {
    const top = this.historyEl.getBoundingClientRect().top;
    const children = [...this.historyEl.children];
    for (let i = 0; i < children.length; i++) {
      const r = children[i].getBoundingClientRect();
      if (r.bottom > top) {
        return { index: i, offset: r.top - top };
      }
    }
    return null;
  }

  private restoreAnchor(anchor: { index: number; offset: number }) {
    const child = this.historyEl.children[anchor.index] as HTMLElement | undefined;
    if (!child) return;
    this.suppressScroll = true;
    // child.offsetTop 相对 historyEl（position:relative 充当 offsetParent），无需再减容器偏移
    this.historyEl.scrollTop = child.offsetTop - anchor.offset;
    setTimeout(() => {
      this.suppressScroll = false;
    }, 0);
  }

  showPanel() { this.el.hidden = false; this.visible = true; }
  isVisible() {
    return this.visible;
  }

  // ── 统一可见性 API（语义单源，分支只做事件翻译）──
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

  /** 按中心点定位并显示（系统恢复路径）。
   *  不做 clamp（§出屏与重叠：不压人 > 完全可见，部分出屏接受） */
  showAt(center: { x: number; y: number }) {
    this.el.style.left = `${center.x - PANEL_W / 2}px`;
    this.el.style.top = `${center.y - PANEL_H / 2}px`;
    this.showPanel();
  }

  /** 唤出（右键 toggle）：
   *  intentOpen 复位 + engine.place 固定 sse 方位展开（自己的定位引擎） +
   *  渲染历史 + 初次打开定位历史底部（跟随最新） */
  open() {
    this.intentOpen();
    this.el.hidden = false;
    this.visible = true;
    if (!this.engine) { console.error("[chat] open called without engine"); return; }
    const pos = this.engine.place(
      { id: "chat-panel", width: PANEL_W, height: PANEL_H },
      Direction.sse,
    );
    // 不做 clamp（不压人 > 完全可见）
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
    // 初次打开定位到历史底部，进入「跟随最新」
    this.follow = true;
    this.pendingNew = 0;
    this.updatePill();
    this.scrollToBottom();
    this.inputEl.focus();
  }

  /** 关闭（右键 toggle / × 同一收口）：统一 API */
  close() {
    this.intentClose();
  }

  /** 右键 toggle：可见则关、不可见则开 */
  toggle() {
    if (this.visible) {
      this.close();
    } else {
      this.open();
    }
  }

  /** UI 文案（i18n）：标题 = 聊天（i18n 固有文案，不显示 pet 名称）；
   *  按钮文案跟随 UI 语言。切换语言只重贴文案，不动历史 */
  private relabel() {
    this.titleEl.textContent = t("chat.title");
    this.sendEl.textContent = t("chat.send");
    this.updateQueueStatus();
    this.updatePill();
  }

  private renderHistory(msgs: ContextMessage[]) {
    // 阅读历史时以刷新前第一条可见消息为锚点（不机械复用旧 scrollTop）
    const anchor = this.follow ? null : this.captureAnchor();
    const visible = msgs.filter((m) => (m.role === "user" || m.role === "assistant") && m.content);
    this.historyEl.replaceChildren();
    for (const m of visible) {
      const row = document.createElement("div");
      row.className = `chat-msg chat-${m.role}`;
      row.textContent = m.content;
      this.historyEl.append(row);
    }
    // 全量重渲不冲掉在飞的流式气泡与发送失败提示行（界面瞬态，非 Context 内容）
    if (this.streamRow) this.historyEl.append(this.streamRow);
    if (this.thinkEl) this.historyEl.append(this.thinkEl);
    if (this.sendErrorEl) this.historyEl.append(this.sendErrorEl);
    if (this.llmErrorEl) this.historyEl.append(this.llmErrorEl);
    // 新消息计数（阅读历史时；一条流式回复只计一条，不因增量重复计）
    if (!this.follow && visible.length > this.lastRenderedCount) {
      this.pendingNew += visible.length - this.lastRenderedCount;
      this.updatePill();
    }
    this.lastRenderedCount = visible.length;
    if (this.follow) this.scrollToBottom();
    else if (anchor) this.restoreAnchor(anchor);
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
