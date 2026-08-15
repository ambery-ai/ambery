// 双语文档配对校验（对齐 DSH i18n 纪律）：
// 1. 每个 foo.zh.md 必须有对应 foo.md（docs/、根 concepts/spec/docs-spec、README/CONTRIBUTING）；
// 2. 每个待翻译 foo.md 必须有 foo.zh.md；
// 3. 配对两侧不得完全相同（同内容 = 未翻译/复制占位）。
// 退出码非零 = 违规（列出全部问题）。

import { readFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { join, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const failures = [];
const fail = (msg) => failures.push(msg);

/** 递归收集指定目录下的 .md 文件路径（相对 root） */
function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      if (name === "node_modules" || name === ".git") continue;
      walk(p, out);
    } else if (name.endsWith(".md")) {
      out.push(p);
    }
  }
  return out;
}

// K-6 范围：docs/ 全部 + 根入口/规范文档。过程文档（reports/dev/experiments）不在配对范围。
const rootFiles = [
  "README.md", "README.zh.md",
  "CONTRIBUTING.md", "CONTRIBUTING.zh.md",
  "concepts.md", "concepts.zh.md",
  "spec.md", "spec.zh.md",
  "docs-spec.md", "docs-spec.zh.md",
];
const files = walk(join(root, "docs"))
  .map((p) => p.slice(root.length + 1))
  .concat(rootFiles.filter((f) => existsSync(join(root, f))));
const zh = new Set(files.filter((f) => f.endsWith(".zh.md")));
const en = new Set(files.filter((f) => f.endsWith(".md") && !f.endsWith(".zh.md")));

for (const f of zh) {
  const pair = f.slice(0, -".zh.md".length) + ".md";
  if (!en.has(pair)) fail(`中文 ${f} 缺英文配对 ${pair}`);
}
for (const f of en) {
  if (f.endsWith(".i18n.md")) continue;
  const pair = f.slice(0, -".md".length) + ".zh.md";
  if (!zh.has(pair)) fail(`英文 ${f} 缺中文配对 ${pair}`);
}
for (const f of zh) {
  const enPath = join(root, f.slice(0, -".zh.md".length) + ".md");
  const zhPath = join(root, f);
  const a = readFileSync(enPath, "utf8").replace(/\s+/g, " ").trim();
  const b = readFileSync(zhPath, "utf8").replace(/\s+/g, " ").trim();
  if (a === b) fail(`${f} 与英文配对内容完全相同（未翻译）`);
}

if (failures.length) {
  console.error(`FAIL: ${failures.length} 处 i18n 配对违规`);
  for (const f of failures) console.error(`- ${f}`);
  process.exit(1);
}
console.log(`i18n 配对通过（${zh.size} 对）`);
