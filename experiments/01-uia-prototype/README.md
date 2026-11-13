# Experiment 01: UIA 原型

## 假设

> 用 UIA WaitForInputIdle 枚举所有 Windows Terminal 窗口和标签页，读取 TermControl 全量文字，检测 Claude Code 状态。

## 做法

1. PowerShell 脚本验证：枚举所有 `CASCADIA_HOSTING_WINDOW_CLASS` 窗口 → 遍历 TabItem → SelectionItemPattern.Select() 切换 → TextPattern.DocumentRange.GetText(-1) 读取
2. C# 控制台程序 `ambery.exe`：定时轮询 + 哈希对比 + 稳定检测

## 结果

| 验证项 | 结果 |
|---|---|
| 枚举所有 WT 窗口 | ✅ 发现 13 个窗口 |
| 读取标签页名称 | ✅ TabItem.Current.Name |
| 切换后台标签页 | ✅ SelectionItemPattern.Select() |
| 读取 TermControl 全文 | ✅ 4000-5000 字 |
| 检测 Claude Code 空闲 | ✅ 末行 `⏵⏵` |
| 检测 Claude Code 处理中 | ✅ 末行 braille spinner |

## 结论

UIA 方案完全可行，无需 ConPTY、剪贴板或 OCR。所有 WT 窗口/标签页可读，状态可检测。
