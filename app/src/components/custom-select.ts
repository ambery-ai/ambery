// 自定义下拉组件（config-reflect.ts enum 控件的实现）：
// 原生 select 的选项弹出层是 OS 级 popup，在 alwaysOnTop + 500ms TOPMOST 重申
// 协调器（docs/tauri-shell.md）下会被窗口本体盖住。改为 DOM 内 div 列表渲染——
// 它是窗口表面的一部分，抬窗口时一起抬，盖不住。视觉复刻原生 select。
// 列表契约（styles.css .cfg-select-list）：position:fixed，JS 按按钮 rect 定位，
// append 到 body，逃出 modal/滚动容器，像原生 popup 一样浮在上层。
// addMode（T8 调研前的 stopgap）：下拉下方渲染「+」触发器（独立行靠右）；
// 点它弹一格输入+✓，位置/宽/样式与原下拉列表一致（盖住「+」本身）。
// 一格输入弹层抽为共享 openOneCellPopup（api-key 行同构复用，一致性见 elegant-code-analysis）。

let openDropdown: HTMLElement | null = null;
let openBtn: HTMLElement | null = null;
function closeDropdown() {
  openDropdown?.remove();
  openDropdown = null;
  openBtn = null;
}
document.addEventListener("click", (e) => {
  if (
    openDropdown &&
    !(e.target as HTMLElement).closest(".cfg-select, .cfg-select-list, .cfg-select-add-popup")
  ) {
    closeDropdown();
  }
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeDropdown();
});

export interface AddModeOpts {
  /** 「+」触发器 hover 提示 / 输入占位 */
  addLabel: string;
  /** 确认回调：返回 {ok} 成功关弹层；失败 {ok:false, error} 弹层内红字 */
  onAdd: (name: string) => Promise<{ ok: boolean; error?: string }>;
}

export interface CustomSelectOpts {
  options: readonly string[];
  value: string;
  readOnly: boolean;
  onChange: (v: string) => void;
  /** add 模式：下拉下方渲染「+」，弹一格输入+✓（config path grammar 校验内置） */
  addMode?: AddModeOpts;
}

const NAME_RE = /^[a-z][a-z0-9_-]*$/;

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

  // 选项列表（点按钮 toggle；fixed 浮层按按钮 rect 定位，append 到 body）
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    if (openDropdown && openBtn === btn) {
      closeDropdown();
      return;
    }
    if (openDropdown) closeDropdown();
    openBtn = btn;
    const r = btn.getBoundingClientRect();
    const list = document.createElement("div");
    list.className = "cfg-select-list";
    list.style.left = `${r.left}px`;
    list.style.top = `${r.bottom + 2}px`;
    list.style.width = `${r.width}px`;
    list.style.maxHeight = "220px";
    // 列表 append 到 body，不继承配置行上下文——按按钮计算字体复制
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

  // addMode：「+」触发器（独立行靠右），点它弹一格输入+✓
  if (opts.addMode && !opts.readOnly) {
    const addBtn = document.createElement("button");
    addBtn.className = "cfg-select-add-trigger";
    addBtn.type = "button";
    addBtn.textContent = "+";
    addBtn.title = opts.addMode.addLabel;
    addBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      openAddCell(opts, btn);
    });
    wrap.appendChild(addBtn);
  }

  return wrap;
}

export interface OneCellPopupOpts {
  /** 弹层定位基准元素（触发按钮） */
  anchor: HTMLElement;
  /** 输入占位 */
  placeholder: string;
  /** 确认前本地校验：返回错误文本则格内红字拦截；null 放行 */
  validate?: (value: string) => string | null;
  /** 确认回调：{ok} 成功关弹层；失败 {ok:false,error} 格内红字 */
  onConfirm: (value: string) => Promise<{ ok: boolean; error?: string }>;
  /** 定位覆盖：默认锚点 rect 下方同宽（addMode）；api-key 行右对齐 + 最小宽度 */
  position?: (r: DOMRect) => { left: number; top: number; width: number };
}

/** 一格输入弹层（addMode / api-key 行共用）：fixed 浮层盖住锚点，输入+✓，格内红字，Esc/点外关 */
export function openOneCellPopup(opts: OneCellPopupOpts): void {
  closeDropdown();
  const r = opts.anchor.getBoundingClientRect();
  const pos = opts.position ? opts.position(r) : { left: r.left, top: r.bottom + 2, width: r.width };
  const popup = document.createElement("div");
  popup.className = "cfg-select-add-popup";
  popup.style.left = `${pos.left}px`;
  popup.style.top = `${pos.top}px`;
  popup.style.width = `${pos.width}px`;
  // 弹层 append 到 body，不继承配置行主题——按锚点计算字体复制
  const cs = getComputedStyle(opts.anchor);
  popup.style.fontSize = cs.fontSize;
  popup.style.fontFamily = cs.fontFamily;

  const cell = document.createElement("div");
  cell.className = "cfg-select-add-cell";
  const input = document.createElement("input");
  input.className = "cfg-select-add-input";
  input.placeholder = opts.placeholder;
  const confirm = document.createElement("button");
  confirm.className = "cfg-select-add-confirm";
  confirm.type = "button";
  confirm.textContent = "✓";
  cell.append(input, confirm);

  const err = document.createElement("div");
  err.className = "cfg-select-add-err";
  err.hidden = true;

  popup.append(cell, err);
  document.body.appendChild(popup);
  openDropdown = popup;

  const doAdd = async () => {
    const value = input.value.trim();
    const vErr = opts.validate ? opts.validate(value) : null;
    if (vErr) {
      err.textContent = vErr;
      err.hidden = false;
      return;
    }
    try {
      const resp = await opts.onConfirm(value);
      if (!resp.ok) {
        err.textContent = resp.error ?? "写入失败";
        err.hidden = false;
        return;
      }
      closeDropdown();
    } catch (e) {
      err.textContent = String(e ?? "写入失败");
      err.hidden = false;
    }
  };
  confirm.addEventListener("click", (e) => {
    e.stopPropagation();
    void doAdd();
  });
  input.addEventListener("click", (e) => e.stopPropagation());
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.stopPropagation();
      void doAdd();
    }
  });
  input.focus();
}

/** addMode 一格弹层：锚点 = 下拉按钮，位置/宽/样式与原下拉列表一致（盖住「+」触发器） */
function openAddCell(opts: CustomSelectOpts, btn: HTMLElement) {
  openOneCellPopup({
    anchor: btn,
    placeholder: opts.addMode!.addLabel,
    validate: (value) =>
      NAME_RE.test(value) ? null : "名称不合法：小写字母开头，仅小写字母/数字/_/-",
    onConfirm: opts.addMode!.onAdd,
  });
}
