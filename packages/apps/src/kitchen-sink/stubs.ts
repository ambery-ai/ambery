// 原型桩：本页唯一的「数据源」。
// 「取消点击事件」的机制不是拦事件，而是没有真实现——每个回调都是 no-op，
// 没有 bridge、没有 IPC、没有窗口动作，所以点什么都不会发生（控制条除外）。
// 面板类组件要的是它们的 state/actions 形状，这里手写满足接口的静态对象。
import type { ConfigSchemaNode, ContextMessage, RestoredCard } from "../bridge";
import type { ChatState } from "../shell/kinds/chat-state.svelte";
import type { MenuState } from "../shell/kinds/menu-state.svelte";
import type { ShelfState } from "../shell/kinds/shelf-state.svelte";
import type { SetupState } from "../shell/kinds/setup-state.svelte";
import type { ShelfActions } from "../components/shelf-panel/shelf-actions";
import { CARD_SAMPLES } from "./cards";
import { noop } from "./noop";

/** 主题名：目前只有内置 dark（空表 = styles/tokens.css 的 :root 默认）。
    真实主题（身份 × 明度、value 字段清单）归 T19——本页不预设任何配色。 */
export const THEMES = ["dark"] as const;

/** 桌面地：透明窗口里判半透明/投影/轮廓必须能换底（样式在 page.css） */
export const GROUNDS = ["checker", "wall-dark", "wall-light", "white", "black"] as const;
export type ProtoGround = (typeof GROUNDS)[number];

// ── config schema 样例：每个 kind 一份，配置行的各条分支都要有活样本 ──
const node = (
  path: string,
  type: ConfigSchemaNode["type"],
  value: unknown,
  desc?: string,
): ConfigSchemaNode => ({ path, type, value, desc });

const KAOMOJI_SYSTEM = { happy: { face: "(＾▽＾)", motion: "float" }, sad: { face: "(；ω；)", motion: "still" } };

export function createMenuStub(): MenuState {
  return {
    loading: false,
    offline: false,
    readOnly: false,
    loadError: null,
    restartRequired: ["timer.interval_ms"],
    groups: [
      {
        name: "__top",
        nodes: [
          node("name", { kind: "str" }, "pet", "pet 名称：稳定身份值，不参与翻译"),
          node("view_scale", { kind: "float", min: 0.2, max: 4 }, 0.5, "View 缩放"),
        ],
      },
      {
        name: "ui",
        nodes: [
          node("ui.topmost.pet", { kind: "bool" }, true, "pet 窗口置顶"),
          node("ui_language", { kind: "enum", options: ["zh", "en"] }, "zh", "应用固有文案的语言"),
          node("badge_style", { kind: "enum", options: ["number", "bubble"] }, "number", "未读角标样式"),
          node("theme", { kind: "enum", options: ["dark", "light", "paper"] }, "dark", "当前主题名"),
          node("themes", { kind: "map" }, { dark: {}, light: "{…30 项}" }, "主题名 → token 覆写表"),
        ],
      },
      {
        name: "llm",
        nodes: [
          node("llm.active", { kind: "enum", options: ["deepseek", "moonshot", "ollama"] }, "deepseek", "当前 provider"),
          node("llm.providers.deepseek.model", { kind: "str" }, "deepseek-chat", "模型名"),
          node("max_tool_calls_per_turn", { kind: "int", min: 1 }, 12, "单轮工具调用预算"),
        ],
      },
      {
        name: "kaomoji",
        nodes: [
          node("kaomoji.system.happy.face", { kind: "str" }, "(＾▽＾)", "表情面（可在池间原子移动）"),
          node("kaomoji.system.happy.motion", { kind: "enum", options: ["still", "float", "bounce", "shake"] }, "float", "该表情的动作"),
        ],
      },
    ],
    pools: { system: KAOMOJI_SYSTEM, user: { think: { face: "(・_・;)", motion: "still" } } },
    apiKeyRows: [
      { group: "llm", provider: "deepseek", envName: "AMBERY_DEEPSEEK_API_KEY", local: false },
      { group: "llm", provider: "ollama", envName: "AMBERY_OLLAMA_API_KEY", local: true },
    ],
    status: { text: "配置已保存", cls: "ok" },
    theme: "dark",
    load: async () => {},
    setStatus: noop,
    apply: async () => true,
    apiKeyStatus: async () => ({ set: true, source: "env 文件" }),
    apiKeySave: async () => ({ ok: true }),
    exportTheme: async () => ({ ok: true, path: "~/.config/ambery/themes/dark.theme.json" }),
    importTheme: async () => ({ ok: true, name: "light" }),
  };
}

const MESSAGES: ContextMessage[] = [
  { role: "user", content: "现在 pull 一下我们继续其他推进", ts: 1 },
  { role: "assistant", content: "拉好了，远端有两刀我没做过的提交……", ts: 2 },
  { role: "tool", content: "[call_component] text_card / git_display", ts: 3 },
  { role: "system", content: "[观察] 用户在 mdb-nocontroller-3.3-wt·a2158254 输入：好了吗", ts: 4 },
  { role: "user", content: "卡片更新后窗口没有跟着变高，底部被裁掉了", ts: 5 },
];

export function createChatStub(): ChatState {
  return {
    messages: MESSAGES,
    offline: false,
    optimisticUsers: [{ text: "（这条是乐观回显，还没落 Context）", ts: 6 }],
    streamingText: "正在读窗口尺寸投影……",
    thinkingText: "先确认 card 文件的 _meta.layout.size 与 layoutVersion 是否 stale；" +
      "再判断壳侧是否在 render 之后重新投影尺寸——契约说 Card 窗口永不自己 resize，所以这条链路是壳的责任。",
    thinking: true,
    replying: true,
    queued: 2,
    errorBubbles: [{ ts: 7, message: "TIMER 判死被拒绝：枚举为空，信念不动" }],
    sendFailed: null,
    banner: { text: "LLM 未配置，点击打开配置引导", action: "setup", reportState: "setup" },
    unconfigured: false,
    rev: 3,
    userClosed: false,
    visible: true,
    onOpenSetup: null,
    onIntentClose: null,
    send: async () => "sent",
    clearSendFailed: noop,
    clearBanner: noop,
    showSetupBanner: noop,
    showSetupError: noop,
    setUnconfigured: noop,
    show: noop,
    intentClose: noop,
    intentOpen: noop,
    systemHide: noop,
    systemRestore: () => true,
  };
}

export function createShelfStub(): { state: ShelfState; actions: ShelfActions } {
  const cards: RestoredCard[] = Object.values(CARD_SAMPLES).map((component) => ({
    component,
    user_closed: false,
    layout: { direction: null, offset: null, manual: false },
  }));
  return {
    state: { cards, load: async () => {} },
    actions: {
      list: async () => cards,
      setUserClosed: async () => {},
      dismiss: async () => {},
      onCardsChanged: noop,
    },
  };
}

export function createSetupStub(): SetupState {
  return {
    loading: false,
    offline: false,
    readOnly: false,
    activeNode: node("llm.active", { kind: "enum", options: ["deepseek", "moonshot", "ollama"] }, "deepseek", "当前 provider"),
    provider: "deepseek",
    providerNodes: [
      node("llm.providers.deepseek.base_url", { kind: "str" }, "https://api.deepseek.com", "接口地址"),
      node("llm.providers.deepseek.model", { kind: "str" }, "deepseek-chat", "模型名"),
      node("llm.providers.deepseek.api_key_env", { kind: "str" }, "AMBERY_DEEPSEEK_API_KEY", "key 环境变量名"),
    ],
    envName: "AMBERY_DEEPSEEK_API_KEY",
    local: false,
    testText: "✓ 连通正常（deepseek-chat 已应答）",
    testClass: "ok",
    testing: false,
    load: async () => {},
    apply: async () => true,
    addProvider: async () => ({ ok: true }),
    apiKeyStatus: async () => ({ set: true, source: "env 文件" }),
    apiKeySave: async () => ({ ok: true }),
    runTest: async () => {},
  };
}
