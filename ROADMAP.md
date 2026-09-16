# omenic ROADMAP

> omenic = 用 Rust 复刻 deepseek-harness（dsh，TS monorepo，本地 `~/projects/harness/deepseek-harness`）的 agent harness。
>
> **本文档是索引。** 内容拆成两份，按「已实现 = 背景 / 未实现 = 规划」分列：
>
> | 文档 | 内容 |
> |---|---|
> | [ROADMAP-delivered.md](ROADMAP-delivered.md) | **已交付**（背景）：C1–C8 能力域、R1–R7 路线、G1–G5 整合门验收记录、边界决定 |
> | [ROADMAP-active.md](ROADMAP-active.md) | **当前进度与规划**：开放缺口（带证据）、G6 下一个整合点、backlog 排序、工作约定 |
>
> **编号体系**（全仓稳定，源码注释与 UI 文本按编号引用，勿改）：`C1–C8` 能力域 / `R1–R7` 并发路线 / `G1–G6` 整合门。
>
> **当前位置（2026-09-16）**：G1–G5 全部已过，main 干净（`e45cee0`）。C7 tag 等用户拍板时机，C8 已裁定不做。下一步是 **G6 总装消费**——把装配出的插件容器接进 daemon→web 生产路径，详见 [ROADMAP-active.md](ROADMAP-active.md)。
>
> 历史锚点：起点 `53419ec` / `906ea2e` / `1b405f0`；设计蓝图 `todo/dsh/README.md`；冻结签名锚点 `todo/dsh/BACKGROUND.md`；三路交叉校验记录 `todo/roadmap-verify-2026-09-15.md`。
