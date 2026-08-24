// Bridge：前端与 Ambery 内核之间的唯一边界。
// Tauri 模式走 IPC（后续 Tauri 壳迭代接入），浏览器测试模式走内存 mock，
// 使全部显示逻辑可在 Chrome DevTools 中直接驱动验证。

// ── 领域模型（与  对齐） ──

/**  Status 状态机 */
export type CodeCliStatus = "idle" | "processing" | "unknown";

export interface CodeCliInstance {
  id: string;
  name: string;
  status: CodeCliStatus;
}

/** Autonomy 顶层状态 = Context 中 Code CLI Status 一览 + 未决通知数 */
export interface TopState {
  instances: CodeCliInstance[];
  pendingNotifications: number;
}

/** Motion 枚举 */
export type Motion = "still" | "float" | "bounce" | "shake";

export interface KaomojiEntry {
  face: string;
  motion: Motion;
}

/**  Config（当前仅前端所需子集） */
export interface AppConfig {
  /** 表情两池：system 系统池（尺寸扫描来源）+ user 用户池。
   *  key 全局唯一（后端校验不相交），默认状态与 set_autonomy(key) 按并集解析 */
  kaomoji: {
    system: Record<string, KaomojiEntry>;
    user: Record<string, KaomojiEntry>;
  };
  /** set_autonomy 省略 ttlMs 时的默认值 */
  setAutonomyDefaultTtlMs: number;
  /** View 缩放（默认 0.5） */
  viewScale: number;
  /** 未读角标样式（#5）：number 纯数字（默认）/ bubble 气泡 */
  badgeStyle?: "number" | "bubble";
  /** 未读角标方位（#5）：right（默认）/ left */
  badgeSide?: "right" | "left";
  /** 当前主题名：themes 的 key */
  theme?: string;
  /** 主题表：主题名 → token 覆写表（token 名去 --ov- 前缀 → CSS 值） */
  themes?: Record<string, Record<string, string>>;
  /** UI 语言：zh / en */
  uiLanguage?: "zh" | "en";
  /** pet 名称：稳定身份值，不参与翻译 */
  name?: string;
}

/** Component 方位（八方位词；引擎内部按 16 方位环解析） */
export type Direction =
  | "n"
  | "ne"
  | "e"
  | "se"
  | "s"
  | "sw"
  | "w"
  | "nw"
  | "auto";

/** call_component 协议（判别联合） */
export type ComponentSpec =
  | { id: string; type: "text_card"; direction?: Direction; title: string; text: string }
  | { id: string; type: "quick_jump"; direction?: Direction; label: string; target: string }
  | {
      id: string;
      type: "git_display";
      direction?: Direction;
      title: string;
      entries: { hash: string; msg: string; time: string }[];
      diff?: string;
    }
  | {
      id: string;
      type: "data_chart";
      direction?: Direction;
      title: string;
      chart: {
        kind: "line" | "bar" | "pie";
        labels: string[];
        series: { name: string; data: number[] }[];
      };
    }
  | {
      id: string;
      type: "todobox";
      direction?: Direction;
      title: string;
      items: { text: string; done: boolean }[];
    };

/** Queue 消息（四 role） */
export interface ContextMessage {
  role: "user" | "assistant" | "tool" | "system";
  content: string;
  ts: number;
}

/** 错误通知（错误即通知模型）：retention 是唯一路由轴——transient 瞬时气泡 /
 *  persistent 常驻 banner；action 可选：banner 点击分派目标（如 "setup" 配置引导），无则纯告知 */
export interface ErrorEvent {
  message: string;
  retention: "transient" | "persistent";
  action?: string;
}

export interface Bridge {
  getConfig(): Promise<AppConfig>;
  getTopState(): Promise<TopState>;
  onTopStateChanged(cb: (s: TopState) => void): void;
  /** Ambery → UI：渲染 Component（pet call_component 的执行结果） */
  onRenderComponent(cb: (spec: ComponentSpec) => void): void;
  /** UI → Harness：Component 交互事件写入 Event Buffer。
   *  前端只上报结构化事实；自然语言文本由 core 按 Harness 语言现写
   *  （lifecycle 语义单源）。
   *  dismiss：closed_by_user 双行事件 + 删 .card.json + 出注册表；
   *  todo_toggle/todo_add 经 state 携带双载荷快照（同 card 去重合并） */
  pushEvent(ev: UserActionEvent): void;
  /** Queue：对话历史读取 + 用户输入写入 user role。
   *  appendUserMessage 返回是否成功送达 core——失败时调用方必须让用户文字不丢
   *  （明确说明失败 + 可重试/继续编辑） */
  getContext(): Promise<ContextMessage[]>;
  appendUserMessage(text: string): Promise<boolean>;
  onContextChanged(cb: (msgs: ContextMessage[]) => void): void;
  /** 可选（RemoteBridge）：Ambery 推送 set_autonomy（pet 的 tool call 结果） */
  onSetAutonomy?(
    cb: (args: { face?: string; motion?: Motion; ttlMs?: number; once?: boolean }) => void,
  ): void;
  /** 可选（RemoteBridge）：Ambery 推送 Config 变更（edit_config 的结果） */
  onConfigChanged?(cb: (cfg: AppConfig) => void): void;
  /** 可选：流式增量——assistant 回复片段，纯显示优化 */
  onAssistantDelta?(cb: (d: { content?: string; reasoning_content?: string }) => void): void;
  /** 可选：一轮回复完毕（loading 收尾，完整回复已写 Context） */
  onAssistantDone?(cb: () => void): void;
  /** 可选：错误通知（错误即通知模型）——按 retention 路由呈现 */
  onError?(cb: (e: ErrorEvent) => void): void;
  /** 可选：显式关闭卡片（Component 持续管理协议：action="close"） */
  onCloseComponent?(cb: (id: string) => void): void;
  /** 可选（TauriBridge）：Card 跨重启恢复——启动拉取全部存活卡片
   *  （component + _meta 显示选择与布局；readonly 查询） */
  listCards?(): Promise<RestoredCard[]>;
  /** 可选（TauriBridge）：Card 显示选择回写（Cards Shelf 显隐切换 → _meta.user_closed） */
  setCardUserClosed?(id: string, userClosed: boolean): Promise<{ ok: boolean; error?: string }>;
  /** 可选：Card 集合外部变化通知（agent render/close）——Shelf 面板刷新触发 */
  onCardsChanged?(cb: () => void): void;
  /** 可选（TauriBridge/RemoteBridge）：设置面板 schema 读取（单消费者、体积大，不进 store） */
  getConfigSchema?(): Promise<ConfigSchemaResp>;
  /** 可选（TauriBridge/RemoteBridge）：设置面板改值（写：core 统一修改管道，
   *  端点记录 config_update effect） */
  setConfig?(path: string, value: unknown): Promise<SetConfigResp>;
  /** 可选（TauriBridge/RemoteBridge）：LLM 连通测试——按 active provider 构建一次调用，
   *  返回成功或具体失败原因（env 未设 / 401 / 超时 / 网络 / provider 缺失） */
  testLlm?(): Promise<{ ok: boolean; reply?: string; error?: string }>;
  /** 可选（TauriBridge/RemoteBridge）：provider key 存在性状态（env 文件 → 进程环境，
   *  本地即时）——返回 set 与来源（"env 文件" / "环境变量" / null） */
  getApiKeyStatus?(provider: string): Promise<{ ok: boolean; set: boolean; source: string | null }>;
  /** 可选（TauriBridge/RemoteBridge）：写/清 provider key——Some 写入应用级 env 文件
   *  （统一 AMBERY_<PROVIDER>_API_KEY 名 + api_key_env 归一），null 清除 */
  setApiKey?(provider: string, key: string | null): Promise<{ ok: boolean; error?: string }>;
  /** 可选（TauriBridge）：Card 布局回写（写：拖拽结束落 _meta.layout，
   *  端点记录 card_layout effect） */
  updateCardLayout?(id: string, offset: [number, number]): Promise<{ ok: boolean; error?: string }>;
}

/** Component 交互事件（结构化事实；desc 组合在 core，lifecycle.rs user_action_desc） */
export interface UserActionEvent {
  action: "copy" | "jump" | "expand_diff" | "todo_toggle" | "todo_add" | "dismiss";
  cardId?: string;
  cardType?: string;
  title?: string;
  text?: string;
  target?: string;
  checked?: boolean;
  state?: unknown;
}

/** list_cards 返回项：component 原文 + _meta 状态 */
export interface RestoredCard {
  component: ComponentSpec;
  user_closed: boolean;
  layout: { direction: string | null; offset: [number, number] | null; manual: boolean };
}

/** 设置面板 schema：menu 单消费者且体积大，
 *  不满足 store 边界判据（多窗口/组件读），经 bridge 方法直读，不进 store */
export interface ConfigNodeType {
  kind: "bool" | "int" | "float" | "str" | "enum" | "map" | "other";
  min?: number;
  max?: number;
  options?: string[];
}
export interface ConfigSchemaNode {
  path: string;
  type: ConfigNodeType;
  desc?: string;
  value: unknown;
}
export interface ConfigSchemaResp {
  version: number;
  readOnly: boolean;
  restartRequired?: string[];
  loadError?: string | null;
  /** LLM 初始化失败原因（active 指向损坏 provider）；null = 可初始化 */
  llmError?: string | null;
  nodes: ConfigSchemaNode[];
}

/** set_config 响应（core 统一修改管道） */
export interface SetConfigResp {
  ok: boolean;
  restartRequired?: string[];
  error?: string;
}

/** mock 的 debug 文本组合（仅 BrowserMockBridge 事件检视用；生产文本在 core lifecycle） */
function mockActionDesc(ev: UserActionEvent): string {
  switch (ev.action) {
    case "copy":
      return `用户复制了 ${ev.cardType}「${ev.title}」的内容`;
    case "jump":
      return `用户点击 ${ev.cardType} 跳转到「${ev.target}」`;
    case "expand_diff":
      return `用户展开了 ${ev.cardType}「${ev.title}」的 diff`;
    case "todo_toggle":
      return `用户${ev.checked ? "勾选" : "取消勾选"}了 ${ev.cardType} 条目「${ev.text}」`;
    case "todo_add":
      return `用户新增了 ${ev.cardType} 条目「${ev.text}」`;
    case "dismiss":
      return `用户关闭了 ${ev.cardType ?? "card"}(${ev.cardId})`;
  }
}

// ── Chrome DevTools 调试驱动接口（window.__ambery） ──

export interface DebugApi {
  setInstanceStatus(name: string, status: CodeCliStatus): void;
  addInstance(name: string, status: CodeCliStatus): void;
  notify(n?: number): void;
  clearNotifications(): void;
  setAutonomy(args: { face?: string; motion?: Motion; ttlMs?: number; once?: boolean }): void;
  viewState(): {
    center: { x: number; y: number };
    face: string | null;
    motion: string;
  };
  /** 模拟 pet 的 call_component tool call */
  callComponent(spec: ComponentSpec): void;
  /** 读取 Event Buffer 当前内容（不写 Queue user role） */
  eventBuffer(): string[];
  /** 模拟 LLM 触发时合并注入后清空 Buffer */
  flushEventBuffer(): string[];
  /** 模拟 pet 回复 / Ambery 注入 system 消息（真实链路由 Rust Harness 写入） */
  appendMessage(role: ContextMessage["role"], content: string): void;
}

declare global {
  interface Window {
    __ambery?: DebugApi;
  }
}

// ── 浏览器 mock ──

const DEFAULT_CONFIG: AppConfig = {
  kaomoji: {
    system: {
      idle: { face: "(´ω`)", motion: "still" },
      processing: { face: "(ˇωˇ」∠)_", motion: "float" },
      notify: { face: "✧*｡٩(ˊᗜˋ*)و✧*｡", motion: "bounce" },
    },
    user: {},
  },
  setAutonomyDefaultTtlMs: 5000,
  viewScale: 1,
};

export class BrowserMockBridge implements Bridge {
  private state: TopState = {
    instances: [
      { id: "mock-1", name: "mock-a", status: "processing" },
      { id: "mock-2", name: "mock-b", status: "idle" },
    ],
    pendingNotifications: 0,
  };
  private listeners: ((s: TopState) => void)[] = [];
  private renderListeners: ((spec: ComponentSpec) => void)[] = [];
  private queueListeners: ((msgs: ContextMessage[]) => void)[] = [];
  private events: string[] = [];
  private queue: ContextMessage[] = [];

  async getConfig(): Promise<AppConfig> {
    return structuredClone(DEFAULT_CONFIG);
  }

  async getTopState(): Promise<TopState> {
    return structuredClone(this.state);
  }

  onTopStateChanged(cb: (s: TopState) => void): void {
    this.listeners.push(cb);
  }

  onRenderComponent(cb: (spec: ComponentSpec) => void): void {
    this.renderListeners.push(cb);
  }

  // ── mock Card 集合（Shelf 面板数据源；真实链路的 .card.json 注册表在 core） ──
  private mockCards = new Map<string, { spec: ComponentSpec; user_closed: boolean }>();
  private cardsListeners: (() => void)[] = [];

  pushEvent(ev: UserActionEvent): void {
    // mock 本地组合 debug 文本（生产由 core 按 Harness 语言现写，lifecycle 单源）
    this.events.push(mockActionDesc(ev));
    // 用户 × 关卡 / Shelf dismiss：出 mock 注册表（对应 core 的 cards_remove）
    if (ev.action === "dismiss" && ev.cardId && this.mockCards.delete(ev.cardId)) this.emitCards();
  }

  debugCallComponent(spec: ComponentSpec) {
    this.mockCards.set(spec.id, { spec: structuredClone(spec), user_closed: false });
    for (const cb of this.renderListeners) cb(structuredClone(spec));
    this.emitCards();
  }

  async listCards(): Promise<RestoredCard[]> {
    return [...this.mockCards.values()].map((c) => ({
      component: structuredClone(c.spec),
      user_closed: c.user_closed,
      layout: { direction: null, offset: null, manual: false },
    }));
  }

  async setCardUserClosed(id: string, userClosed: boolean): Promise<{ ok: boolean; error?: string }> {
    const c = this.mockCards.get(id);
    if (!c) return { ok: false, error: `Card '${id}' 不存在` };
    c.user_closed = userClosed;
    this.emitCards();
    return { ok: true };
  }

  onCardsChanged(cb: () => void): void {
    this.cardsListeners.push(cb);
  }

  private emitCards() {
    for (const cb of this.cardsListeners) cb();
  }

  debugEventBuffer(): string[] {
    return [...this.events];
  }

  debugFlushEventBuffer(): string[] {
    const out = [...this.events];
    this.events = [];
    return out;
  }

  async getContext(): Promise<ContextMessage[]> {
    return structuredClone(this.queue);
  }

  async appendUserMessage(text: string): Promise<boolean> {
    this.queue.push({ role: "user", content: text, ts: Date.now() });
    this.emitQueue();
    return true;
  }

  onContextChanged(cb: (msgs: ContextMessage[]) => void): void {
    this.queueListeners.push(cb);
  }

  debugAppendMessage(role: ContextMessage["role"], content: string) {
    this.queue.push({ role, content, ts: Date.now() });
    this.emitQueue();
  }

  private emitQueue() {
    const snapshot = structuredClone(this.queue);
    for (const cb of this.queueListeners) cb(snapshot);
  }

  private emit() {
    const snapshot = structuredClone(this.state);
    for (const cb of this.listeners) cb(snapshot);
  }

  /** 模拟 Hook 触发后 Ambery 更新的实例状态（真实链路后续由 Rust 内核喂入） */
  debugSetInstanceStatus(name: string, status: CodeCliStatus) {
    const inst = this.state.instances.find((i) => i.name === name);
    if (inst) inst.status = status;
    this.emit();
  }

  debugAddInstance(name: string, status: CodeCliStatus) {
    this.state.instances.push({ id: `mock-${Date.now()}`, name, status });
    this.emit();
  }

  debugNotify(n = 1) {
    this.state.pendingNotifications += n;
    this.emit();
  }

  debugClearNotifications() {
    this.state.pendingNotifications = 0;
    this.emit();
  }
}

/** Tauri 原生 IPC bridge（invoke + listen） */
class TauriBridge implements Bridge {
  private renderListeners: ((spec: ComponentSpec) => void)[] = [];
  private contextListeners: ((m: ContextMessage[]) => void)[] = [];
  private topStateListeners: ((s: TopState) => void)[] = [];
  private autonomyListeners: ((args: { face?: string; motion?: Motion; ttlMs?: number; once?: boolean }) => void)[] = [];
  private configListeners: ((cfg: AppConfig) => void)[] = [];
  private deltaListeners: ((d: { content?: string; reasoning_content?: string }) => void)[] = [];
  private doneListeners: (() => void)[] = [];
  private errorListeners: ((e: ErrorEvent) => void)[] = [];
  private closeListeners: ((id: string) => void)[] = [];
  private cardsListeners: (() => void)[] = [];

  constructor(
    private invokeFn: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>,
    listenFn: (event: string, cb: (ev: { payload: unknown }) => void) => Promise<unknown>,
  ) {
    void listenFn("effect", (ev) => {
      const msg = ev.payload as {
        kind?: string;
        spec?: ComponentSpec;
        face?: string;
        motion?: Motion;
        ttlMs?: number;
        once?: boolean;
        state?: TopState;
        content?: string;
        reasoning_content?: string;
        message?: string;
        retention?: "transient" | "persistent";
        action?: string | null;
        id?: string;
      };
      if (!msg?.kind) return;
      switch (msg.kind) {
        case "render_component":
          if (msg.spec) this.renderListeners.forEach((cb) => cb(msg.spec!));
          this.cardsListeners.forEach((cb) => cb());
          break;
        case "close_component":
          if (msg.id) this.closeListeners.forEach((cb) => cb(msg.id!));
          this.cardsListeners.forEach((cb) => cb());
          break;
        case "set_autonomy":
          this.autonomyListeners.forEach((cb) =>
            cb({ face: msg.face, motion: msg.motion, ttlMs: msg.ttlMs, once: msg.once }),
          );
          break;
        case "config":
          void this.getConfig().then((cfg) => this.configListeners.forEach((cb) => cb(cfg)));
          break;
        case "context_changed":
          void this.getContext().then((m) => this.contextListeners.forEach((cb) => cb(m)));
          break;
        case "top_state":
          if (msg.state) this.topStateListeners.forEach((cb) => cb(msg.state!));
          break;
        case "assistant_delta":
          this.deltaListeners.forEach((cb) =>
            cb({ content: msg.content, reasoning_content: msg.reasoning_content }),
          );
          break;
        case "assistant_done":
          this.doneListeners.forEach((cb) => cb());
          break;
        case "error":
          this.errorListeners.forEach((cb) =>
            cb({
              message: msg.message ?? "LLM 调用失败",
              retention: msg.retention === "persistent" ? "persistent" : "transient",
              action: msg.action ?? undefined,
            }),
          );
          break;
      }
    });
  }

  async getConfig(): Promise<AppConfig> {
    return this.invokeFn("get_config") as Promise<AppConfig>;
  }
  async getTopState(): Promise<TopState> {
    // wait_state 竞态期（run_core 未注入）保底：不炸前端，返回空态
    return (this.invokeFn("get_state") as Promise<TopState>).catch((e) => {
      console.error("[bridge] get_state", e);
      return { instances: [], pendingNotifications: 0 };
    });
  }
  async getContext(): Promise<ContextMessage[]> {
    return (this.invokeFn("get_context") as Promise<ContextMessage[]>).catch((e) => {
      console.error("[bridge] get_context", e);
      return [];
    });
  }
  async appendUserMessage(text: string): Promise<boolean> {
    try {
      await this.invokeFn("append_user", { text });
      return true;
    } catch (e) {
      console.error("[bridge] append_user", e);
      return false;
    }
  }
  pushEvent(ev: UserActionEvent): void {
    void this.invokeFn("push_event", {
      action: ev.action,
      cardId: ev.cardId,
      cardType: ev.cardType,
      title: ev.title,
      text: ev.text,
      target: ev.target,
      checked: ev.checked,
      stateSnapshot: ev.state,
    }).catch((e) => console.error("[bridge] push_event", e));
  }
  onRenderComponent(cb: (spec: ComponentSpec) => void): void {
    this.renderListeners.push(cb);
  }
  onContextChanged(cb: (m: ContextMessage[]) => void): void {
    this.contextListeners.push(cb);
  }
  onTopStateChanged(cb: (s: TopState) => void): void {
    this.topStateListeners.push(cb);
  }
  onSetAutonomy(cb: (args: { face?: string; motion?: Motion; ttlMs?: number; once?: boolean }) => void): void {
    this.autonomyListeners.push(cb);
  }
  onConfigChanged(cb: (cfg: AppConfig) => void): void {
    this.configListeners.push(cb);
  }
  onAssistantDelta(cb: (d: { content?: string; reasoning_content?: string }) => void): void {
    this.deltaListeners.push(cb);
  }
  onAssistantDone(cb: () => void): void {
    this.doneListeners.push(cb);
  }
  onError(cb: (e: ErrorEvent) => void): void {
    this.errorListeners.push(cb);
  }
  onCloseComponent(cb: (id: string) => void): void {
    this.closeListeners.push(cb);
  }
  async listCards(): Promise<RestoredCard[]> {
    // 恢复失败 = 无卡片（best-effort，不阻断 pet 启动）
    return (this.invokeFn("list_cards") as Promise<RestoredCard[]>).catch((e) => {
      console.error("[bridge] list_cards", e);
      return [];
    });
  }
  async setCardUserClosed(id: string, userClosed: boolean): Promise<{ ok: boolean; error?: string }> {
    return this.invokeFn("set_card_user_closed", { id, userClosed }) as Promise<{ ok: boolean; error?: string }>;
  }
  onCardsChanged(cb: () => void): void {
    this.cardsListeners.push(cb);
  }
  async getConfigSchema(): Promise<ConfigSchemaResp> {
    return this.invokeFn("get_config_schema") as Promise<ConfigSchemaResp>;
  }
  async setConfig(path: string, value: unknown): Promise<SetConfigResp> {
    return this.invokeFn("set_config", { path, value }) as Promise<SetConfigResp>;
  }

  async testLlm(): Promise<{ ok: boolean; reply?: string; error?: string }> {
    return this.invokeFn("test_llm") as Promise<{ ok: boolean; reply?: string; error?: string }>;
  }
  async getApiKeyStatus(provider: string): Promise<{ ok: boolean; set: boolean; source: string | null }> {
    return this.invokeFn("get_api_key_status", { provider }) as Promise<{ ok: boolean; set: boolean; source: string | null }>;
  }
  async setApiKey(provider: string, key: string | null): Promise<{ ok: boolean; error?: string }> {
    return this.invokeFn("set_api_key", { provider, key }) as Promise<{ ok: boolean; error?: string }>;
  }
  async updateCardLayout(id: string, offset: [number, number]): Promise<{ ok: boolean; error?: string }> {
    return this.invokeFn("update_card_layout", { id, offset }) as Promise<{ ok: boolean; error?: string }>;
  }
}

/** Tauri 模式 → TauriBridge（原生 IPC）；浏览器 → ambery-core 在跑用 RemoteBridge，否则内存 mock */
export async function createBridge(): Promise<Bridge> {
  if ("__TAURI_INTERNALS__" in window) {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");
    return new TauriBridge(invoke, listen);
  }
  const { RemoteBridge } = await import("./remote");
  if (await RemoteBridge.probe()) {
    const b = new RemoteBridge();
    b.connect();
    // 等首个 WS open 再交出桥——调用方拿到即可收发（否则紧接的 effect 可能在
    // WS 建立前到达而被丢失，时序竞态）
    await Promise.race([
      b.ready,
      new Promise((r) => setTimeout(r, 5000)),
    ]);
    return b;
  }
  return new BrowserMockBridge();
}
