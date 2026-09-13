// 实验页要在浏览器里直接 import `.ts`，所以需要一层转译——这就是本实验自己的构建配置。
// 不依赖任何外部工程：`npm install` 后 `npm run dev` 即可。
import { defineConfig } from "vite";

export default defineConfig({
  server: {
    port: 3011,
    strictPort: true,
    // 编辑器的原子写会在目录里落下 `.<name>.<pid>.<uuid>.tmpdir/` 暂存目录，
    // 监控扫到它会 EBUSY 崩掉；那是编辑过程的产物，不是源码。
    watch: { ignored: ["**/node_modules/**", "**/.*.tmpdir/**"] },
  },
});
