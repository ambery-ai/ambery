// 丢弃式原型填充脚本（scroll-lab 用，不进正式实现）：把每段铺满骨架块，
// 让 180vh 的段有真实页面的密度——「平滑」有没有问题，得在内容密集时才看得出来。
(() => {
  const WIDTHS = ["a", "b", "c", "d"]; // 宽度档：别一样长，不然看着像栅栏
  const block = (kind) => {
    const el = document.createElement("div");
    el.className = kind === "tall" ? "blk tall" : "blk";
    el.innerHTML =
      '<span class="hd"></span>' +
      WIDTHS.map((w) => '<i class="' + w + '"></i>').join("") +
      '<span class="pad"></span>' +
      (kind === "tall" ? '<span class="bar"></span><span class="bar"></span>' : '<span class="bar"></span>');
    return el;
  };
  const grid = (count, offset, tallEvery) => {
    const wrap = document.createElement("div");
    wrap.className = "blocks";
    for (let n = 0; n < count; n += 1) wrap.append(block((n + offset) % tallEvery === 0 ? "tall" : ""));
    return wrap;
  };

  document.querySelectorAll("section").forEach((section, index) => {
    // 每段三排：两排矮块 + 一排高块，铺满 180vh（顶上一行是说明与读数）
    const anchor = section.querySelector("#fill-1") ?? section.querySelector(".rule") ?? section.lastElementChild;
    const grids = [grid(8, index, 3), grid(6, index + 1, 2), grid(4, index + 2, 1)];
    grids.forEach((g) => anchor.after(g));
  });
})();
