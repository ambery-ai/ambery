// 主题应用：切换 theme = 全应用立即覆写 --ov-* token 表。
// 纯视觉变更：只写 documentElement 内联 CSS 变量，不触碰窗口开关/位置/尺寸/布局记忆、
// 阅读位置、输入内容、Card 内容与可见性、pet 名称/表情或任何 Harness 行为。
// 应用规则：先清全部已知 token 的内联覆写（回到 styles.css :root 内置默认），再写
// 当前主题表的覆写——内置 dark 为空表，即「清干净 = dark 视觉」。

import type { AppConfig } from "./bridge";

/** 已知 token 清单（去 --ov- 前缀，与 styles.css :root 一一对应；scripts/lint-tokens.mjs 守卫漂移） */
export const KNOWN_TOKENS = [
  "bg",
  "panel-bg",
  "panel-border",
  "panel-border-soft",
  "panel-radius",
  "card-bg",
  "card-border",
  "card-radius",
  "text",
  "text-strong",
  "muted",
  "group",
  "code-bg",
  "accent",
  "accent-bg",
  "accent-border",
  "danger",
  "dismiss",
  "success",
  "ok",
  "warn",
  "error",
  "view-bg",
  "view-border",
  "input-bg",
  "input-border",
  "hover-bg",
  "divider",
  "divider-soft",
  "control-radius",
  "input-radius",
  "bubble-user",
  "bubble-assistant",
  "bubble-system",
  "thinking-border",
  "overlay-bg",
  "modal-bg",
  "modal-text",
  "chart-1",
  "chart-2",
  "chart-3",
  "chart-4",
] as const;

/** 应用 config 中的当前主题到本窗口（每个窗口在 store 基线与 config 变化时调用） */
export function applyTheme(cfg: AppConfig | null) {
  const root = document.documentElement;
  for (const k of KNOWN_TOKENS) root.style.removeProperty(`--ov-${k}`);
  const table = cfg?.themes?.[cfg.theme ?? "dark"];
  if (!table) return;
  for (const [k, v] of Object.entries(table)) {
    if ((KNOWN_TOKENS as readonly string[]).includes(k)) {
      root.style.setProperty(`--ov-${k}`, v);
    }
  }
}

/** 窗口接线单点：基线即应用 + config 变化即重应用（store 订阅） */
export function wireTheme(store: { config: AppConfig | null; onConfig(cb: (c: AppConfig) => void): void }) {
  applyTheme(store.config);
  store.onConfig(applyTheme);
}
