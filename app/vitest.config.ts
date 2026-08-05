import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    // 前端进 case v2（docs/case-runner.md）：headless JS + shim Tauri IPC + 真 core
    include: ["test/**/*.test.ts"],
    testTimeout: 30000,
    hookTimeout: 60000,
  },
});
