# omenic PROGRESS（当前进度与规划）

> 已落地的背景见 [ROADMAP.md](ROADMAP.md)（已完成档）。本文只讲**还没做的**和**接下来做什么**——两份文档状态不同，ROADMAP 是已完成的沉淀，本文是未完成的规划。
>
> 编号体系沿用 ROADMAP：`C1–C8` / `R1–R7` / `G1–G7`。

## 当前位置（2026-09-16）

**G1–G7 全部已过，main 干净。** G6 之前的 6 条开放缺口**全部关闭**（1/2/3/5 由 G6，4/6 由 G7）：

- G6（#373/#374）把装配容器接进 daemon→web 生产路径，C4 的 AGENTS.md 注入与压缩第一次对真实用户生效，并补了真链路 e2e。
- G7（#376）关掉最后两条：**谱系分组**（5.3/5.5，数据模型 + 侧栏树）与 **per-run 事件归属**（协议帧 run_id + 订阅端过滤）。
- C7（tag）前置条件全满足，**等用户拍板时机**。
- C8 已裁定不做。

**G7 之后没有排队中的整合点。** 下一步是 backlog 里的插件 schema 项，或等用户对 C7 拍板。

## 缺口表（全部已关闭，留作记账）

| # | 缺口 | 关闭证据 |
|---|---|---|
| 1 | 装配容器零消费 | ✅ G6（#373）：`Daemon::orbit_setup` 在 `server.rs:210/213/218` 真实 resolve 三个 harness 服务；`Fiber::resolve` 有了生产调用者 |
| 2 | crash-repair 未接线 | ✅ G6（#373）：`server.rs:147` 的 `repair_interrupted_runs` 是 `interrupted_run_closers` 第一个生产调用者 |
| 3 | orbit 接缝超限 | ✅ G6（#373）：压缩接缝 56→3 行，远低于 R3 ≤20 限额 |
| 4 | 5.3/5.5 谱系分组 | ✅ **G7（#376）**：`sessions.parent_id` 列 + 幂等迁移（`apply_parent_id_column`）；`SessionSummary`/`Session` 双层贯通；侧栏 `group_sessions` 树渲染（孤儿当根 / visited 防环 / 深度封顶）；`session.create` + `DaemonClient::session_create_with_parent` wire 通路 |
| 5 | 2.3 turn codec 零生产写入 | ✅ G6（#373）：`turn_log` 列 + `append_turn_log`/`load_turn_log` 有了生产读写路径 |
| 6 | 单 worker 事件无会话归属 | ✅ **G7（#376）**：`EventFrame.run_id`（serde-optional，旧订阅端无感）+ `WorkerHandle` sticky active-run 槽（prompt 前设、下一个归属 prompt 覆盖、reset 清）+ `RunFilteredSubscription` 按 run 过滤且不侵占调用方 tick 预算 |

## 后续 backlog（按价值÷成本排序）

| 优先 | 项 | 说明 |
|---|---|---|
| 1 | 插件 per-plugin Config schema + inject 依赖门控 | `DshPlugin` trait 现在只有 `name()` + `register()`（`registry.rs:14`）；要加关联类型/serde 结构 + 从主文档切插件切片 + 校验失败拒注册。两项改同一个 trait + registry，合并做省一半 |
| 2 | 插件反注册（单插件卸载） | `fiber.rs:82` 的 `unload()` 只能逆序拆全部；`EventBus::unsubscribe` 已有 |
| — | 插件发现/加载层（loader） | dsh 的 loader 绑 Node ESM hook，Rust 侧等价物是目录扫描+声明式注册表；**范围外**（C6 只对齐 cordis），暂不分配 |
| — | token-meter（C8.3） | 已裁定不做，stats 卡靠「无真数据则隐藏」兜底 |
| — | interaction 交互层（C8.1/8.2） | 已裁定不做，omenic 无 agent→用户提问通路 |
| — | C7（tag） | 前置全满足，等用户拍板时机 |

## G7 验收记录（#376）

- **CI**：`3f7f7a3` SUCCESS、`8841190` SUCCESS
- **审查**：CRG `detect-changes --brief --base e5d229c`（0 affected flows，untested 项均为跨模块名字匹配噪音，逐条在 PR comment 佐证）+ ocr 28 条逐条裁定 **0 真阳性**
- **smoke**：真 `daemon` 二进制（`cpulimit -l 65` 构建）+ 临时 socket + 手写 mock omp，10/10 断言通过——含**手工造的 pre-G7 旧库**（daemon 在其上启动并自动加 `parent_id` 列）、谱系 wire 往返、空白 parent 归一、归属 prompt 受理
- **测试**：`session/tests/lineage.rs`（5 例，含旧库迁移的 ALTER 分支）、`daemon/tests/run_routing.rs`（2 例，真 daemon + mock omp 的 run 盖章）、`components/tests/group_sessions.rs`（7 例，防环/孤儿/深链封顶）


## 工作约定

- **工作目录**：`.wt/<branch>`（git worktree add 必须在仓库根执行，防嵌套）
- **PR-only**：2026-09-15 裁定只开 PR 不开 issue；缺 `Fixes #` 只是 WARN 不阻塞
- **测试一律不在本地跑**：`cargo test` / `cargo clippy` / 全量 `cargo build` / `npm install` 全部交给 PR 的 CI（`.github/workflows/ci.yml`）。本地只允许 `cargo fmt --check`、单 crate `cargo check -p <crate>`（**不得**加 `--workspace` / `--all-targets`）、grep/ls/git/读写
- **唯一例外**：web UI 需要肉眼确认时的 `cpulimit -l 65 -i -- cargo build --bin oi-web`
- **提交前必跑** `cargo fmt --all`（commit checklist hook 拦不合格的 rust）
- **测试放同层 `tests/`**，不在 src/ 写 `#[cfg(test)]`；重型测试推 CI
- **改组件源码要同步** `.githooks/spec/` 下对应 UI 契约 yaml 的 `find` 锚点（新 spec 必须登记 `bin/web/tests/ui_contract.rs` 的 `SPEC_FILES`）
- **加依赖必须同步改 `Cargo.lock`**（CI `--locked` 拒绝重算）
- **`.githooks/` 只读**：gate 规则、hook 脚本、spec yaml 下的 gate 规则都不许改（UI 契约 yaml 由 `ui_contract.rs` 消费，不算 gate 规则，可改）。gate FAIL 了改自己的提交/正文去迎合，不要改规则
- **合并**：gate 拦截的 `gh pr merge --squash` + 删分支 + 清工作树；绝不本地 merge 到 main
- **领域依赖方向**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO
- **无 `Co-authored-by` trailer**
- **schema 迁移模式**：照 G6 的 `apply_turn_log_column`——`PRAGMA table_info` 检查 + 缺列才 `ALTER TABLE`，幂等，每次 open 都跑
