// i18n：UI 自身文案的语言设施。
// 两个独立全局语言偏好中的 UI 半（harness_language 的 core 侧见各 core 模块）：
// ui_language 控制应用固有文案（设置/按钮/placeholder/状态提示/错误说明），切换即重渲染；
// 不翻译用户消息、用户命名、既有 Chat 历史、卡片内容或已生成 LLM 输出；
// 机器契约（tool name / Config path / JSON key / 枚举值 / kaomoji.system key）永不翻译。

import type { AppConfig } from "./bridge";
import type { Store } from "./store";

export type UiLanguage = "zh" | "en";

/** 字符串表（zh 为基准；en 为 AI 生成翻译） */
const zh = {
  // pet 正式默认名（名称，用户定案「不改」）：名称不参与翻译，两语言同值
  "pet.default-name": "pet",
  "chat.title": "聊天",
  "chat.offline": "⚠ 未连接到 core",
  "chat.thinking-title": "思考中（点击看思维链）",
  "chat.send": "发送",
  "chat.retry": "重试",
  "chat.send-failed": "发送失败（文字已保留在输入框，可继续编辑）",
  "chat.queued": "已排队等待处理（{n}）",
  "chat.new-messages": "↓ {n} 条新消息",
  "card.copy": "复制",
  "card.expand-diff": "展开 diff",
  "card.todo-placeholder": "新增条目…",
  "shelf.loading": "加载中…",
  "shelf.empty": "（无存活卡片）",
  "shelf.show": "显示",
  "shelf.hide": "隐藏",
  "shelf.dismiss-title": "dismiss（结束 Surface，忘记布局）",
  "menu.title": "⚙ 设置",
  "menu.close-title": "关闭",
  "menu.toggle-pet": "显示/隐藏",
  "menu.quit": "退出",
  "menu.loading": "加载中…",
  "menu.offline": "连不上 core",
  "menu.readonly": "只读降级模式：备份文件加载中，修改被拒绝",
  "menu.load-error": "配置文件外部载入失败：{error}（当前仍使用已加载配置；修复文件后自动重试）",
  "menu.restart-banner": "以下字段已保存，重启应用后生效：{paths}",
  "menu.move-title": "把「{key}」移到 {to} 池（原子移动）",
  "menu.need-restart": "⚠ {paths} 需重启",
  "menu.theme-group": "theme",
  "menu.theme-export": "导出当前主题",
  "menu.theme-import": "导入",
  "menu.theme-file-placeholder": "主题文件名",
  "menu.exported": "✓ 已导出 {path}",
  "menu.imported": "✓ 已导入 {name}",
  "menu.map-tag": "(map)",
};

const en: Record<keyof typeof zh, string> = {
  "pet.default-name": "pet", // 名称不参与翻译（名称）
  "chat.title": "Chat",
  "chat.offline": "⚠ Cannot reach core",
  "chat.thinking-title": "Thinking (click to view the reasoning trace)",
  "chat.send": "Send",
  "chat.retry": "Retry",
  "chat.send-failed": "Failed to send (your text is kept in the input for editing)",
  "chat.queued": "Queued for processing ({n})",
  "chat.new-messages": "↓ {n} new messages",
  "card.copy": "Copy",
  "card.expand-diff": "Expand diff",
  "card.todo-placeholder": "New item…",
  "shelf.loading": "Loading…",
  "shelf.empty": "(No live cards)",
  "shelf.show": "Show",
  "shelf.hide": "Hide",
  "shelf.dismiss-title": "Dismiss (end Surface, forget layout)",
  "menu.title": "⚙ Settings",
  "menu.close-title": "Close",
  "menu.toggle-pet": "Show/Hide",
  "menu.quit": "Quit",
  "menu.loading": "Loading…",
  "menu.offline": "Cannot reach core",
  "menu.readonly": "Read-only fallback: loaded from backup; edits are rejected",
  "menu.load-error": "External config reload failed: {error} (current config still in use; it auto-retries once the file is fixed)",
  "menu.restart-banner": "Saved; takes effect after restart: {paths}",
  "menu.move-title": "Move \"{key}\" to the {to} pool (atomic move)",
  "menu.need-restart": "⚠ {paths} requires restart",
  "menu.theme-group": "theme",
  "menu.theme-export": "Export current theme",
  "menu.theme-import": "Import",
  "menu.theme-file-placeholder": "Theme file name",
  "menu.exported": "✓ Exported to {path}",
  "menu.imported": "✓ Imported {name}",
  "menu.map-tag": "(map)",
};

export type I18nKey = keyof typeof zh;

let current: UiLanguage = "zh";
const listeners = new Set<() => void>();

function fromConfig(cfg: AppConfig | null): UiLanguage {
  return cfg?.uiLanguage === "en" ? "en" : "zh";
}

/** 当前 UI 语言（未 wire 时为项目默认 zh） */
export function uiLanguage(): UiLanguage {
  return current;
}

/** 文案查表：当前语言命中即用，缺失回退 zh；{param} 插值 */
export function t(key: I18nKey, params?: Record<string, string>): string {
  const table: Record<string, string> = current === "en" ? en : zh;
  let s = table[key] ?? zh[key];
  for (const [k, v] of Object.entries(params ?? {})) s = s.replaceAll(`{${k}}`, v);
  return s;
}

/** 语言变化订阅（组件级原地 relabel 用） */
export function onLanguageChange(cb: () => void): void {
  listeners.add(cb);
}

/** 窗口接线单点：基线定语言 + store config 变化时切语言并触发重渲染 */
export function wireI18n(store: Store, rerender?: () => void) {
  current = fromConfig(store.config);
  if (rerender) listeners.add(rerender);
  store.onConfig((cfg) => {
    const l = fromConfig(cfg);
    if (l !== current) {
      current = l;
      for (const cb of listeners) cb();
    }
  });
}
