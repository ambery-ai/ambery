// 跨平台运行 frontend case：Windows 用 .exe，macOS/Linux 用裸二进制。
// package.json test 脚本入口（替代 Windows 硬编码路径）。

import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "..", "..");
const exe = process.platform === "win32" ? "ambery-case.exe" : "ambery-case";
const bin = join(root, "target", "debug", exe);

execFileSync(bin, ["frontend", "--silent"], { stdio: "inherit" });
