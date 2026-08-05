// 设计 token 守卫（node scripts/lint-tokens.mjs）：
// 1. styles.css 的 :root token 表之外禁止裸色值（#hex / rgb()/rgba()），白名单仅 #fff；
// 2. 所有 var(--ov-*) 引用必须在 :root 表内有定义；
// 3. src 业务 TS（windows/components/view/autonomy/pet-size）禁止裸色值（debug 面板豁免）。
// 退出码非零 = 违规（列出全部行号）。

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const srcDir = join(dirname(fileURLToPath(import.meta.url)), "../src");
const css = readFileSync(join(srcDir, "styles.css"), "utf8");
const lines = css.split("\n");

let failures = 0;
const fail = (msg) => {
  console.error(`FAIL: ${msg}`);
  failures++;
};

// :root 块范围（styles.css 顶部唯一 token 表）
const rootStart = lines.findIndex((l) => l.trim().startsWith(":root"));
if (rootStart < 0) fail("找不到 :root token 表");
let rootEnd = rootStart;
for (let i = rootStart, depth = 0; i < lines.length; i++) {
  depth += (lines[i].match(/\{/g) ?? []).length - (lines[i].match(/\}/g) ?? []).length;
  if (depth === 0 && i > rootStart) {
    rootEnd = i;
    break;
  }
}

const COLOR_RE = /#[0-9a-fA-F]{3,8}\b|\brgba?\(/g;
/** 色值判别：hex 须含数字（排除 #face 等选择器/注释引用）；#fff 白名单单列 */
const isColor = (m) => (m.startsWith("#") ? /\d/.test(m) || m === "#fff" : true);
const VAR_RE = /var\((--ov-[a-z0-9-]+)\)/g;
const defined = new Set();
for (let i = rootStart; i <= rootEnd; i++) {
  for (const m of lines[i].matchAll(/(--ov-[a-z0-9-]+)\s*:/g)) defined.add(m[1]);
}

for (let i = 0; i < lines.length; i++) {
  if (i >= rootStart && i <= rootEnd) continue;
  const line = lines[i];
  for (const m of line.matchAll(COLOR_RE)) {
    if (m[0] === "#fff") continue; // 白名单：danger 底上的固定对比白
    if (!isColor(m[0])) continue;
    fail(`styles.css:${i + 1} 裸色值 ${m[0]} — ${line.trim().slice(0, 80)}`);
  }
  for (const m of line.matchAll(VAR_RE)) {
    if (!defined.has(m[1])) fail(`styles.css:${i + 1} 引用未定义 token ${m[1]}`);
  }
}

// TS 侧裸色值（递归收集，豁免 debug 面板与 scripts）
const tsFiles = [];
(function walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (p.endsWith(".ts")) tsFiles.push(p);
  }
})(srcDir);
for (const f of tsFiles) {
  if (f.includes("debug-vite-panel")) continue; // debug 专用，豁免
  const content = readFileSync(f, "utf8").split("\n");
  for (let i = 0; i < content.length; i++) {
    const t = content[i].trim();
    if (t.startsWith("//") || t.startsWith("*") || t.startsWith("/*")) continue; // 注释豁免
    for (const m of content[i].matchAll(COLOR_RE)) {
      if (!isColor(m[0])) continue;
      fail(`${f.split(/[\\/]src[\\/]/)[1]}:${i + 1} TS 裸色值 ${m[0]} — ${content[i].trim().slice(0, 80)}`);
    }
  }
}

if (failures) {
  console.error(`\n${failures} 处违规`);
  process.exit(1);
}
console.log(`token 守卫通过（${defined.size} 个 --ov-* token 定义在案）`);
