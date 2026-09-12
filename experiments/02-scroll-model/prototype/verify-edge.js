// 实验 02 探针：跨段停点必须被复位（v=2 的关键验收）。
// 摆到若干个"视口压着两段"的位置，停一会儿，看是否滑到最近的那条段边界。
(async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const S = () => window.__lab.state();
  const secs = [...document.querySelectorAll("section")];
  const edges = secs.slice(0, -1).map((s) => s.offsetTop + s.offsetHeight - innerHeight);
  const out = { 视口: innerHeight, 边线: edges.map((e) => Math.round(e)), 复位结果: [] };
  for (const top of [edges[0] + 300, edges[1] + 300, edges[2] + 300]) {
    window.__lab.setDoc(Math.round(top));
    await wait(1600);
    const st = S();
    const near = edges.reduce((a, e) => (Math.abs(e - st.scrollY) < Math.abs(a - st.scrollY) ? e : a), edges[0]);
    out.复位结果.push({
      摆到: Math.round(top),
      现在: st.scrollY,
      snap: st.snap,
      最近边界: Math.round(near),
      差: Math.round(st.scrollY - near),
    });
  }
  out["读数"] = S().readout;
  window.__probe = JSON.stringify(out, null, 1);
  return window.__probe;
})();
