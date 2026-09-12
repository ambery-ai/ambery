// 丢弃式原型探针（v=2）：把"一小格一小格往下"的每一次滚轮都记下来，看判定为什么没接管。
(async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const S = () => window.__lab.state();
  const secs = [...document.querySelectorAll("section")];
  const e = Math.round(secs[0].offsetTop + secs[0].offsetHeight - innerHeight);
  const out = { 视口: innerWidth + "x" + innerHeight, 第一条边线: e };

  // 日志数组：hook 到 __labLog（push 里会写），并记录每次滚轮后的状态
  window.__labLog = [];
  const rows = [];
  const wheel = (dy) => document.dispatchEvent(new WheelEvent("wheel", { deltaY: dy, deltaMode: 0, bubbles: true, cancelable: true }));

  window.__lab.setDoc(e - 400);
  await wait(900);
  rows.push({ 阶段: "起点", scrollY: S().scrollY, 目标位: S().target, 接管: S().gliding, 原因: S().snap });
  for (let n = 1; n <= 8; n += 1) {
    wheel(80);
    await wait(220);
    const s = S();
    rows.push({ 格: n, scrollY: s.scrollY, 目标位: s.target, 接管: s.gliding ? "是" : "", 原因: s.snap || "" });
  }
  await wait(1200);
  out["每次滚轮后"] = rows;
  out["SNAP 日志"] = window.__labLog.length ? window.__labLog : "从未触发接管";
  out["结束"] = { scrollY: S().scrollY, 边线: e, 差: S().scrollY - e, 原因: S().snap };
  out["读数"] = S().readout;
  window.__probe = JSON.stringify(out, null, 1);
  return window.__probe;
})();
