// vite 配置：dev server 固定端口 5174（与 tauri.conf.json devUrl 127.0.0.1:5174 对齐，
// strictPort 防端口漂移导致 tauri dev 等待错位端口）。

import { defineConfig } from "vite";

export default defineConfig({
  server: {
    port: 3000,
    strictPort: true,
    // tauri 重编译会锁定 src-tauri/target 下的 Rust 产物；vite 不盯它，
    // 否则 watcher 撞 EBUSY 崩溃（Windows 文件锁）。
    watch: {
      ignored: ["**/src-tauri/target/**"],
    },
  },
});
