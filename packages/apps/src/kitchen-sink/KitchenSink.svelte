<!-- Kitchen Sink——「全部组件与样式，一眼能看全吗」（契约见 docs/kitchen-sink.md）。
     版式：一种读法的两段（表面 → 规格）。每段正好一屏，整页是一篇文档、页面高度即内容高度，
     滚动条在窗口右缘；静止位置不跨段——每帧判"视口顶底是否同段"，跨段就朝滚动方向滑到边界
     （见 reading-scroll.ts）。
     页眉在两段开头各一次、读同一份 model。
     两条硬约束：
       1) 取消点击事件——样张区回调用 stubs.ts 的 no-op，且样张内容 pointer-events: none
          （页眉是 .page-chrome，仍然可点）；
       2) 不能拖拽——卡片走 buildCard 而非 ComponentManager 的定位/拖拽路径（cards.ts）。 -->
<script lang="ts">
  import { onMount } from "svelte";
  import { wireI18n } from "../i18n";
  import type { AppConfig } from "../bridge";
  import type { Store } from "../store";
  import CardWall from "./blocks/CardWall.svelte";
  import PanelBoard from "./blocks/PanelBoard.svelte";
  import RowBoard from "./blocks/RowBoard.svelte";
  import SpecTables from "./blocks/SpecTables.svelte";
  import TokenBoard from "./blocks/TokenBoard.svelte";
  import WidgetBoard from "./blocks/WidgetBoard.svelte";
  import Header from "./chrome/Header.svelte";
  import { attachReadingScroll } from "./reading-scroll";
  import { THEMES } from "./stubs";

  const params = new URLSearchParams(window.location.search);
  const startTheme = params.get("theme") ?? "dark";
  const startLang = params.get("lang") === "en" ? "en" : "zh";

  let theme = $state((THEMES as readonly string[]).includes(startTheme) ? startTheme : "dark");
  let lang = $state(startLang);
  let rev = $state(0);
  /** 样张区一律不可命中：契约要求本页不触发任何应用动作（见文件头两条硬约束）。 */
  let interactive = $state(false);
  let page = $state<HTMLElement | null>(null);

  onMount(() => {
    const el = page;
    if (!el) return;
    const reading = attachReadingScroll(el);
    // 调试入口：控制台里读实时状态（`__reading.state()`）
    (window as unknown as { __reading?: unknown }).__reading = reading;
    return () => reading.destroy();
  });

  // 语言走产物自己的 i18n：wireI18n 是它的接线单点，页面只补一份最小 store。
  let cfg: AppConfig = {
    kaomoji: { system: {}, user: {} },
    setAutonomyDefaultTtlMs: 60_000,
    viewScale: 0.5,
    uiLanguage: startLang,
  };
  let notifyConfig: ((c: AppConfig) => void) | null = null;
  const configStore = {
    get config() {
      return cfg;
    },
    onConfig(cb: (c: AppConfig) => void) {
      notifyConfig = cb;
    },
  } as unknown as Store;

  onMount(() => {
    // rerender 钩子：语言变化时整页重取文案（t() 在渲染期取值）
    wireI18n(configStore, () => (rev += 1));
    // 首屏渲染发生在 wire 之前，wire 定下的语言要重挂一次才落地
    rev += 1;
  });

  function setLang(next: string) {
    lang = next;
    cfg = { ...cfg, uiLanguage: next === "en" ? "en" : "zh" };
    notifyConfig?.(cfg);
  }

  $effect(() => {
    document.documentElement.dataset.kitchenSinkTheme = theme;
  });

  $effect(() => {
    const q = new URLSearchParams({ theme, lang });
    window.history.replaceState(null, "", `?${q}#kitchen-sink`);
  });
</script>

<div class="page" bind:this={page}>
  {#key rev}
    <section class="tier">
      <Header {theme} onTheme={(v) => (theme = v)} {lang} onLang={setLang} />
      <div class="band" class:static={!interactive}>
        <PanelBoard />
        <CardWall />
      </div>
    </section>

    <section class="tier">
      <Header {theme} onTheme={(v) => (theme = v)} {lang} onLang={setLang} />
      <div class="band" class:static={!interactive}>
        <SpecTables {theme} />
        <WidgetBoard />
        <RowBoard />
        <TokenBoard />
      </div>
    </section>
  {/key}
</div>

<style>
  .page {
    color: var(--ov-text);
  }
  /* 每段至少一屏；内容是长段落时整页变长——**滚动只有一层**（页面自己那条，在窗口右缘）。
     段内不设滚动区：那会多出一条段内滚动条（实测出现过两重滚动条）。 */
  .tier {
    position: relative;
    min-height: 100dvh;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--ov-divider);
  }
  .tier:first-child {
    border-top: none;
  }
  .band {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding: 14px 24px 64px;
  }
  /* 取消点击事件：样张内容一律不可命中，页眉（.page-chrome）单独放回可点。
     :global 是必须的：.band 的子元素是各区块组件的根节点，不属于本组件，
     写成跨组件选择器会被 Svelte 的 scoped CSS 判为无用选择器并整条删除。 */
  .band.static {
    pointer-events: none;
  }
  .band.static :global(.page-chrome) {
    pointer-events: auto;
  }
</style>
