// vite 配置：dev server 固定端口 5174（与 tauri.conf.json devUrl 127.0.0.1:5174 对齐，
// strictPort 防端口漂移导致 tauri dev 等待错位端口）。

import { defineConfig } from "vite";

export default defineConfig({
  server: {
    port: 5174,
    strictPort: true,
  },
});
