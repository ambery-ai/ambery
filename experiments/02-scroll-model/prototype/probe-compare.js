// 对照探针：同一套滚轮序列，在"原版实验页"与"模块驱动精简页"上各跑一遍。
// 记录相对段界的偏移，便于两页（段高不同）直接比较。
(async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const secs = [...document.querySelectorAll("section")];
  const isModule = typeof window.__motion === "object" && typeof window.__motion.input === "function";
  const edge = Math.round(secs[1].offsetTop);
  const wheel = (dy) => window.dispatchEvent(new WheelEvent("wheel", { deltaY: dy, deltaMode: 0, bubbles: true, cancelable: true }));
  const y = () => Math.round(window.scrollY);
  const out = {
    页: isModule ? "模块精简版" : "原版内联",
    段高: secs[0].offsetHeight,
    视口: innerHeight,
    段界: edge,
  };

  // A) 段中一格：起始延迟 + 收尾逐帧
  window.scrollTo(0, 200);
  await wait(900);
  const rows = [];
  let stop = false;
  const sample = () => {
    rows.push([Math.round(performance.now()), y()]);
    if (!stop) requestAnimationFrame(sample);
  };
  const t0 = performance.now();
  requestAnimationFrame(sample);
  wheel(120);
  await wait(600);
  stop = true;
  await wait(60);
  const base = rows[0][1];
  const first = rows.find((r) => r[1] !== base);
  out["A_段中一格"] = {
    第一动ms: first ? Math.round(first[0] - t0) : "从未动",
    位移: y() - base,
    逐帧: rows.filter((_, i) => i % 2 === 0).slice(0, 8).map((r) => r[1] - base),
  };

  // B) 从段界上方 300px 往下连滚 12 格（相对段界的偏移）
  window.scrollTo(0, Math.max(0, edge - 300));
  await wait(900);
  const down = [];
  for (let n = 1; n <= 12; n += 1) {
    wheel(120);
    await wait(180);
    if (n % 3 === 0) down.push(y() - edge);
  }
  await wait(1000);
  out["B_往下12格（相对段界）"] = { 轨迹: down, 结束: y() - edge };

  // C) 反向 8 格
  const up = [];
  for (let n = 1; n <= 8; n += 1) {
    wheel(-120);
    await wait(180);
    if (n % 2 === 0) up.push(y() - edge);
  }
  await wait(900);
  out["C_往上8格（相对段界）"] = { 轨迹: up, 结束: y() - edge };

  out["引擎状态"] = isModule ? window.__motion.state() : "（原版内联，无该入口）";
  window.__probeOut = JSON.stringify(out, null, 1);
  return window.__probeOut;
})();
