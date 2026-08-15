# ambery-case

Storage 快照驱动的回归测试与概念观测工具（docs/case-runner.md）。

- `.case` 两段式格式、step 语义、observe 输出、导出与 health：**docs/case-runner.md**
- 读取 / store / 表达式 / 变量 / parser / 类型系统 / checkhealth：**docs/case-eval-system.md**
- 可观测性机制（Observable trait + derive 覆盖断言）：**docs/observability.md**
- 前端 effect 上报：**docs/effect-reporting.md**

## 构建与运行

```bash
# 在仓库根（workspace）执行
cargo run -p ambery-case -- ambery-case/cases/closed-stale-cache.case          # 执行所有 steps
cargo run -p ambery-case -- ambery-case/cases/closed-stale-cache.case --health # case 合法性校验
cargo run -p ambery-case -- <case> --step-num 2                                  # 仅执行到第 N 步
cargo run -p ambery-case -- export --case-id <id> [--storage DIR] [--instances a,b] [--keep-agents] \
    [--keep-memory --memory name-a,AGENTS] [--keep-cron --cron-ids id-a,id-b] [--dry-run]
```

`ambery-core` 需 `case-runner` feature 编译（本 crate 已自动启用）。

## 注意

- 沙盒：`%TEMP%\ambery-case-<case_id>/` 开跑即重建；生产 storage/config 永不写。
- `cases/` 目录整体 gitignore（真实数据可能含敏感信息）。
- case 头部必须声明 `meta.llm_mode`（debug / real，无默认）；禁止携带 `llm.providers.*`。
