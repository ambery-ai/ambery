<!-- 原型块：卡片墙——枚举 card 类型，每种一张真卡（渲染走生产 renderer）。
     扩展性：类型清单来自 CARD_SAMPLES（Record<ComponentSpec["type"], …>，编译期强制齐全），
     所以将来加一种 card 只会发生两件事：tsc 要求补样例 → 本墙自动多一张。
     不用 ComponentManager 的公开路径（那会带定位与拖拽），只取它的 buildCard。 -->
<script lang="ts">
  import { onMount } from "svelte";
  import { CARD_SAMPLES, CARD_TYPES, cardEl } from "../cards";

  let host: HTMLDivElement | null = $state(null);

  onMount(() => {
    if (!host) return;
    const children: HTMLElement[] = [];
    for (const type of CARD_TYPES) {
      const wrap = document.createElement("div");
      wrap.className = "proto-cell";
      const label = document.createElement("span");
      label.className = "proto-kind";
      label.textContent = type;
      const card = cardEl(CARD_SAMPLES[type]);
      wrap.append(label, card);
      children.push(wrap);
    }
    host.replaceChildren(...children);
    return () => host?.replaceChildren();
  });
</script>

<section class="proto-block">
  <h2 class="proto-h">Component / card · {CARD_TYPES.length} 类</h2>
  <p class="proto-note">
    真渲染器产物（非复刻）；点击处理器挂在桩上，且默认关交互，所以点不动。
  </p>
  <div class="proto-wall cards-mode" bind:this={host}></div>
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
  .proto-note {
    font-size: 11px;
    color: var(--ov-muted);
    max-width: 68ch;
  }
  /* card.css 的 .cards-mode 是「卡片窗内流式布局」：height:100vh 会撑爆这一格，
     故按类名叠加覆盖（卡片仍是 static 流式，正是想要的）。 */
  .proto-wall.cards-mode {
    height: auto;
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 16px;
  }
  .proto-wall :global(.proto-cell) {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .proto-wall :global(.proto-kind) {
    font-family: Consolas, monospace;
    font-size: 10px;
    color: var(--ov-muted);
  }
</style>
