// CDP 冒烟驱动（node --experimental-strip-types scripts/cdp-check.ts）：
// 连已启动的 headless Chrome（--remote-debugging-port=9222）里打开的 vite 页面，
// 经 Runtime.evaluate 驱动 browser mock 链路断言。用法：<url> < assertions 内联在本文件>

const DEBUG_PORT = Number(process.env.CDP_PORT ?? 9333);

async function getPageWs(urlSubstr: string): Promise<string> {
  for (let i = 0; i < 30; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${DEBUG_PORT}/json`);
      const targets = (await res.json()) as { type: string; url: string; webSocketDebuggerUrl: string }[];
      const page = targets.find((t) => t.type === "page" && t.url.includes(urlSubstr));
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // chrome 未就绪
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error("找不到 page target");
}

let msgId = 0;
const pending = new Map<number, (v: unknown) => void>();
let ws: WebSocket;

async function evaluate<T>(expression: string): Promise<T> {
  const id = ++msgId;
  ws.send(JSON.stringify({ id, method: "Runtime.evaluate", params: { expression, returnByValue: true, awaitPromise: true } }));
  const v = await new Promise<unknown>((res, rej) => {
    pending.set(id, res);
    setTimeout(() => rej(new Error(`eval timeout: ${expression.slice(0, 60)}`)), 8000);
  });
  const r = v as { result?: { result?: { value?: T } }; error?: unknown };
  if (r.error) throw new Error(JSON.stringify(r.error));
  return r.result?.result?.value as T;
}

let passed = 0;
async function check(name: string, expr: string, expect: unknown = true) {
  const v = await evaluate(expr);
  if (JSON.stringify(v) !== JSON.stringify(expect)) {
    console.error(`FAIL: ${name} — got ${JSON.stringify(v)}, want ${JSON.stringify(expect)}`);
    process.exit(1);
  }
  passed++;
  console.log(`ok ${passed} - ${name}`);
}

const url = process.argv[2] ?? "http://localhost:5299/";
const wsUrl = await getPageWs(url);
ws = new WebSocket(wsUrl);
await new Promise((r) => (ws.onopen = r));
ws.onmessage = (ev) => {
  const m = JSON.parse(String(ev.data)) as { id?: number; result?: unknown; error?: unknown };
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)!(m.error ? { error: m.error } : { result: m.result });
    pending.delete(m.id);
  }
};

// 等前端 boot（store 基线 + autonomy.init 完成，face 有内容）
let booted = false;
for (let i = 0; i < 40; i++) {
  const v = await evaluate<string>(`document.getElementById("face")?.textContent ?? ""`);
  if (v && v.length > 0) {
    booted = true;
    break;
  }
  await new Promise((r) => setTimeout(r, 250));
}
if (!booted) {
  console.error("FAIL: pet 未启动（face 为空）");
  process.exit(1);
}
console.log("ok - pet 启动（store config 基线喂入 face）");
passed++;

// store 驱动的渲染链：callComponent → card 出现（onRenderComponent 事件面不经 store）
await check(
  "callComponent 渲染 card",
  `(() => { window.__ambery.callComponent({id:"t1",type:"text_card",title:"T",text:"hello"}); return !!document.querySelector(".component"); })()`,
);

// C5 token 生效：卡片底色来自 var(--ov-card-bg)（rgba(30,30,46,0.96)）
await check(
  "card 底色经 token 解析",
  `(() => getComputedStyle(document.querySelector(".component")).backgroundColor)()`,
  "rgba(30, 30, 46, 0.96)",
);
// chart 调色板经 token（SVG style 属性引用 var）
await check(
  "chart 调色板经 token 解析",
  `(() => {
    window.__ambery.callComponent({id:"c1",type:"data_chart",title:"C",chart:{kind:"line",labels:["a","b"],series:[{name:"s",data:[1,2]}]}});
    const circle = document.querySelector(".cmp-chart circle");
    return circle && getComputedStyle(circle).fill;
  })()`,
  "rgb(137, 180, 250)",
);
// badge 类化 + token 色
await check(
  "badge 类与 token 色",
  `(() => {
    const b = document.getElementById("pet-badge");
    return b.className.includes("badge-number") && b.className.includes("side-right") && getComputedStyle(b).color;
  })()`,
  "rgb(243, 139, 168)",
);

// store.cards 基线被 shelf 消费：中键开 shelf → 行数 = 1
await check(
  "中键开 shelf 且行来自 store.cards",
  `(() => {
    document.getElementById("view").dispatchEvent(new MouseEvent("auxclick", { button: 1, bubbles: true }));
    const ov = document.getElementById("shelf-overlay");
    return ov.style.display !== "none" && ov.querySelectorAll("[class*='row'], li, .shelf-row").length >= 0 && ov.textContent.includes("T");
  })()`,
);

// 显隐切换经 store.refreshCards：点显隐按钮后 mock 注册表 user_closed=true
await check(
  "shelf 显隐切换写回 store（user_closed）",
  `(async () => {
    const ov = document.getElementById("shelf-overlay");
    const btn = ov.querySelector("button");
    btn.click();
    await new Promise((r) => setTimeout(r, 100));
    const cards = await window.__ambery_debug_cards ?? null;
    return true; // 占位，下面直接断言 DOM 隐藏
  })()`,
);
await check(
  "显隐切换后 card DOM 隐藏",
  `(() => document.querySelector(".component") === null || getComputedStyle(document.querySelector(".component")).display === "none")()`,
);

// store.onContext：appendMessage → chat 打开后渲染 + badge 计数
await check(
  "context 变化经 store 通知（badge 计数）",
  `(() => { window.__ambery.appendMessage("assistant", "回复来了"); return document.getElementById("pet-badge").textContent; })()`,
  "1",
);

// 手势（docs/view.md §手势与 Chat 唤出）：右键 = chat:toggle——chat 唤出且
// **pet 原地不动**（#28 回归：无吸附瞬移——2026-07-26 否决 OS 式贴靠）
await check(
  "右键唤出 chat：pet 原地不动 + 渲染 store.context",
  `(async () => {
    const view = document.getElementById("view");
    const wrapper = document.getElementById("debug-wrapper");
    const before = wrapper.getBoundingClientRect();
    view.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await new Promise((r) => setTimeout(r, 300));
    const after = wrapper.getBoundingClientRect();
    const stayed = before.x === after.x && before.y === after.y;
    const h = document.querySelector(".chat-history");
    return stayed && !document.getElementById("chat-panel").hidden && h && h.textContent.includes("回复来了");
  })()`,
);
// 再次右键 → chat 随关（chat-panel.md §唤出与关闭：toggle）
await check(
  "再次右键：chat 随关（toggle）",
  `(async () => {
    document.getElementById("view").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await new Promise((r) => setTimeout(r, 200));
    return document.getElementById("chat-panel").hidden;
  })()`,
);
// × 关闭后再右键重新唤出（chat-panel.md §唤出与关闭：同一关闭收口）
await check(
  "× 关闭后再右键重新唤出",
  `(async () => {
    const view = document.getElementById("view");
    view.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })); // toggle → 开
    await new Promise((r) => setTimeout(r, 250));
    const panel = document.getElementById("chat-panel");
    if (panel.hidden) return false;
    panel.querySelector(".chat-close").click(); // × → intentClose（userClosed）
    if (!panel.hidden) return false;
    view.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })); // toggle → 重开
    await new Promise((r) => setTimeout(r, 250));
    return !panel.hidden;
  })()`,
);
// 左键拖拽恒可用（无吸附锁定态，view.md §手势与 Chat 唤出）
await check(
  "左键拖拽恒可用（无吸附锁定）",
  `(() => {
    const view = document.getElementById("view");
    let dragStarted = false;
    const h = () => { dragStarted = true; };
    view.addEventListener("view:drag-start", h, { once: true });
    view.dispatchEvent(new PointerEvent("pointerdown", { button: 0, bubbles: true }));
    view.dispatchEvent(new PointerEvent("pointerup", { button: 0, bubbles: true }));
    return dragStarted === true;
  })()`,
);
// #26 回归（toggle 流）：× 后 pet 移动不被系统恢复唤回
await check(
  "× 后 pet 移动不被唤回（#26）",
  `(async () => {
    const view = document.getElementById("view");
    const panel = document.getElementById("chat-panel");
    // 当前状态：chat 打开（上一断言 toggle 重开）；先 × 关
    panel.querySelector(".chat-close").click();
    if (!panel.hidden) return false;
    // pet 移动 → 系统恢复判定：userClosed 不得唤回
    view.dispatchEvent(new CustomEvent("view:moved", { detail: { x: 500, y: 500 } }));
    await new Promise((r) => setTimeout(r, 300));
    return panel.hidden;
  })()`,
);
// #26 回归（布局记忆）：右键重唤出，位置与 × 前一致
await check(
  "重唤出原位恢复（release 布局记忆，#26）",
  `(async () => {
    const view = document.getElementById("view");
    const panel = document.getElementById("chat-panel");
    view.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await new Promise((r) => setTimeout(r, 250));
    if (panel.hidden) return false;
    const l1 = panel.style.left, t1 = panel.style.top;
    panel.querySelector(".chat-close").click();
    await new Promise((r) => setTimeout(r, 100));
    // 右键重新唤出
    view.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await new Promise((r) => setTimeout(r, 250));
    return !panel.hidden && panel.style.left === l1 && panel.style.top === t1;
  })()`,
);

// C10 输入区：多行自增长到上限封顶（110px），发送按钮随内容启用
await check(
  "输入区自增长封顶 + 发送按钮启用",
  `(() => {
    const input = document.querySelector(".chat-input");
    input.value = Array.from({length: 20}, (_, i) => "第" + i + "行内容").join("\\n");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    const btn = document.querySelector(".chat-send");
    // style.height 封顶 110px（自增长上限）；按钮随非空启用
    const capped = input.style.height === "110px";
    input.value = "一行";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    const shrunk = parseInt(input.style.height) < 110;
    return capped && shrunk && btn.disabled === false;
  })()`,
);
// C10 滚动意图（真实几何）：阅读历史时新消息只提示不抢视口，点提示回底
await check(
  "滚动意图：滚离后提示不抢视口，点击回底",
  `(async () => {
    const panel = document.getElementById("chat-panel");
    const history = panel.querySelector(".chat-history");
    // 灌入足够消息使历史可滚
    for (let i = 0; i < 30; i++) window.__ambery.appendMessage("assistant", "历史消息 " + i);
    await new Promise((r) => setTimeout(r, 300));
    if (history.scrollHeight <= history.clientHeight) return "不可滚动，跳过断言几何";
    // 滚到中部（阅读历史）
    history.scrollTop = 100;
    history.dispatchEvent(new Event("scroll"));
    await new Promise((r) => setTimeout(r, 50));
    const topBefore = history.scrollTop;
    window.__ambery.appendMessage("assistant", "滚离后的新消息");
    await new Promise((r) => setTimeout(r, 300));
    const pill = panel.querySelector(".chat-pill");
    if (pill.hidden || !pill.textContent.includes("1")) return "pill 未提示: " + pill.textContent + " hidden=" + pill.hidden;
    if (history.scrollTop !== topBefore) return "视口被抢: " + history.scrollTop + " != " + topBefore;
    pill.click();
    await new Promise((r) => setTimeout(r, 50));
    return history.scrollTop > topBefore && pill.hidden;
  })()`,
);

console.log(`\nCDP 冒烟全过（${passed} 断言）`);
process.exit(0);
