// 丢弃式原型探针：v=1 回归（段内收尾 τ=0.7 + 跨段接管落点）。
(async () => {
  const S = () => window.__lab.state();
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const board0 = document.querySelector(".board");
  const out = {};

  // 段内：松手后的收尾长度
  window.__lab.setInner(600);
  await wait(500);
  const rows = [];
  const w = (dy) => document.querySelector(".canvas, main, #scroller").dispatchEvent(new WheelEvent("wheel", { deltaY: dy, deltaMode: 0, bubbles: false, cancelable: true }));
  w(120);
  const t0 = performance.now();
  for (let n = 0; n < 40; n += 1) {
    rows.push([Math.round(performance.now() - t0), Math.round(board0.scrollTop)]);
    await wait(16);
  }
  const steps = rows.map((r, n) => (n === 0 ? 0 : r[1] - rows[n - 1][1]));
  out["1_段内收尾ms"] = rows.reduce((acc, r, n) => (Math.abs(steps[n]) > 1 ? r[0] : acc), 0);
  out["1_逐帧位移"] = steps.slice(0, 16).join(" ");
  out["1_总位移"] = Math.round(board0.scrollTop) - 600;

  // 段底再滚一格：应接管并落到下一段起点（整数屏）
  const innerMax = board0.scrollHeight - board0.clientHeight;
  window.__lab.setInner(innerMax);
  await wait(600);
  out["2_停在段底"] = S();
  w(40);
  await wait(150);
  out["3_跨段当场"] = S();
  await wait(1000);
  out["4_跨段落点"] = S();
  out["4_落点是否整数屏"] = out["4_跨段落点"].outer % document.getElementById("scroller").clientHeight === 0 ? "是（对）" : "否（错）";

  out["读数"] = S().readout;
  window.__probe = JSON.stringify(out, null, 1);
  return window.__probe;
})();
