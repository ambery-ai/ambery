<!-- 原型块：tier-1 widget 全陈列（六件 × 变体/状态）。
     回调一律 no-op（见 stubs.ts），控件仍是活的（可聚焦/输入），但什么都不发生。 -->
<script lang="ts">
  import Button from "../../widgets/button/Button.svelte";
  import Input from "../../widgets/input/Input.svelte";
  import Panel from "../../widgets/panel/Panel.svelte";
  import Select from "../../widgets/select/Select.svelte";
  import Tooltip from "../../widgets/tooltip/Tooltip.svelte";
  import { noop } from "../noop";
</script>

<section class="proto-block">
  <h2 class="proto-h">tier-1 widget · 6 件</h2>

  <div class="proto-spec">
    <span class="proto-label">Button</span>
    <div class="proto-items">
      <Button variant="quiet" onclick={noop}>quiet</Button>
      <Button variant="danger" onclick={noop}>danger</Button>
      <Button variant="close" onclick={noop}>×</Button>
      <Button variant="quiet" disabled>disabled</Button>
      <Tooltip text="只有符号的控件才配 Tooltip">
        {#snippet children({ props })}
          <Button variant="quiet" {...props} onclick={noop} aria-label="帮助">?</Button>
        {/snippet}
      </Tooltip>
    </div>
  </div>

  <div class="proto-spec">
    <span class="proto-label">Input</span>
    <div class="proto-row">
      <Input type="text" value="文本值" placeholder="占位符" onCommit={noop} />
      <Input type="number" value="42" min={0} max={100} onCommit={noop} />
      <Input type="password" value="sk-abcdef" onCommit={noop} />
      <Input type="checkbox" checked onCommit={noop} />
      <Input type="text" value="禁用态" disabled onCommit={noop} />
    </div>
  </div>

  <div class="proto-spec">
    <span class="proto-label">Select</span>
    <div class="proto-row">
      <Select options={["zh", "en"]} value="zh" onChange={noop} />
      <Select options={["deepseek", "moonshot", "ollama"]} value="deepseek" onChange={noop} />
      <Select options={["只读", "不可改"]} value="只读" readOnly onChange={noop} />
    </div>
  </div>

  <div class="proto-spec">
    <span class="proto-label">Panel</span>
    <div class="proto-row">
      <div class="proto-frame">
        <Panel title="panel 档（有边框的对话面板）" onClose={noop} closeTitle="关闭">
          <div class="proto-pad proto-dim">内容区</div>
        </Panel>
      </div>
      <div class="proto-frame">
        <Panel tone="popup">
          <div class="proto-pad proto-dim">popup 档（软边、圆角内裁、无标题栏）</div>
        </Panel>
      </div>
      <div class="proto-frame">
        <Panel title="带 headRight" onClose={noop}>
          {#snippet headRight()}
            <span class="proto-dim">状态字</span>
          {/snippet}
          <div class="proto-pad proto-dim">标题栏右侧可挂状态</div>
        </Panel>
      </div>
    </div>
  </div>
</section>

<style>
  .proto-block {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .proto-h {
    font-size: 13px;
    font-weight: 600;
    color: var(--ov-group);
    letter-spacing: 0.04em;
  }
  .proto-spec {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .proto-label {
    font-family: Consolas, monospace;
    font-size: 11px;
    color: var(--ov-muted);
  }
  .proto-items {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .proto-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
  }
  .proto-frame {
    width: 210px;
    height: 130px;
    overflow: hidden;
    border: 1px dashed var(--ov-divider);
    border-radius: var(--ov-panel-radius);
  }
  .proto-pad {
    padding: 8px;
    font-size: 12px;
  }
  .proto-dim {
    color: var(--ov-muted);
    font-size: 11px;
  }
</style>
