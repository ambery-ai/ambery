// 正式页（kitchen sink）验收：只允许一层滚动条（窗口那条），且跨段复位仍然成立。
(async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const secs = [...document.querySelectorAll(".tier")];
  const scrollables = [...document.querySelectorAll("*")].filter((el) => {
    const cs = getComputedStyle(el);
    const scrollable = /auto|scroll/.test(cs.overflowY) && el.scrollHeight > el.clientHeight + 1;
    return scrollable;
  });
  const out = {
    视口: innerWidth + "x" + innerHeight,
    段数: secs.length,
    段高: secs.map((s) => s.offsetHeight),
    文档高: document.documentElement.scrollHeight,
    可滚动容器: scrollables.map((el) => el.className || el.tagName),
    滚动条层数: scrollables.length,
  };
  const segOf = (y) => {
    let i = 0;
    secs.forEach((s, n) => {
      if (y >= s.offsetTop) i = n;
    });
    return i;
  };
  const straddle = () => segOf(window.scrollY) !== segOf(window.scrollY + innerHeight - 1);

  // 跨段处停住 → 应复位到边界
  const e0 = secs[0].offsetTop + secs[0].offsetHeight - innerHeight;
  window.scrollTo(0, Math.round(e0 * 0.4 + 200));
  await wait(1300);
  out["跨段复位"] = {
    现在: Math.round(window.scrollY),
    跨段: straddle() ? "仍跨段（错）" : "已复位（对）",
  };

  // 段中（非跨段）停住 → 不该被校正
  const mid = secs[1].offsetTop + 200;
  window.scrollTo(0, mid);
  await wait(1200);
  out["段中停住"] = {
    摆位: Math.round(mid),
    现在: Math.round(window.scrollY),
    判定: Math.abs(window.scrollY - mid) <= 2 ? "没被校正（对）" : "被校正了（错）",
    跨段: straddle() ? "跨段" : "同一段",
  };
  window.__probe = JSON.stringify(out, null, 1);
  return window.__probe;
})();
