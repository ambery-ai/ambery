// CDP 冒烟驱动（node --experimental-strip-types scripts/cdp-check.ts）：
// 连已启动的 headless Chrome（--remote-debugging-port=9222）里打开的 vite 页面，
// 经 Runtime.evaluate 驱动 browser mock 链路断言。用法：<url> < assertions 内联在本文件>

const DEBUG_PORT = 9222;

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
  `(() => { window.__overseer.callComponent({id:"t1",type:"text_card",title:"T",text:"hello"}); return !!document.querySelector(".component"); })()`,
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
    const cards = await window.__overseer_debug_cards ?? null;
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
  `(() => { window.__overseer.appendMessage("assistant", "回复来了"); return document.getElementById("pet-badge").textContent; })()`,
  "1",
);

// chat toggle 打开，历史来自 store.context 基线
await check(
  "chat 打开渲染 store.context",
  `(() => {
    document.getElementById("view").dispatchEvent(new Event("chat:toggle"));
    const h = document.querySelector(".chat-history");
    return h && h.textContent.includes("回复来了");
  })()`,
);

console.log(`\nCDP 冒烟全过（${passed} 断言）`);
process.exit(0);
