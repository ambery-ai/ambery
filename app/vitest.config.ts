import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    // 前端进 case（docs/case-runner.md §壳类比）：case-runner 内嵌单例 core——
    // 测试文件共享同一份 config/storage，串行防跨文件污染（如 i18n 语言切换）
    fileParallelism: false,
    include: ["test/**/*.test.ts"],
    testTimeout: 30000,
    hookTimeout: 60000,
  },
});
