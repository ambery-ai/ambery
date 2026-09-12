// Kitchen Sink 的阅读位移（契约见 docs/kitchen-sink.md §滚动）。
//
// 模型：整页是一篇文档，**页面高度就是内容高度**（不固定视口），滚动条是产物自己的那条、在窗口右缘。
// 位移由浏览器原生滚动承担，本模块只做两件事：
//   1) 记录滚动方向（复位落点由方向唯一决定）；
//   2) 每帧判「视口顶端与底端是否落在同一段里」——跨段就朝上一次滚动方向滑到边界：
//      往下滚 → 下一段起点对齐屏顶；往上滚 → 本段末尾对齐屏底。
//
// 两条实现约束（都是踩过的坑）：
//   · 段元素**每帧按选择器现取**。页面用 {#key} 重挂内容（语言切换）时会换掉节点，
//     接入时一次性抓住的旧节点会脱离文档、offsetTop/offsetHeight 全变成 0，复位判定于是永不成立。
//   · 判定与"取段号"的函数别用同名局部量遮蔽（const 不提升），否则调用到的其实是另一层的函数。
//
// 复位只发生在跨段时：位置已落在一段里、或已贴着边界时本模块不介入，读者的自由滚动（含停在段中）不被校正。

/** 一次复位的时长（秒）：定时的三次缓出；落点是算出来的位置，所以精确停在边界上 */
const RESET_SECONDS = 0.5;
/** 小于这个距离就不动（避免为 1~2px 的舍入差反复触发） */
const MIN_MOVE = 2;

const easeOutCubic = (p: number) => 1 - Math.pow(1 - p, 3);

export type ReadingScroll = {
  /** 当前落在第几段（0 起）：分页器等 UI 用它高亮 */
  current: () => number;
  /** 调试用：帧循环的实时状态（页面控制台可直接读） */
  state: () => Record<string, unknown>;
  /** 停掉帧循环与监听 */
  destroy: () => void;
};

/**
 * 在 `root` 里接入阅读位移：页面（document）滚动，`root` 内匹配 `sectionSelector` 的元素是各段。
 */
export function attachReadingScroll(root: HTMLElement, sectionSelector = ".tier"): ReadingScroll {
  let raf = 0;
  let dir = 1;
  let current = 0;
  let anim: { t0: number; from: number; to: number } | null = null;

  const viewH = () => window.innerHeight;
  const maxDoc = () => Math.max(0, document.documentElement.scrollHeight - viewH());
  /** 当前文档里的段（每帧现取，见文件头） */
  const tiers = () => [...root.querySelectorAll<HTMLElement>(sectionSelector)];

  /** 位置落在第几段（按段的页面坐标） */
  const sectionIndexOf = (y: number, sections: HTMLElement[]) => {
    let i = 0;
    sections.forEach((s, n) => {
      if (y >= s.offsetTop) i = n;
    });
    return i;
  };

  /** 跨段复位：方向决定落点（往下 → 下一段起点对齐屏顶；往上 → 本段末尾对齐屏底）。
   *  两种端头例外：往下只剩最后一段 → 落到页面末；往上已在第一段 → 落到页面顶。 */
  const resetFor = (top: number, sections: HTMLElement[]): number | null => {
    if (!sections.length) return null;
    const vh = viewH();
    const max = maxDoc();
    const sTop = sectionIndexOf(top, sections);
    const sBottom = sectionIndexOf(top + vh - 1, sections);
    if (sTop === sBottom) return null;
    const down = dir >= 0;
    const landing = down
      ? sBottom >= sections.length - 1
        ? max
        : sections[sBottom].offsetTop
      : sTop <= 0
        ? 0
        : sections[sTop].offsetTop + sections[sTop].offsetHeight - vh;
    const clamped = Math.max(0, Math.min(max, landing));
    return Math.abs(clamped - top) > MIN_MOVE ? clamped : null;
  };

  const tick = (now: number) => {
    const sections = tiers();
    if (anim) {
      const p = Math.min(1, (now - anim.t0) / (RESET_SECONDS * 1000));
      window.scrollTo(0, anim.from + (anim.to - anim.from) * easeOutCubic(p));
      if (p >= 1) {
        window.scrollTo(0, anim.to);
        anim = null;
      }
    } else {
      const landing = resetFor(window.scrollY, sections);
      if (landing !== null) anim = { t0: now, from: window.scrollY, to: landing };
    }
    const shown = Math.max(0, Math.min(sections.length - 1, sectionIndexOf(window.scrollY, sections)));
    if (shown !== current) current = shown;
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);

  // 滚轮只管两件事：记方向；复位期间同方向不接（换方向即交还）。
  const onWheel = (e: WheelEvent) => {
    const next = Math.sign(e.deltaY);
    if (!next) return;
    // 先记方向、再判掉头：顺序反了会把"这次手势"误判成掉头（实测落到反方向的边界）
    if (anim && next !== dir) anim = null; // 掉头：复位取消，位置交还读者
    dir = next;
  };
  root.addEventListener("wheel", onWheel, { passive: true });

  // 原生滚动（触屏 / 键盘 / 拖动）也要认方向：位置在动就说明方向
  let lastY = window.scrollY;
  const onScroll = () => {
    const y = window.scrollY;
    if (y > lastY + 1) dir = 1;
    else if (y < lastY - 1) dir = -1;
    lastY = y;
  };
  window.addEventListener("scroll", onScroll, { passive: true });

  return {
    current: () => current,
    state: () => {
      const sections = tiers();
      return {
        scrollY: Math.round(window.scrollY),
        dir,
        current,
        animating: !!anim,
        viewH: viewH(),
        maxDoc: Math.round(maxDoc()),
        offsets: sections.map((s) => [s.offsetTop, s.offsetHeight]),
        resetTo: anim ? Math.round(anim.to) : resetFor(window.scrollY, sections),
      };
    },
    destroy: () => {
      cancelAnimationFrame(raf);
      root.removeEventListener("wheel", onWheel);
      window.removeEventListener("scroll", onScroll);
    },
  };
}
