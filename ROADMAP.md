# omenic ROADMAP（项目背景）

> omenic = 用 Rust 复刻 deepseek-harness（dsh，TS monorepo，本地 `~/projects/harness/deepseek-harness`）的 agent harness。
>
> **本文档记录已落地的项目背景。** 接下来要开发什么、进度如何，见 [PROGRESS.md](PROGRESS.md)——两份文档状态不同，本文是**已完成**的沉淀，PROGRESS 是**未完成**的规划。
>
> 编号体系（全仓稳定，源码注释与 UI 文本按编号引用，勿改）：`C1–C8` 能力域 / `R1–R7` 并发路线 / `G1–G6` 整合门。

## 已交付的能力域 C1–C8

格式：小功能 → omenic 文件 → dsh 参考。✅ = 已合并 main。

### C1 agent 循环 ✅

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 1.1 循环 + 5 不变式（工具成对/截断不执行/中止不孤儿/压缩失败保原文/max_turns） | `crates/agent/orbit/src/lib.rs`（`run_agent_streaming`，23 集成测试） | `packages/core/agent-loop/src/{agent.ts,tool-calls.ts,invariant.ts}` |
| 1.2 流式 LLM（OpenAI 兼容 + SSE） | `crates/agent/adaptor/src/{openai.rs,sse.rs}` | `packages/llm/llm-pi-ai/src/` |
| 1.3 10 内置工具 + `Guarded` 策略包装 | `crates/agent/tools/src/`（10 文件 + `builtin_tools`） | `packages/core/tools/src/` + `packages/fs/` |
| 1.4 内嵌压缩（120k 字符策略，已被 C4 抽成插件） | `crates/agent/orbit/src/compaction_bridge.rs` | — |

### C2 会话持久化 + crash-repair ✅

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 2.1 会话 CRUD + libSQL 持久化 | `crates/infra/session/src/lib.rs`（`SessionDb`） | `packages/core/session/src/index.ts` |
| 2.2 事件词汇表（`WorkerEvent` 拆出 ToolExecutionStart/End/Error 专属变体，Unknown 只留未识别帧） | `crates/infra/rpc/src/worker.rs` | `packages/core/session/src/{known-event-types.ts,types.ts}` |
| 2.3 ✅ turn codec 已接生产（G6，#373）：`turn_log` 列 + 幂等迁移 + `append_turn_log`/`load_turn_log`；dispatch 在 TurnStart/TurnEnd 边界 best-effort 记录；`encode/decode_turn_log` 有了生产读写路径 | `crates/infra/session/src/lib.rs` | `packages/core/session/src/{chunk-rows.ts,json.ts}` |
| 2.4 ✅ repair 已接线（G6，#373）：`Daemon::start` 在 open SessionDb 之后、bind 之前调 `repair_interrupted_runs` → `interrupted_run_closers`，半开 run 落库标 `ABORTED`；幂等（`daemon/tests/repair.rs` 3 测试 + `g6_e2e.rs` 1 测试） | `crates/infra/daemon/src/server.rs` | `packages/core/session/src/{repair.ts,request-header.ts}` |

### C3 事件流可推 web ✅（#343）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 3.1 `AgentEvent` serde DTO（snake_case tag，跨 crate 契约） | `crates/agent/orbit/src/lib.rs`（`AgentEvent`） | `packages/core/session/src/types.ts` |
| 3.2 `WorkerEvents` 推形态（`subscribe → Receiver` + pump 线程，保留 `read_event` 轮询兜底） | `crates/infra/rpc/src/worker.rs` | `packages/client/runtime/src/client/sessions/{notifier.ts,service.ts}` |
| 3.3 daemon `event.subscribe` + `EventBus` 广播（UDS 多订阅者，断连自动清理） | `crates/infra/daemon/src/{protocol.rs,state.rs,dispatch.rs}` | `packages/sdk/server/src/server.ts` + `packages/api/gateway/src/{types.ts,client}` |
| 3.4 e2e：起 worker → run → 收完整 `TurnStart…TurnEnd` 序列 | `crates/infra/daemon/tests/event_push.rs` | `packages/core/session/src/invariant.ts` |

### C4 compaction 插件 + 指令注入 ✅（#346 + G6 消费 #373）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 4.1 `CompactionPolicy` trait + 默认实现（120k 策略平移） | `crates/harness/compaction/src/policy.rs` | `packages/compaction/compaction/src/{index.ts,checkpoint.ts}` |
| 4.2 tool_call/tool_result 成对裁剪不变式 | 同上（`pairing.rs`） | `packages/compaction/compaction/src/tool-pairing.ts` |
| 4.3 预算分区（system/工具/最近 N 条区） | 同上（`region.rs`） | `packages/compaction/compaction-basic/src/{region.ts,types.ts,config.ts}` |
| 4.4 LLM 摘要钩子（超预算区段 → 摘要替换原文） | 同上（`summarize.rs`） | `packages/compaction/compaction-basic/src/summarizer.ts` |
| 4.5 ✅ orbit 压缩接缝已瘦身（G6，#373）：`LlmSummarizer` 桥 + wire↔DTO 转换搬进 `compaction_bridge.rs`，接缝从 56 行降到 **3 行代码**（`mod` + `pub use`），远低于 R3 ≤20 限额 | `crates/agent/orbit/src/compaction_bridge.rs` | — |
| 4.6 AGENTS.md 向上查找 + 状态缓存 | `crates/harness/instruction/src/files.rs` | `packages/context/agent-instructions/src/{files.ts,state.ts}` |
| 4.7 指令 digest 去重 + 渲染 `PromptTemplate` | 同上（`render.rs`） | `packages/context/agent-instructions/src/{render.ts,digest.ts,config.ts}` |
| 4.8 orbit system prompt 注入（#354）：`LoopConfig.instruction_cwd` 旋钮 → 查找+去重+前置到 TASK 之前 | `crates/agent/orbit/src/lib.rs`（`build_system_prompt`）+ `tests/instruction_prompt.rs` | `packages/context/agent-instructions/src/render.ts` |

> **G6 的意义**：C4 的压缩与 AGENTS.md 注入在 G5 之前只有测试口径——daemon worker 硬编码 `LoopConfig::default()`，真实 web 聊天既不读 AGENTS.md 也不压缩。G6（#373）让 daemon 从装配容器取 cwd/compaction/max_turns，这两项能力第一次在生产路径生效，`g6_e2e.rs`（#374）在真实 HTTP 请求体字节级证实。

### C5 web 页面跑真数据 ✅

读侧真数据 + 事件订阅端 + orbit 真运行全通（用户浏览器实测确认对话成功）。mock crate 已整体删除（#366）。

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 5.1 `AgentEvent` DTO + 转译层（纯函数可单测，15 测试） | `crates/web/state/`（含 `memory_link`） | `packages/core/session/src/surface.ts` |
| 5.2a daemon RPC 读客户端（封装 `session.*`/`run.list` + DTO 转换，#341） | `crates/web/client/`（`daemon.rs`；`llm.rs` 保留） | `packages/client/runtime/src/client/sessions/{manager.ts,remotes.ts}` |
| 5.2b 订阅接收端（`WireTranslator` 双形状兼容 + 工具同名 LIFO 配对；断线退避重连 1s→5s；error 帧→TurnEnd 兜底） | `crates/web/client/`、`page-workspace/` | `packages/client/runtime/src/client/sessions/{notifier.ts,service.ts}` |
| 5.3 真数据已接 + 状态三态（#356，`run.list` 推断 Idle/Active/Aborted）；谱系分组未做（见 PROGRESS） | `crates/web/page-workspace/` | `packages/client/runtime/src/client/sessions/{session.ts,lineage.ts}` |
| 5.4 聊天流式：delta 追加 + tool 折叠卡（由订阅管线承载） | `crates/web/components/`（chat.rs） | `client/conversation/{event-registry.ts,view-registry.ts}` + `sessions/tool-call-tree.ts` |
| 5.5 sidebar 真会话已接；谱系分组未做（见 PROGRESS） | `crates/web/components/`（sidebar.rs） | `packages/client/runtime/src/client/sessions/lineage.ts` |
| 5.6 statusline 计时（#365）：`run_started_at_ms`/`elapsed_ms` + 起表/结算，13 个计时测试 | `crates/web/state/tests/statusline_timing.rs`（状态行内联在 `components/chat.rs`） | `packages/client/runtime/src/client/sessions/assistant-timing.ts` |
| 5.7 stats 走 `stats.summary` 真实 ledger（#364）；token 用量卡**无真数据则隐藏**（完整需 C8.3，已裁定不做） | `crates/web/page-stats/` | `packages/llm/token-meter/src/{usage-projection.ts,projection.ts}` |
| 5.8 配置页 TOML 往返（#364 测试：`client/tests/config_roundtrip.rs`）；`load_from_system`/`save_to_file`/`test_connection` 已接线 | `crates/web/page-config/` | `packages/settings/settings-file/src/index.ts` |
| 5.9 mock 真删（#366）：`crates/web/mock/` 从成员表、`Cargo.lock`、目录三处删除 | — （crate 已不存在） | — |
| 5.10 ui-validate 契约层（#344）：`.githooks/spec/` 下 7 份 UI 契约 yaml / 71 锚点 + 3 契约测试 | `bin/web/tests/` + `.githooks/spec/` | — |

### C6 插件面四件套 ✅（#339）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 6.1 服务注册容器（`ServiceRegistry`，静态 `HashMap<TypeId, Arc<dyn Any>>`） | `crates/harness/plugin/src/context.rs` | `vendor/cordis/src/{context.ts,service.ts}` |
| 6.2 事件总线（同步顺序派发，插件卸载自动注销） | 同上（`events.rs`） | `vendor/cordis/src/events.ts` |
| 6.3 插件生命周期（on_load/on_unload，Drop 逆序） | 同上（`fiber.rs`） | `vendor/cordis/src/fiber.ts` |
| 6.4 插件注册表 + 重名拒绝 | 同上（`registry.rs`） | `vendor/cordis/src/registry.ts` |
| 6.5 装配根（#363）：`assemble()` 由 `Daemon::start` 在 bind 之前调用，注册 Compaction / Instruction 两个核心插件；`tests/assemble.rs` 5 测试 | `crates/composition/src/lib.rs` + `crates/infra/daemon/src/server.rs` | `packages/bundle/base/src/index.ts` |
| 6.6 orbit `run_agent` 服务化接线（daemon orbit worker 模式，#349/#350） | `crates/infra/rpc/src/worker.rs`（`OrbitEngine`） | `packages/core/agent-loop/src/index.ts` |

> **G6 装配消费（#373）**：`Fiber::resolve` 有了生产调用者——`Daemon::orbit_setup` 从容器解析 `harness.tools` / `harness.compaction` / `harness.loop`（缺失回退各自家族默认）；`assemble_plugins` 把 cwd/max_turns 写进文档；fiber 字段不再带下划线，`Fiber::resolve`/`Fiber::config` 是它的读路径。

### C7 tag + ferrite 接线 ⏸️ 暂缓（2026-09-15 裁定）

前置条件已满足（C1–C6 全绿 + main CI 绿 + G6 装配消费已过）。`tag omenic-harness-v0.1.0` 与 ferrite 接线的**时机由用户拍板**，不分配工作。

### C8 interaction + token-meter + llm-retry ⏸️ 不做

**2026-09-15 裁定：C8 不在 omenic 范围**，R5 路线保持占位。5.7 的 token 数据源需求由「无真数据则隐藏该卡」兜底。omenic 目前**没有** agent→用户提问通路（无 `crates/harness/interaction/`），这是范围裁定不是遗漏。

## 已完成的并发路线 R1–R7

| 路线 | 覆盖 | 独占文件 | 完成 |
|---|---|---|---|
| **R1 插件面** ✅ | C6（6.1–6.6） | `crates/harness/plugin/`、`crates/composition/`；orbit ≤30 行 | #339 |
| **R2 事件流+修复** ✅ | C2（2.2–2.4）+ C3（3.1–3.4） | `crates/infra/{daemon,session,rpc}/`；orbit 只读 | #343 |
| **R3 核心插件** ✅ | C4（4.1–4.8） | `crates/harness/{compaction,instruction}/` | #346（验收②③ 由 #354/#355 补齐） |
| **R4 web** ✅ | C5（5.1–5.10） | `crates/web/{client,state,components,page-workspace,page-stats,page-config}/`；壳在 `bin/web/` | #340–#352 + #356/#357 |
| **R5（占位）** ⏸️ | C8 | `crates/harness/{interaction,metering}/`、`adaptor/retry.rs` | 裁定不做 |
| **R6 总装** ✅ | 6.5 真装配 + 5.9 删 mock | `crates/composition/`、page-stats/page-workspace 换数据源、删 `crates/web/mock/` | #363–#366 |
| **R7 总装消费** ✅ | 装配容器消费 + crash-repair 接线 + orbit 接缝瘦身 | `worker.rs`、`composition/src/lib.rs`、`server.rs`、orbit 接缝区 | **#373**（G6） |

## 已通过的整合门 G1–G6

| 门 | 验证内容（当时实际跑过的） |
|---|---|
| **G1 契约冻结** ✅ | 6.1 插件面 trait + 3.1 `AgentEvent` serde 冻结；`cargo test -p omenic-harness-plugin` 4 测试绿 + serde 往返 3 测试绿 |
| **G2 C6 收编** ✅ | orbit 23 测试全量回归不改断言 + `oi task add` → 流式 run → 事件流 → 持久化全链路通 |
| **G3 插件回归闸** ✅ | ① orbit 23 测试绿；② >120k 字符长会话压缩不炸（`compaction_e2e.rs` 4 测试）；③ AGENTS.md 注入（`instruction_prompt.rs` 4 测试）；④ web 页真实 run 压缩不中断流式（用户浏览器实测 2026-09-15） |
| **G4 事件流汇合** ✅ | ① 3.4 e2e 绿（`event_push.rs`）；② web 聊天页流式 delta 逐字追加（**用户浏览器实测 2026-09-15**）；③ 断线重连不白屏（`reconnect.rs` 3 测试）；④ 半开 run 标 aborted（`run_status.rs` 8 测试）；⑤ web 全仓零 `read_event` 调用 |
| **G5 总装** ✅ | ① C1–C6 全绿（C6.5 由 #363 接通）；② main CI `cargo test --locked --all-targets` 在 `e6039d6` 上 SUCCESS；③ `oi-web` 起在 8026，页面内联 40KB 真实 Tailwind，`grep -ci mock` = 0 |
| **G6 总装消费** ✅（#373/#374，2026-09-16） | ① daemon worker 从装配容器取 cwd/compaction/max_turns，AGENTS.md 注入首次在生产路径生效；② crash-repair 接线，`Daemon::start` 修复半开 run；③ orbit 压缩接缝 56→3 行；④ 真链路 e2e（`g6_e2e.rs` 3 测试）：起真实 daemon + 本地 OpenAI mock server，断言**真实 HTTP 请求体字节**含 AGENTS.md 标记、max_turns 卡住真实多轮 run、孤儿 run 重启修复 |

## 边界决定（稳定，勿翻案）

**明确不做**（dsh 有但 omenic 不复刻）：`hooks`（外部 agent hook 协议）、`skill`（SKILL.md 加载）、`guard`（timeout-policy/repeat-reminder）、`lsp`、`sandbox`、`subprocess`、`e2b`、react `ui-*`（40+ 包）、以及 dsh 的 Node ESM loader 层（`vendor/loader`，C6 只对齐 `vendor/cordis`）。

**独有功能保护区**（omenic 独有，dsh 无对应物——任何路线**只读/单向依赖**，不许重构、不许当缺口往里塞）：
- `crates/infra/memory`（jcode：`embed/graph/recall/inject/pipeline`）
- `crates/agent/task`（`runner/graph/store/template` RPC 任务模型）
- `crates/agent/subagent`、`crates/agent/mcp`
- `crates/evidence/spec` + `bin/gate`（合规工具；远期归宿 = C6 插件面落地后注册成工具插件，现在不动）

**冻结契约**（改动权归首发路线，他路线按冻结类型消费）：`AgentEvent` serde（3.1）、daemon protocol（3.3）、harness trait（6.1）。改 `daemon/protocol.rs` **只能加命令**，不许改既有 `session.*` / `run.list` 语义（task/memory 的 RPC 依赖这些）。

**领域依赖方向**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO。

## 历史锚点

起点 `53419ec` / `906ea2e` / `1b405f0`；设计蓝图 `todo/dsh/README.md`；冻结签名锚点 `todo/dsh/BACKGROUND.md`；三路交叉校验记录 `todo/roadmap-verify-2026-09-15.md`。
