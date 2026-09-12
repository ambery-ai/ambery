<!-- 规格段的两张表（都要「逐项核对」，不是「看气质」）：
       ① token 对照表——同一个 token 在当前主题与参照主题下的取值并排（切主题即重算；
          用带 data-kitchen-sink-theme 的探针元素读另一套值，不闪屏、不动全局）；
       ② 组件状态矩阵——每件控件在常态/禁用/错误（只读）下的实际渲染。
     格子底色是中性衬底（故意不取设计 token），被检对象才浮得出来。 -->
<script lang="ts">
  import Button from "../../widgets/button/Button.svelte";
  import Input from "../../widgets/input/Input.svelte";
  import Panel from "../../widgets/panel/Panel.svelte";
  import Select from "../../widgets/select/Select.svelte";
  import { KNOWN_TOKENS } from "../../theme";
  import { noop } from "../noop";

  let { theme }: { theme: string } = $props();

  let probe: HTMLDivElement | null = $state(null);
  let values = $state<Record<string, string>>({});

  /** 探针读值：给探针换 data-kitchen-sink-theme，同一元素上就能算出那一套 token */
  function readTheme(name: string): Record<string, string> {
    const out: Record<string, string> = {};
    if (!probe) return out;
    probe.dataset.kitchenSinkTheme = name;
    const cs = getComputedStyle(probe);
    for (const k of KNOWN_TOKENS) out[k] = cs.getPropertyValue(`--ov-${k}`).trim();
    return out;
  }

  $effect(() => {
    theme; // 依赖：主题变了重算
    const current = readTheme(theme);
    if (probe) delete probe.dataset.kitchenSinkTheme;
    values = current;
  });
</script>

<div class="sheet">
  <h1>品种规格表</h1>

  <div id="proto-probe" bind:this={probe} aria-hidden="true"></div>

  <section>
    <h2>token · {theme}（{KNOWN_TOKENS.length} 个）</h2>
    <table>
      <thead>
        <tr><th>token</th><th>当前取值</th></tr>
      </thead>
      <tbody>
        {#each KNOWN_TOKENS as key (key)}
          <tr>
            <td class="k">--ov-{key}</td>
            <td class="v">{values[key] ?? "—"}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>

  <section>
    <h2>组件状态矩阵</h2>
    <table class="matrix">
      <thead>
        <tr><th>组件</th><th>常态</th><th>禁用</th><th>错误 / 只读</th></tr>
      </thead>
      <tbody>
        <tr>
          <td class="k">Button</td>
          <td><Button variant="quiet" onclick={noop}>quiet</Button> <Button variant="danger" onclick={noop}>danger</Button> <Button variant="close" onclick={noop}>×</Button></td>
          <td><Button variant="quiet" disabled>quiet</Button></td>
          <td class="dim">—</td>
        </tr>
        <tr>
          <td class="k">Input · text</td>
          <td><Input type="text" value="文本值" onCommit={noop} /></td>
          <td><Input type="text" value="禁用" disabled onCommit={noop} /></td>
          <td><div class="cfg-line"><Input type="text" class="bad" value="被 core 拒绝" onCommit={noop} /></div></td>
        </tr>
        <tr>
          <td class="k">Input · 其他型</td>
          <td><Input type="number" value="42" min={0} max={100} onCommit={noop} /></td>
          <td><Input type="password" value="sk-abc" disabled onCommit={noop} /></td>
          <td><Input type="checkbox" checked onCommit={noop} /></td>
        </tr>
        <tr>
          <td class="k">Select</td>
          <td><Select options={["zh", "en"]} value="zh" onChange={noop} /></td>
          <td><Select options={["只读"]} value="只读" readOnly onChange={noop} /></td>
          <td class="dim">下拉打开态需真实点击（本页关交互）</td>
        </tr>
        <tr>
          <td class="k">Panel</td>
          <td>
            <div class="mini">
              <Panel title="panel 档" onClose={noop}><div class="dim pad">内容</div></Panel>
            </div>
          </td>
          <td>
            <div class="mini">
              <Panel tone="popup"><div class="dim pad">popup 档</div></Panel>
            </div>
          </td>
          <td class="dim">两档差异：边框软硬 + 圆角内裁</td>
        </tr>
      </tbody>
    </table>
  </section>
</div>

<style>
  .sheet {
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 24px;
    min-height: 100vh;
  }
  h1 {
    font-size: 16px;
    font-weight: 600;
    color: var(--ov-text-strong);
  }
  h2 {
    font-size: 12px;
    font-weight: 600;
    color: var(--ov-group);
    margin-bottom: 8px;
  }
  #proto-probe {
    position: absolute;
    width: 0;
    height: 0;
    overflow: hidden;
  }
  table {
    width: 100%;
    max-width: 980px;
    border-collapse: collapse;
    font-size: 11px;
  }
  th {
    text-align: left;
    font-weight: 600;
    color: var(--ov-muted);
    padding: 4px 8px;
    border-bottom: 1px solid var(--ov-divider);
  }
  td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--ov-divider-soft);
    vertical-align: middle;
  }
  .k {
    font-family: Consolas, monospace;
    color: var(--ov-text-strong);
    white-space: nowrap;
  }
  .v {
    font-family: Consolas, monospace;
    color: var(--ov-muted);
    white-space: nowrap;
  }
  /* 状态矩阵的格子：底色取自主题 token（§原则「不自带颜色」） */
  .matrix td {
    background: var(--ov-code-bg);
  }
  .dim {
    color: var(--ov-muted);
    font-size: 10px;
  }
  .pad {
    padding: 6px;
  }
  .mini {
    width: 200px;
    height: 96px;
    overflow: hidden;
    border: 1px solid var(--ov-divider);
    border-radius: var(--ov-panel-radius);
  }
</style>
