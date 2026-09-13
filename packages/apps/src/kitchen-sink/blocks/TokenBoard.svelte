<!-- 原型块：token 全表 + 覆盖面。
     每个 token 一颗芯片（颜色/圆角/投影按其性质呈现），并显示「当前主题下的实时取值」——
     切主题后重读，所以它能直接回答「这个组件吃没吃 token」。 -->
<script lang="ts">
  import { onMount } from "svelte";
  import { KNOWN_TOKENS } from "../../theme";

  let values = $state<Record<string, string>>({});

  function refresh() {
    const cs = getComputedStyle(document.documentElement);
    const next: Record<string, string> = {};
    for (const k of KNOWN_TOKENS) next[k] = cs.getPropertyValue(`--ov-${k}`).trim();
    values = next;
  }

  onMount(() => {
    refresh();
    const mo = new MutationObserver(refresh);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-kitchen-sink-theme"] });
    return () => mo.disconnect();
  });

  /** 芯片形态按 token 性质分：圆角给方框、投影给卡片、其余给色块 */
  type Chip = "swatch" | "radius" | "shadow";
  const chipOf = (key: string): Chip =>
    key.includes("radius") ? "radius" : key.includes("shadow") ? "shadow" : "swatch";

  /** 覆盖面：文件系统里有的 widget vs 本页清单，多出来的就是没摆上来的 */
  const widgetFiles = Object.keys(import.meta.glob("../../widgets/**/*.svelte"));
  const listedWidgets = [
    "widgets/button/Button.svelte",
    "widgets/dialog/Dialog.svelte",
    "widgets/input/Input.svelte",
    "widgets/panel/Panel.svelte",
    "widgets/select/Select.svelte",
    "widgets/tooltip/Tooltip.svelte",
  ];
  const unlisted = $derived(
    widgetFiles
      .map((p) => p.replace("../../", ""))
      .filter((p) => !listedWidgets.includes(p)),
  );
</script>

<section class="proto-block">
  <h2 class="proto-h">token 表 · {KNOWN_TOKENS.length} 个</h2>
  <p class="proto-note">
    取值从 <code>&lt;html&gt;</code> 现算——主题 = 覆写这张表，所以这里看到的就是全应用看到的。
  </p>
  <div class="proto-grid">
    {#each KNOWN_TOKENS as key (key)}
      <div class="proto-chip">
        {#if chipOf(key) === "swatch"}
          <span class="proto-swatch" style="background: var(--ov-{key})"></span>
        {:else if chipOf(key) === "radius"}
          <span class="proto-radius" style="border-radius: var(--ov-{key})"></span>
        {:else}
          <span class="proto-shadow" style="box-shadow: var(--ov-{key})"></span>
        {/if}
        <span class="proto-key">--ov-{key}</span>
        <span class="proto-value">{values[key] ?? "—"}</span>
      </div>
    {/each}
  </div>

  <h3 class="proto-h3">覆盖面</h3>
  <p class="proto-note">
    widget 文件 {widgetFiles.length} 份，本页陈列 {listedWidgets.length} 份{unlisted.length
      ? `；未陈列：${unlisted.join("、")}`
      : "；无遗漏"}。
  </p>
</section>

<style>
  .proto-block {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .proto-h {
    font-size: 13px;
    font-weight: 600;
    color: var(--ov-group);
    letter-spacing: 0.04em;
  }
  .proto-h3 {
    font-size: 12px;
    font-weight: 600;
    color: var(--ov-text-strong);
  }
  .proto-note {
    font-size: 11px;
    color: var(--ov-muted);
    max-width: 68ch;
  }
  code {
    font-family: Consolas, monospace;
    background: var(--ov-code-bg);
    border-radius: 3px;
    padding: 0 3px;
  }
  .proto-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
    gap: 6px;
  }
  .proto-chip {
    display: grid;
    grid-template-columns: 22px 1fr;
    grid-template-rows: auto auto;
    column-gap: 8px;
    align-items: center;
    padding: 4px 6px;
    border: 1px solid var(--ov-divider);
    border-radius: var(--ov-control-radius);
    background: var(--ov-panel-bg);
  }
  .proto-swatch,
  .proto-radius,
  .proto-shadow {
    grid-row: 1 / span 2;
    width: 22px;
    height: 22px;
    display: block;
  }
  .proto-swatch {
    border: 1px solid var(--ov-divider);
    border-radius: 4px;
  }
  .proto-radius {
    border: 2px solid var(--ov-accent);
  }
  .proto-shadow {
    border-radius: 4px;
    background: var(--ov-panel-bg);
  }
  .proto-key {
    font-family: Consolas, monospace;
    font-size: 10px;
    color: var(--ov-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .proto-value {
    font-family: Consolas, monospace;
    font-size: 10px;
    color: var(--ov-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
