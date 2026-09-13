<!-- 页眉：整页的控件，两段开头各渲染一次、读同一份 model。三行：标题 / 主题（含底）/ 语言。
     遵循 §原则「全部平铺」：没有下拉、没有悬浮，候选项全部摆成一排排小框，点一次即生效。
     契约见 docs/kitchen-sink.md §版式 / §控件。 -->
<script lang="ts">
  import { THEMES } from "../stubs";

  let {
    theme,
    onTheme,
    lang,
    onLang,
  }: {
    theme: string;
    onTheme: (v: string) => void;
    lang: string;
    onLang: (v: string) => void;
  } = $props();

  const LANGS = ["zh", "en"] as const;
  const LANG_LABEL: Record<string, string> = { zh: "中", en: "EN" };

  /** 主题色带画哪几颗 token（顺序即条纹顺序）：底 / 面 / 文字 / 强调 / 危险 / 成功 / 用户气泡 / 助手气泡 / 图表 */
  const KEYS = [
    "bg",
    "panel-bg",
    "text",
    "accent",
    "danger",
    "success",
    "bubble-user",
    "bubble-assistant",
    "chart-1",
  ] as const;

  let swatches = $state<Record<string, string[]>>({});
  let probe = $state<HTMLElement | null>(null);

  $effect(() => {
    const el = probe;
    if (!el) return;
    const next: Record<string, string[]> = {};
    for (const name of THEMES) {
      el.dataset.kitchenSinkTheme = name;
      const cs = getComputedStyle(el);
      next[name] = KEYS.map((key) => cs.getPropertyValue(`--ov-${key}`).trim());
    }
    delete el.dataset.kitchenSinkTheme;
    swatches = next;
  });
</script>

<header class="head page-chrome">
  <div class="row">
    <span class="mark">kitchen sink</span>
  </div>

  <div class="row" role="radiogroup" aria-label="theme">
    <span class="cap">主题：</span>
    {#each THEMES as name (name)}
      <button
        class="box theme"
        class:on={name === theme}
        onclick={() => onTheme(name)}
        title={name}
        aria-label={name}
        aria-pressed={name === theme}
      >
        {#each swatches[name] ?? [] as color, i (i)}
          <span style="background: {color}"></span>
        {/each}
      </button>
    {/each}
  </div>

  <div class="row" role="radiogroup" aria-label="language">
    <span class="cap">语言：</span>
    {#each LANGS as code (code)}
      <button class="box lang" class:on={code === lang} onclick={() => onLang(code)} aria-pressed={code === lang}>
        {LANG_LABEL[code]}
      </button>
    {/each}
  </div>

  <div class="probe" bind:this={probe} aria-hidden="true"></div>
</header>

<style>
  /* 控件皮肤沿用 prototype 原有口径：与产物输入件同一套 token */
  .head {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 0 10px;
  }
  .row {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  /* 行标签加粗 + 冒号 */
  .cap {
    font-size: 11px;
    font-weight: 600;
    color: var(--ov-text-strong);
  }
  .mark {
    font-family: Consolas, monospace;
    font-size: 11px;
    letter-spacing: 0.08em;
    color: var(--ov-muted);
  }
  .box {
    padding: 3px 6px;
    font: inherit;
    font-size: 11px;
    color: var(--ov-text-strong);
    cursor: pointer;
    border: 1px solid var(--ov-input-border);
    border-radius: var(--ov-control-radius);
    background: var(--ov-input-bg);
  }
  .box.on {
    border-color: var(--ov-accent-border);
    background: var(--ov-accent-bg);
    color: var(--ov-accent);
  }
  /* 主题：一条色带就是那套主题的配色 */
  .theme {
    display: inline-flex;
    width: 96px;
    height: 20px;
    padding: 0;
    overflow: hidden;
    border-radius: var(--ov-control-radius);
  }
  .theme span {
    flex: 1 1 0;
  }
  .lang {
    min-width: 34px;
    text-align: center;
  }
  /* 探针：只用来算各套主题的 token 值（不可见、不占位） */
  .probe {
    position: absolute;
    width: 0;
    height: 0;
    overflow: hidden;
  }
</style>
