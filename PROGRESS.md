# omenic PROGRESS（当前进度与规划）

> 已落地的背景见 [ROADMAP.md](ROADMAP.md)（已完成档）。本文只讲**还没做的**和**接下来做什么**——两份文档状态不同，ROADMAP 是已完成的沉淀，本文是未完成的规划。
>
> 编号体系沿用 ROADMAP：`C1–C8` / `R1–R7` / `G1–G8`。

## 当前位置（2026-09-17）

**G1–G8 全部已过，main 干净（`a91caee`）。** G6 之前的 6 条开放缺口**全部关闭**（1/2/3/5 由 G6，4/6 由 G7）：

- G6（#373/#374）把装配容器接进 daemon→web 生产路径，C4 的 AGENTS.md 注入与压缩第一次对真实用户生效，并补了真链路 e2e。
- G7（#376）关掉最后两条：**谱系分组**（5.3/5.5，数据模型 + 侧栏树）与 **per-run 事件归属**（协议帧 run_id + 订阅端过滤）。
- G8（#377，2026-09-16 合并）修三处「单测绿、生产路径失效」的缺陷（见下表），让三态生命周期在真实 orbit run 下可观测。**验收**：gate merge --dry-run ALL PASS（119 checks）；PR CI 三连绿（`35130516365` 1m29s、`35130936505` 1m20s）；合并后 main CI `35134561055` SUCCESS（2m10s）；真二进制 smoke 7/7；终审 ocr 34 条裁定 7 真阳性全部已修。
- C7（tag）前置条件全满足，**等用户拍板时机**。
- C8 已裁定不做。

**G8 之后没有排队中的整合点。** 下一步是 dsh 全量对照 backlog（见文末，2026-09-16 archify 盘点），或等用户对 C7 拍板。

## 缺口表（全部已关闭，留作记账）

| # | 缺口 | 关闭证据 |
|---|---|---|
| 1 | 装配容器零消费 | ✅ G6（#373）：`Daemon::orbit_setup` 在 `server.rs:210/213/218` 真实 resolve 三个 harness 服务；`Fiber::resolve` 有了生产调用者 |
| 2 | crash-repair 未接线 | ✅ G6（#373）：`server.rs:147` 的 `repair_interrupted_runs` 是 `interrupted_run_closers` 第一个生产调用者 |
| 3 | orbit 接缝超限 | ✅ G6（#373）：压缩接缝 56→3 行，远低于 R3 ≤20 限额 |
| 4 | 5.3/5.5 谱系分组 | ✅ **G7（#376）**：`sessions.parent_id` 列 + 幂等迁移（`apply_parent_id_column`）；`SessionSummary`/`Session` 双层贯通；侧栏 `group_sessions` 树渲染（孤儿当根 / visited 防环 / 深度封顶）；`session.create` + `DaemonClient::session_create_with_parent` wire 通路 |
| 5 | 2.3 turn codec 零生产写入 | ✅ G6（#373）：`turn_log` 列 + `append_turn_log`/`load_turn_log` 有了生产读写路径 |
| 6 | 单 worker 事件无会话归属 | ✅ **G7（#376）**：`EventFrame.run_id`（serde-optional，旧订阅端无感）+ `WorkerHandle` sticky active-run 槽（prompt 前设、下一个归属 prompt 覆盖、reset 清）+ `RunFilteredSubscription` 按 run 过滤且不侵占调用方 tick 预算 |

## G8：会话生命周期正确性（#377，已合并）

三处「单测绿、生产失效」缺陷。共同特征是验收测试恰好绕开了生产路径的真实条件。

| 子任务 | 缺陷 | 修复 |
|---|---|---|
| **G8-A** run 收尾过早 | orbit 模式 `prompt()` 只投 channel 就返回，dispatch 紧接着 `runs.finish("ok")` + `TurnEnd{ok}` → `in_flight_runs` 恒 0，三态状态机失效，`interrupted_run_closers` 无半开记录可修（G6 的崩溃修复被掏空） | `AgentEnd` 携带 `stop_reason`（`#[serde(default)]` 向后兼容旧帧）→ pump 线程收到 `AgentEnd` 才 `finish` + `record_turn` + CAS 清 sticky 槽；orbit prompt 只回 ack 不同步收尾，omp 兼容模式保留同步收尾；pump 改在 orbit prompt 前启动（不再依赖客户端订阅） |
| **G8-B** 配置写回抹段 | `save_to_file` 用 `format!()` 整文件重写 6 个键，`[mcp]`/`[memory]`/`[daemon]` 静默消失；`config_roundtrip` 正好只覆盖被重写的键 | `toml_edit::DocumentMut` 增量写回，未管理段/注释/排版逐字节保留；解析失败时备份失败即中止（不吞错误）；`omp_path` 缺失才补，全量路径沿用原值 |
| **G8-C** spill 文件碰撞 | `truncate_output` 的 `counter` 参数 5 个调用点全传 0 → 文件名恒 `oi-output-0.txt`，第二次溢出覆盖第一次全文（另有一处 mcp 调用点子代理与 cg 都漏） | 删参数，文件名 `oi-output-{pid}-{seq}.txt`（pid 隔进程、seq 进程内单调） |

**验收记录**：

- **CI**：PR 三连绿（`35130516365` 1m29s、`35130936505` 1m20s、`35126051332` 1m35s）；合并后 main push `35134561055` SUCCESS（2m10s）。
- **gate**：`gate merge --dry-run` ALL PASS（119 checks）。
- **审查**：CRG `detect-changes --base 84ba2a4` 报 0 affected flow；ocr 终审 34 条逐条裁定，**7 真阳性全部已修**——其中 1 个 high 在 G8-B 自己的代码里（`preserve_omp_path` 行循环的 `?` 在首个注释行就返回 None，第二行的自定义路径被丢）；另有 `write_managed_keys` 的 `expect` panic、`runs.finish()` 三处静默丢错误、`close_run_on_agent_end` 检查-写入窗口可能追加第二个 TurnEnd（已让 `finish` 在 ledger 层幂等）。
- **smoke**：真 `daemon` 二进制（`cpulimit -l 60` 构建）orbit 模式 7/7——prompt 回 `{"started":true}` 立即返回、ack 后 run 为 **open**（修复前不可能出现的状态）、事件泵在 `AgentEnd` 后关闭它；LLM 端口故意指向死端口，turn 以 `TurnStop::Error` 结束**仍然正确关闭**，顺带覆盖「不订阅的客户端也能关闭 run」。
- **测试**：`daemon/tests/run_lifecycle.rs`（4 例，含 `finish` 幂等）、`rpc/tests/subscribe_pump.rs`（+54 行，`AgentEnd` 透传 `stop_reason` + 旧帧反序列化）、`web/client/tests/config_roundtrip.rs`（2 例回退路径：`omp_path` 不在首行、`[llm]` 非表不 panic）、`agent/tools/tests/truncate.rs`（3 例，溢出不碰撞）。
- **过程教训**：`truncate.rs` 一个测试扫整个 `/tmp` 读每个文件，CI 连挂 3 次每次 25 分钟超时；ocr 曾把这条标 high 被误判驳回。修后该二进制从超时降到秒级。

## 后续 backlog（按价值÷成本排序）

| 优先 | 项 | 说明 |
|---|---|---|
| 1 | 插件 per-plugin Config schema + inject 依赖门控 | `DshPlugin` trait 现在只有 `name()` + `register()`（`registry.rs:14`）；要加关联类型/serde 结构 + 从主文档切插件切片 + 校验失败拒注册。两项改同一个 trait + registry，合并做省一半 |
| 2 | 插件反注册（单插件卸载） | `fiber.rs:82` 的 `unload()` 只能逆序拆全部；`EventBus::unsubscribe` 已有 |
| — | 插件发现/加载层（loader） | dsh 的 loader 绑 Node ESM hook，Rust 侧等价物是目录扫描+声明式注册表；**范围外**（C6 只对齐 cordis），暂不分配 |
| — | token-meter（C8.3） | 已裁定不做，stats 卡靠「无真数据则隐藏」兜底 |
| — | interaction 交互层（C8.1/8.2） | 已裁定不做，omenic 无 agent→用户提问通路 |
| — | C7（tag） | 前置全满足，等用户拍板时机 |

## dsh 全量对照 backlog（2026-09-16 archify 盘点）

用 archify 把 dsh（55 顶层包 / 约 227 子包，commit `b150a55`）聚合成 12 个功能域，逐域标注 omenic 复刻状态。图：`dsh-architecture.architecture.html`（deliver SHA-256 `9b2bc572…`），清单：`dsh-functional-inventory.md`。

**完全缺失（omenic 零实现）**：

1. **元工具与治理整片** — goal / todo / plan-mode / schedule / skill / lsp / hooks / guard / feedback / workflow（约 60 子包）。复刻差距最大的功能域。
2. **LLM provider 注册表/路由 + token-meter** — 单 adaptor 硬编码，无第二 provider、无重试、无计量。
3. **jobs 后台作业 + terminal 持久 PTY + persistent shell** — 长命令只能被 30s 超时杀掉。
4. **会话恢复（resume）生产路径** — daemon 重启后 worker ctx 为空，模型只见最新一条消息（审计域2a 最高优先）。
5. **附件全链路** — ui-attachment 前端 + 后端存储 + 上下文注入三段都缺。
6. **host apiproxy + directory-picker** — 多 provider 路由与目录选择无对应物。
7. **session telemetry/otel + title-llm** — 可观测性与自动标题。
8. **core scope + agent-tool-presentation** — 作用域隔离与工具结果呈现策略。
9. **acp 协议 + boot/bundle 声明式装配** — omenic composition 的 dsh 对应物，但声明式层（profile/bundle）缺失。
10. **credentials authorization + identity** — 授权流与匿名身份（依赖 C8 已裁的交互通路，需重新裁定）。

**已复刻但生产路径有缺陷（G8 已修）**：run 收尾时机、配置抹段、spill 碰撞。

**已复刻的功能性偏差（未修）**：压缩配对方向与 dsh 相反（`cut += 1` 前缩 vs `keepFromIdx -= 1` 后扩，丢弃更多原文）；压缩阈值固定字符而非按窗口比例。

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
