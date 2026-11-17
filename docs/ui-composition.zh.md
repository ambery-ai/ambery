# UI Composition

[English](ui-composition.md) | 中文

> - **本文档范围**——本文定义 UI 代码如何分层与组合：Window 组件、widget 各层、样式组合，以及组件遵循的规则；它不定义 Surface 是什么、窗口如何创建与摆放、Card 尺寸如何推导、有哪些主题取值。每个 Surface 是什么归 [concepts.zh.md](../concepts.zh.md) §Surface 与 [multi-window.zh.md](multi-window.zh.md)；窗口创建、摆放与坐标单位归 [window-follow.zh.md](window-follow.zh.md)；Card 尺寸推导归 [card-window-size.zh.md](card-window-size.zh.md)；主题取值归 [theme.zh.md](theme.zh.md)；模块分层与数据流归 [module-storage-flow.zh.md](module-storage-flow.zh.md)。
> - **组件只承担渲染**——数据逻辑不进组件，复杂逻辑抽成纯模块或纯函数，组件只消费其结果。落到 UI 代码上的含义见 §组件原则。

## 分层

```
OS 窗口            host        壳负责创建、定尺寸、摆放、显示、隐藏与销毁
Window 组件        component   每个窗口一个——窗口的外观与挂载约定；组件世界的入口
Surface 内容       component   它里面渲染的业务组件树
widget 各层        component   原子件、复合件与业务件
DOM                rendering   用户看到的东西
```

- 窗口是宿主对象。组件从不创建、定尺寸、移动或显示窗口；那是窗口层的事（[module-storage-flow.zh.md](module-storage-flow.zh.md)）。
- Window 组件是窗口的外观与挂载约定：渲染外框、渲染恰好一个 Surface 的内容。窗口尺寸由宿主决定（`docs/pet-window-size.zh.md`、`docs/card-window-size.zh.md`、`docs/multi-window.zh.md`）；组件从不请求尺寸。标题栏是可选 chrome，不是窗口属性——有标题栏的窗口用 `Panel` 包住内容；pet 与 Cards Shelf 都没有，Card 的标题来自它的内容。
- 同一张 Card 只有一个组件：无论渲染在自己的窗口里，还是渲染在容器 Surface 里。

## 窗口装配

一个窗口只挂载一个组件。窗口需要的服务——bridge、store、主题、i18n、窗口 adapter——由它的入口在组件树之外创建；没有任何组件创建服务。形态的宿主接线——它按自身 DOM 构造的 adapter、它的监听、它的手势动作——在 DOM 就位后由窗口组件启动。

### 入口

```ts
// 窗口入口 —— 唯一创建服务的地方
const shell = await createWindowShell("shelf"); // bridge、store、theme、i18n、adapter
mount(ShelfWindow, { target: document.getElementById("app")!, props: { shell } });
```

入口挂载到页面根（`#app`），页面根填满窗口：窗口组件与它下面的面板高度都从它取。根停在 auto 高度会让这条链塌掉——它下面每一处 `flex: 1` 都按内容定尺寸，面板主体于是既不滚动也不裁切。

### 窗口组件

```svelte
<!-- ChatWindow.svelte —— 发布 shell、渲染，自己不算任何东西 -->
<script lang="ts">
  let { shell }: { shell: WindowShell } = $props();
  setContext(shellContext, shell);
</script>

<Window kind="chat" {shell}>
  <Panel title={t("chat.title")} onClose={() => void shell.adapter?.hide()}>
    <ChatPanel />
  </Panel>
</Window>
```

### 宿主

```rust
// 宿主：壳按投影尺寸创建窗口；页面从不改窗口尺寸
WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html#chat".into()))
    .inner_size(size.w, size.h)
    .build()?;
```

- `createWindowShell(kind)` 是模块不是组件：它拥有 IPC、store、主题与 i18n 的应用，以及窗口 adapter。窗口的数据逻辑只住在这里。
- 形态的宿主接线是壳层里的模块（`shell/kinds/<kind>`）：注册该窗口的监听、在需要自身 DOM 时换用自己构造的 adapter（pet 要测量已渲染的颜文字），并暴露窗口组件转发的手势。窗口组件在挂载后启动它。
- Window 组件通过 Svelte context 发布 shell——这是它唯一不是渲染的动作——渲染外框与内容，自己不计算任何东西。
- widget 从 props 或 context 读数据，经传入的回调上报事件；它从不创建服务。

## widget 各层

| 层 | 归属 | 例子 | 规则 |
|---|---|---|---|
| 0 行为原语 | bits-ui（依赖） | Select、Checkbox、Dialog、Tooltip | 行为、无障碍、定位；不带样式 |
| 1 widget | 本项目 | Button、Input、Panel、Select、Dialog、Tooltip | 变体在这里定义；widget 之间可以互相组合 |
| 2 业务件 | 本项目 | ChatPanel、MenuPanel、ConfigRow | 只做组合与业务语义 |

- 第 1 层 widget 要么给第 0 层原语套本项目的变体，要么在 bits-ui 没有对应物时自写。
- 依赖单向：2 → 1 → 0；第 2 层不得直接用第 0 层原语。
- 变体属于第 1 层。第 2 层只组合与摆放，不定义自己的外观规则。
- widget 的高度是声明的布局常量，绝不由内容撑开（[card-window-size.zh.md](card-window-size.zh.md) §内容块）。
- 出现于两处以上、或带有行为/无障碍契约的东西才做成 widget；一次性布局内联。
- `Panel` 渲染标题栏与关闭按钮；关闭意味着什么由调用方回调决定——隐藏窗口、dismiss 卡片——不是 widget 的决定。

## 样式组合

| 层 | 来源 | 产出 |
|---|---|---|
| token | `--ov-*` 表（主题的落点） | CSS 自定义属性 |
| 语义工具类 | 由 token 映射的 Tailwind theme | 工具类 |
| 原语状态 | bits-ui 的 data attribute | 状态选择器 |
| 变体 | tailwind-variants | 类名函数与 slot |
| 动效 | Svelte transition | `transition:`、`animate:flip` |
| 实例覆盖 | 调用方传入的 `class` | 局部调整 |

纪律：工具类只读 token，绝不写字面值；变体只用工具类与原语状态；实例覆盖只调整、不重定义；主题层不携带尺寸（[card-window-size.zh.md](card-window-size.zh.md) §重算触发）。滚动容器戴主题的滚动条——细条 + 主题的滑块色（`--ov-scrollbar-thumb`）——因为平台滚动条不跟主题，会在暗色面板上画出一条浅色轨道。透明窗口里面板轮廓就是可见边界：铺满窗口的层——模态遮罩——要带面板圆角（`--ov-panel-radius`），否则它会在桌面上画出方角。文本选中遵循 chrome 与内容的划分：chrome——标题、按钮、标签——不可选；内容——消息正文、Card 正文、代码——可选。

## 组件原则

组件只承担渲染；以下是把这条原则落到 UI 代码上：

- 组件不调 IPC、定位引擎、存储或 Harness；数据经 props 进来，经回调出去。
- 输入是 props（数据）与 snippet（结构）；输出是 DOM 与事件。
- 组件绝不推导尺寸；尺寸来自尺寸模型（[card-window-size.zh.md](card-window-size.zh.md)）。

## 命名与文件

- 文件名语义化、多词；一个 widget 一个文件，变体放在它旁边。
- 组件文件装组件；共享的纯逻辑放在 `.ts` 模块里。
- widget 的对外面是 props 与 snippet；调用方不用类名去寻址它的内部元素。
