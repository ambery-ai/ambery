// 卡片样例 + 真实渲染器取口（原型专用）。
// 扩展性机制：CARD_SAMPLES 是 Record<ComponentSpec["type"], ComponentSpec>——
// ComponentSpec 联合类型新增一种 card 而这里没补样例，tsc 立刻报缺键；
// 补了样例，卡片墙自动多出一张（无需改墙）。渲染走生产代码的 buildCard，
// 不复制一份 DOM 构造：本文件只是把私有的渲染方法取出来用，
// 走 ComponentManager 的公开路径会附带定位与拖拽，而本页要求不可拖。
import { ComponentManager } from "../components/component-manager";
import type { Bridge, ComponentSpec } from "../bridge";
import { noop } from "./noop";

export const CARD_SAMPLES: Record<ComponentSpec["type"], ComponentSpec> = {
  text_card: {
    id: "demo_text",
    type: "text_card",
    title: "text_card · 长文本",
    text: "存活实例 1：mdb-nocontroller-3.3-wt·a2158254\n状态：Processing\nproject=mdb-nocontroller-3.3-wt\n\n这是一段用来测试换行、字号与行距的正文。宽度上限 480，高度上限 = 屏高的一半减标题栏，超出在卡内滚动。",
  },
  quick_jump: {
    id: "demo_jump",
    type: "quick_jump",
    label: "跳到 mdb-nocontroller-3.3-wt",
    target: "mdb-nocontroller-3.3-wt",
  },
  git_display: {
    id: "demo_git",
    type: "git_display",
    title: "git_display · 最近提交",
    entries: [
      { hash: "5261221", msg: "shell: silence unused window handle warnings", time: "13:08" },
      { hash: "0f9ec46", msg: "core: silence compiler warnings", time: "13:05" },
      { hash: "83778f6", msg: "refactor(apps): move component styles into their components", time: "01:57" },
    ],
    diff: "--- a/packages/apps/src/styles/index.css\n+++ b/packages/apps/src/styles/card.css\n-  .cmp-header { … }\n+  .cmp-header { … }",
  },
  data_chart: {
    id: "demo_chart",
    type: "data_chart",
    title: "data_chart · 上下文占用",
    chart: {
      kind: "bar",
      labels: ["ctx", "queue", "cards", "errors"],
      series: [
        { name: "今日", data: [12, 5, 3, 1] },
        { name: "昨日", data: [9, 7, 5, 2] },
      ],
    },
  },
  todobox: {
    id: "demo_todo",
    type: "todobox",
    title: "todobox · 待办",
    items: [
      { text: "等待 mdb-nocontroller-3.3-wt 完成", done: true },
      { text: "出货形态复核样式结论", done: false },
      { text: "补 token 守卫覆盖面（card.css / .svelte）", done: false },
    ],
  },
};

/** 页面遍历用的类型清单（顺序即样例声明顺序） */
export const CARD_TYPES = Object.keys(CARD_SAMPLES) as ComponentSpec["type"][];

/** 只给 buildCard 用得到的两个成员：pushEvent（点击处理器）与 t()（文案） */
const stubBridge = { pushEvent: noop } as unknown as Bridge;

type CardBuilder = { buildCard(spec: ComponentSpec): HTMLDivElement };

let builder: ((spec: ComponentSpec) => HTMLDivElement) | null = null;

/** 取一次生产渲染器；离屏 mount 与 windowed=true 让管理器不订阅全局渲染流 */
function cardBuilder(): (spec: ComponentSpec) => HTMLDivElement {
  if (!builder) {
    const offscreen = document.createElement("div");
    const manager = new ComponentManager(offscreen, stubBridge, () => ({ x: 0, y: 0 }), true);
    builder = (manager as unknown as CardBuilder).buildCard.bind(manager);
  }
  return builder;
}

export function cardEl(spec: ComponentSpec): HTMLDivElement {
  return cardBuilder()(spec);
}
