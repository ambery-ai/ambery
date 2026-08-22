// 自定义下拉组件（config-reflect.ts enum 控件的实现）：
// 原生 select 的选项弹出层是 OS 级 popup，在 alwaysOnTop + 500ms TOPMOST 重申
// 协调器（docs/tauri-shell.md）下会被窗口本体盖住。改为 DOM 内 div 列表渲染——
// 它是窗口表面的一部分，抬窗口时一起抬，盖不住。视觉复刻原生 select。
// 列表契约（styles.css .cfg-select-list）：position:fixed，JS 按按钮 rect 定位，
// append 到 body，逃出 modal/滚动容器，像原生 popup 一样浮在上层。

let openDropdown: HTMLElement | null = null;
let openBtn: HTMLElement | null = null;
function closeDropdown() {
  openDropdown?.remove();
  openDropdown = null;
  openBtn = null;
}
document.addEventListener("click", (e) => {
  if (openDropdown && !(e.target as HTMLElement).closest(".cfg-select, .cfg-select-list")) closeDropdown();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeDropdown();
});

export interface CustomSelectOpts {
  options: readonly string[];
  value: string;
  readOnly: boolean;
  onChange: (v: string) => void;
}

/** 枚举下拉控件：按钮视觉复刻原生 select；箭头 = 内联 SVG 下 chevron
 *  （.cfg-select-arrow，stroke="currentColor" 跟随主题）；列表 fixed 浮层。 */
export function createCustomSelect(opts: CustomSelectOpts): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "cfg-select";
  const btn = document.createElement("button");
  btn.className = "cfg-select-btn";
  btn.type = "button";
  btn.disabled = opts.readOnly;
  btn.textContent = opts.value;

  // 朝下 chevron 箭头：内联 SVG（stroke="currentColor" 作为 SVG 属性解析可靠）
  const arrow = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  arrow.setAttribute("class", "cfg-select-arrow");
  arrow.setAttribute("viewBox", "0 0 10 6");
  arrow.setAttribute("width", "10");
  arrow.setAttribute("height", "6");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M1 1l4 4 4-4");
  path.setAttribute("stroke", "currentColor");
  path.setAttribute("stroke-width", "2");
  path.setAttribute("fill", "none");
  path.setAttribute("stroke-linecap", "round");
  path.setAttribute("stroke-linejoin", "round");
  arrow.appendChild(path);
  btn.appendChild(arrow);
  wrap.appendChild(btn);

  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    if (openDropdown && openBtn === btn) {
      closeDropdown(); // 同一下拉再点 → toggle 关闭
      return;
    }
    if (openDropdown) closeDropdown(); // 其他下拉开着 → 关了换这个
    openBtn = btn;
    const r = btn.getBoundingClientRect();
    const list = document.createElement("div");
    list.className = "cfg-select-list";
    list.style.left = `${r.left}px`;
    list.style.top = `${r.bottom + 2}px`;
    list.style.width = `${r.width}px`;
    list.style.maxHeight = "220px";
    // 列表 append 到 body，不继承配置行上下文——按按钮计算字体复制，和原生 select 同字号
    const cs = getComputedStyle(btn);
    list.style.fontSize = cs.fontSize;
    list.style.fontFamily = cs.fontFamily;
    for (const o of opts.options) {
      const item = document.createElement("div");
      item.className = "cfg-select-option" + (o === opts.value ? " selected" : "");
      item.textContent = o;
      item.addEventListener("click", (e) => {
        e.stopPropagation();
        closeDropdown();
        if (o !== opts.value) opts.onChange(o);
      });
      list.appendChild(item);
    }
    document.body.appendChild(list);
    openDropdown = list;
  });
  return wrap;
}
