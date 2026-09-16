# omenic PROGRESS（当前进度与规划）

> 已落地的背景见 [ROADMAP.md](ROADMAP.md)（已完成档）。本文只讲**还没做的**和**接下来做什么**——两份文档状态不同，ROADMAP 是已完成的沉淀，本文是未完成的规划。
>
> 编号体系沿用 ROADMAP：`C1–C8` / `R1–R7` / `G1–G6`。

## 当前位置（2026-09-16）

**G1–G6 全部已过，main 干净**（最新 `8e7fd6c`）。C1–C6 六个能力域的代码、测试、生产接线都在 main 里：

- G6（#373）把装配容器接进了 daemon→web 生产路径，C4 的 AGENTS.md 注入与压缩第一次对真实用户生效；#374 补了真链路 e2e（真实 daemon + 本地 OpenAI mock，字节级断言）。
- C7（tag）前置条件全满足，**等用户拍板时机**。
- C8 已裁定不做。

**G6 之前的 6 条开放缺口，现在只剩 2 条**（其余已被 G6 关闭，见下表）。

## 开放缺口（全部已 grep/cg 实测，2026-09-16）

| # | 缺口 | 现状证据 | 规模 | 归属 |
|---|---|---|---|---|
| ~~1~~ | ~~装配容器零消费~~ | ✅ **已关**（#373）：`Daemon::orbit_setup` 在 `server.rs:210/213/218` 真实 resolve 三个 harness 服务；`Fiber::resolve` 有了生产调用者；fiber 字段已去掉下划线 | — | — |
| ~~2~~ | ~~crash-repair 未接线~~ | ✅ **已关**（#373）：`server.rs:147` 的 `repair_interrupted_runs` 是 `interrupted_run_closers` 第一个生产调用者，`Daemon::start` 顺序为 open SessionDb → repair → bind | — | — |
| ~~3~~ | ~~orbit 接缝超限~~ | ✅ **已关**（#373）：压缩接缝 56→3 行（`compaction_bridge.rs`），远低于 R3 ≤20 限额 | — | — |
| 4 | **5.3/5.5 谱系分组** | 🟡 **仍开放**：web 侧零谱系代码（`grep lineage` 无结果）；`SessionSummary`（`session/src/lib.rs:152`）**无 parent 字段**——不是只缺 UI 分组，是缺数据模型。旧文「待按 `run.list` 组装」不准确，`run.list` 里没有父子关系 | 中（100–200 行 + schema 迁移） | **待规划** |
| ~~5~~ | ~~2.3 turn codec 零生产写入~~ | ✅ **已关**（#373）：G6 加了 `turn_log` 列 + `append_turn_log`/`load_turn_log`，`encode/decode_turn_log` 在 `session/src/lib.rs:819/845/888` 有了生产读写路径 | — | — |
| 6 | **单 worker 事件无会话归属** | 🟡 **仍开放**：并发 turn 事件会混写最近 `on_send` 的会话（`dispatch.rs` 事件推送无 per-run 路由）；#356 只让 web 侧 prompt 带 session_id/run_id；协议（`protocol.rs`）无 run_id 字段。暂靠「单运行」纪律兜底 | 中（协议加字段 + dispatch 路由） | **R2 协议层** |

## 下一个整合点：G7 谱系 + 并发归属

**触发条件已满足**：G6 已过，无前置阻塞。

两个开放缺口**恰好可以并发**——文件所有权互斥：

| 工作包 | 覆盖 | 独占文件 | 依赖 |
|---|---|---|---|
| **A 谱系分组（5.3/5.5）** | 缺口 4 | `crates/infra/session/src/lib.rs`（schema）、`crates/infra/daemon/src/state.rs`、`crates/web/{page-workspace,components}/`（sidebar 分组 UI） | 无 |
| **B per-run 事件归属** | 缺口 6 | `crates/infra/daemon/src/protocol.rs`（加字段）、`crates/infra/daemon/src/dispatch.rs`（路由）、`crates/web/client/`（消费 run_id） | 无 |

**可并发数：2**。两包文件零重叠（A 动 session schema + web 前端；B 动 daemon 协议 + dispatch + client 订阅端），合并不冲突。

### 工作包 A：谱系分组

dsh 的 `lineage.ts` 把会话按父子关系组成树（一个会话 fork 出子会话）。omenic 现在是平铺列表。要做的：

1. **数据模型先行**：`sessions` 表加 `parent_id TEXT NULL` 列 + 幂等迁移（照 G6 的 `apply_turn_log_column` 模式）；`SessionSummary` 加 `parent_id`；`session.create` 支持传 parent
2. **派生分组**：web 侧按 parent_id 组装成树；无 parent 的仍是顶层
3. **侧栏 UI**：`components/sidebar.rs` 的会话行按树缩进渲染（对齐 `.githooks/spec/sidebar.yaml` 契约锚点）

**注意**：不要从 `run.list` 派生父子关系——run 是会话内的执行记录，不是会话间的关系。

### 工作包 B：per-run 事件归属

dispatch 的事件推送当前不带 run_id，并发 turn 时事件会混到错误的会话。要做的：

1. `protocol.rs` 的事件帧加可选 `run_id`（**只能加字段，不改既有命令语义**——冻结契约）
2. `dispatch.rs` 推送时带上当前 prompt 的 run_id
3. `web/client` 的 `WireTranslator` 按 run_id 过滤，只投递当前活跃 run 的事件

**注意**：这是 R2 的协议改动权范围内的事；web 侧 `#356` 已经让 prompt 带了 session_id/run_id，B 只需让**事件反向**也带上归属。

## 后续 backlog（按价值÷成本排序）

| 优先 | 项 | 说明 |
|---|---|---|
| — | 插件 per-plugin Config schema + inject 依赖门控 | `DshPlugin` trait 现在只有 `name()` + `register()`（`registry.rs:14`）；要加关联类型/serde 结构 + 从主文档切插件切片 + 校验失败拒注册。两项改同一个 trait + registry，合并做省一半 |
| — | 插件反注册（单插件卸载） | `fiber.rs:82` 的 `unload()` 只能逆序拆全部；`EventBus::unsubscribe` 已有 |
| — | 插件发现/加载层（loader） | dsh 的 loader 绑 Node ESM hook，Rust 侧等价物是目录扫描+声明式注册表；**范围外**（C6 只对齐 cordis），暂不分配 |
| — | token-meter（C8.3） | 已裁定不做，stats 卡靠「无真数据则隐藏」兜底 |
| — | interaction 交互层（C8.1/8.2） | 已裁定不做，omenic 无 agent→用户提问通路 |

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
