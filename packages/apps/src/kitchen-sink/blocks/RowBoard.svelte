<!-- 原型块：配置行族（tier-2 ConfigRow 的每个 kind 一支活样本 + ApiKeyRow 三种状态）。
     节点取自 menu 桩，保证与设置面板里看到的是同一批数据形状。 -->
<script lang="ts">
  import ApiKeyRow from "../../components/config-rows/ApiKeyRow.svelte";
  import ConfigRow from "../../components/config-rows/ConfigRow.svelte";
  import type { ConfigSchemaNode } from "../../bridge";
  import { createMenuStub } from "../stubs";
  import { noop } from "../noop";

  const menu = createMenuStub();
  const pools = menu.pools;

  const pick = (path: string): ConfigSchemaNode => {
    for (const g of menu.groups) {
      const found = g.nodes.find((n) => n.path === path);
      if (found) return found;
    }
    throw new Error(`原型样本缺节点：${path}`);
  };

  const rows: { node: ConfigSchemaNode; note: string }[] = [
    { node: pick("name"), note: "str" },
    { node: pick("view_scale"), note: "float" },
    { node: pick("max_tool_calls_per_turn"), note: "int" },
    { node: pick("ui.topmost.pet"), note: "bool" },
    { node: pick("ui_language"), note: "enum" },
    { node: pick("llm.active"), note: "enum + enumAdd（下拉里可新增）" },
    { node: pick("themes"), note: "map（只读呈现）" },
    { node: pick("kaomoji.system.happy.face"), note: "表情池行（带池间移动）" },
  ];
</script>

<section class="proto-block">
  <h2 class="proto-h">tier-2 配置行 · ConfigRow 每个 kind</h2>
  <div class="proto-rows">
    {#each rows as row (row.node.path)}
      <div class="proto-row">
        <span class="proto-kind">{row.note}</span>
        <ConfigRow
          node={row.node}
          readOnly={false}
          pools={pools}
          enumAdd={row.note.startsWith("enum +")
            ? { label: "新增 provider", onConfirm: async () => ({ ok: true }) }
            : null}
          applyValue={async () => true}
        />
      </div>
    {/each}
  </div>

  <h3 class="proto-h3">ApiKeyRow · provider key 行</h3>
  <div class="proto-rows">
    <div class="proto-row">
      <span class="proto-kind">已设置</span>
      <ApiKeyRow
        provider="deepseek"
        envName="AMBERY_DEEPSEEK_API_KEY"
        local={false}
        readOnly={false}
        loadStatus={async () => ({ set: true, source: "env 文件" })}
        save={async () => ({ ok: true })}
        onChanged={noop}
      />
    </div>
    <div class="proto-row">
      <span class="proto-kind">未设置</span>
      <ApiKeyRow
        provider="moonshot"
        envName="AMBERY_MOONSHOT_API_KEY"
        local={false}
        readOnly={false}
        loadStatus={async () => ({ set: false, source: null })}
        save={async () => ({ ok: true, error: "示例：写 env 文件失败" })}
        onChanged={noop}
      />
    </div>
    <div class="proto-row">
      <span class="proto-kind">本地端点</span>
      <ApiKeyRow
        provider="ollama"
        envName="AMBERY_OLLAMA_API_KEY"
        local={true}
        readOnly={false}
        loadStatus={async () => ({ set: true, source: "环境变量" })}
        save={async () => ({ ok: true })}
        onChanged={noop}
      />
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
  .proto-h3 {
    font-size: 12px;
    font-weight: 600;
    color: var(--ov-text-strong);
  }
  .proto-rows {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 560px;
  }
  .proto-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--ov-divider-soft);
  }
  .proto-kind {
    font-family: Consolas, monospace;
    font-size: 10px;
    color: var(--ov-muted);
  }
</style>
