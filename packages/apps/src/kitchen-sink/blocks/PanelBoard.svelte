<!-- 原型块：三个窗口面板（tier-2）按真实窗口尺寸摆在框里，用来判断密度与信息层级。
     状态来自 stubs.ts 的静态桩：渲染完全真实，动作全是 no-op。 -->
<script lang="ts">
  import ChatPanel from "../../components/chat-panel/ChatPanel.svelte";
  import MenuPanel from "../../components/menu-panel/MenuPanel.svelte";
  import ShelfPanel from "../../components/shelf-panel/ShelfPanel.svelte";
  import Panel from "../../widgets/panel/Panel.svelte";
  import { createChatStub, createMenuStub, createShelfStub } from "../stubs";
  import { noop } from "../noop";

  const chat = createChatStub();
  const menu = createMenuStub();
  const shelf = createShelfStub();
</script>

<section class="proto-block">
  <h2 class="proto-h">tier-2 面板 · 按真实窗口尺寸</h2>
  <div class="proto-strip">
    <figure class="proto-figure">
      <figcaption>Chat · 320 × 380</figcaption>
      <div class="proto-chat"><ChatPanel {chat} onClose={noop} /></div>
    </figure>
    <figure class="proto-figure">
      <figcaption>Menu（设置）· 380 × 560</figcaption>
      <div class="proto-menu">
        <Panel title="⚙ 设置" id="menu-panel" onClose={noop} closeTitle="关闭">
          <MenuPanel {menu} onTogglePet={noop} onQuit={noop} />
        </Panel>
      </div>
    </figure>
    <figure class="proto-figure">
      <figcaption>Shelf · 240 × 320</figcaption>
      <div class="proto-shelf"><ShelfPanel state={shelf.state} actions={shelf.actions} /></div>
    </figure>
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
  .proto-strip {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 16px;
  }
  .proto-figure {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  figcaption {
    font-family: Consolas, monospace;
    font-size: 10px;
    color: var(--ov-muted);
  }
  .proto-chat {
    width: 320px;
    height: 380px;
    overflow: hidden;
    border: 1px dashed var(--ov-divider);
    border-radius: var(--ov-panel-radius);
  }
  .proto-menu {
    width: 380px;
    height: 560px;
    overflow: hidden;
    border: 1px dashed var(--ov-divider);
    border-radius: var(--ov-panel-radius);
  }
  .proto-shelf {
    width: 240px;
    height: 320px;
    overflow: hidden;
    border: 1px dashed var(--ov-divider);
    border-radius: var(--ov-panel-radius);
  }
</style>
