// 抽取模块的验收探针：与实验 v2 同款序列（连滚过界 / 反向 / 掉头），读模块状态。
(async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const secs = [...document.querySelectorAll("section")];
  const out = { 视口: innerHeight, 段高: secs.map((s) => s.offsetHeight), 段界: secs[1]?.offsetTop };
  const wheel = (dy) => document.dispatchEvent(new WheelEvent("wheel", { deltaY: dy, deltaMode: 0, bubbles: true, cancelable: true }));
  const st = () => ({ ...window.__motion.state(), scrollY: Math.round(window.scrollY), dir: window.__motion.direction() });

  // 1) 从顶连滚 26 格：应一路推进，跨段时 glide
  window.scrollTo(0, 0);
  await wait(600);
  const down = [];
  for (let n = 1; n <= 26; n += 1) {
    wheel(120);
    await wait(140);
    if (n % 4 === 0) {
      const s = st();
      down.push([n, s.scrollY, s.target, s.gliding ? "G" : "-"].join(":"));
    }
  }
  await wait(1200);
  out["1_连滚26格"] = { 结束: Math.round(window.scrollY), 轨迹: down };

  // 2) 反向 8 格
  const up = [];
  for (let n = 1; n <= 8; n += 1) {
    wheel(-120);
    await wait(140);
    if (n % 2 === 0) up.push(Math.round(window.scrollY));
  }
  await wait(1000);
  out["2_反向8格"] = { 结束: Math.round(window.scrollY), 轨迹: up };

  // 3) 段中起始延迟：静置后滚一格，逐帧采样
  window.scrollTo(0, 200);
  await wait(900);
  const rows = [];
  let stop = false;
  const sample = () => { rows.push([Math.round(performance.now()), Math.round(window.scrollY)]); if (!stop) requestAnimationFrame(sample); };
  const t0 = performance.now();
  requestAnimationFrame(sample);
  wheel(120);
  await wait(700);
  stop = true;
  await wait(60);
  const base = rows[0][1];
  const first = rows.find((r) => r[1] !== base);
  out["3_第一动ms"] = first ? Math.round(first[0] - t0) : "从未动";
  out["3_前12帧"] = rows.filter((_, i) => i % 2 === 0).slice(0, 12).map((r) => (r[0] - t0) + ":" + r[1]);

  out["模块"] = window.__probe();
  window.__probeOut = JSON.stringify(out, null, 1);
  return window.__probeOut;
})();
