# Spec — packages/terminal-lib

[English](spec.md) | 中文

## 技术选型

- Rust library crate（`ambery-terminal-lib`），无二进制、无自身运行时。
- 持有：adapter trait（`enumerate` / 三态 `read`）、信封类型（Source 投影、三态读 Content / Gone / Error）、Composite 分发（枚举路由）、平台原语 trait（终端宿主环境能力）、测试桩（MapAdapter）。

## 架构决定

1. **契约 crate 先行**：`terminals/*` 与 `agents/*` 只依赖本 crate；core 也只依赖本 crate。终端包与 agent 包互不依赖、也不依赖 core。
2. **trait 签名随 Ambery Protocol 走**：当前 trait 形态是暂定的；T35（接入协议）落地后按协议契约重新推导——此前本 crate 冻结任何 trait。

## 固定约束

- 三态读（Content / Gone / Error）是唯一读契约——无部分读、无臆测态。
- 打断性宿主动作（切桌面）必过显式同意；终端包绝不自行切换。
