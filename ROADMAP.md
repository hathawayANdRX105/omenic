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
| 5.3 真数据已接 + 状态三态（#356，`run.list` 推断 Idle/Active/Aborted）；谱系分组（#376，G7）：`sessions.parent_id` 列 + 幂等迁移，侧栏按树缩进 | `crates/web/page-workspace/` | `packages/client/runtime/src/client/sessions/{session.ts,lineage.ts}` |
| 5.4 聊天流式：delta 追加 + tool 折叠卡（由订阅管线承载） | `crates/web/components/`（chat.rs） | `client/conversation/{event-registry.ts,view-registry.ts}` + `sessions/tool-call-tree.ts` |
| 5.5 sidebar 真会话已接；谱系分组（#376，G7）：`group_sessions` 扁平转树（孤儿当根 / visited 防环 / 深度封顶）+ 行内新建子会话钮 | `crates/web/components/`（sidebar.rs） | `packages/client/runtime/src/client/sessions/lineage.ts` |
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
| 6.7 per-plugin Config schema + inject 依赖门控（#379） | `crates/harness/plugin/src/{registry.rs,context.rs,fiber.rs}` | `vendor/cordis/src/{registry.ts,fiber.ts}`（`resolveConfig` / `Fiber.start` inject check） |
| 6.8 单插件卸载（#379） | `crates/harness/plugin/src/{registry.rs,fiber.rs}` + `tests/plugin_test.rs` | `vendor/cordis/src/{registry.ts,fiber.ts}`（`RegistryService.delete` / `Fiber.dispose`） |

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
| **G7 谱系 + 并发归属** ✅（#376，2026-09-16） | ① `sessions.parent_id` 列 + 幂等迁移（`apply_parent_id_column`），`SessionSummary`/`Session` 双层贯通；② 侧栏 `group_sessions` 树渲染（孤儿当根 / visited 防环 / 深度封顶不丢节点）+ 行内新建子会话钮；③ `EventFrame.run_id`（serde-optional）+ sticky active-run 槽 + `RunFilteredSubscription` 按 run 过滤；④ 真二进制 smoke 10/10（含手工造 pre-G7 旧库的升级路径）；⑤ 逻辑层测试 14 例（lineage 5 + run_routing 2 + group_sessions 7） |
| **G8 会话生命周期正确性** ✅（#377/#378，2026-09-16/17） | ① 三处「单测绿、生产失效」缺陷：orbit run 在 prompt ack 时就被关闭（`in_flight_runs` 恒 0、三态状态机失效）→ 改由事件泵在 `AgentEnd` 收尾，泵在 prompt 前启动（不订阅也能关闭）；`save_to_file` 整文件重写抹掉 `[mcp]`/`[memory]`/`[daemon]` → `toml_edit` 增量写；spill 文件名恒 `oi-output-0.txt` 互相覆盖 → `oi-output-{pid}-{seq}.txt`；② #377 gate merge --dry-run ALL PASS（119 checks）；③ PR CI 三连绿 + 合并后 main CI `35134561055` SUCCESS；④ 真二进制 smoke 7/7（ack 后 run open、泵在失败的 turn 上仍正确关闭）；⑤ 终审 ocr 34 条裁定 7 真阳性全部已修（含 G8-B 自己代码里的 1 个 high）；⑥ 补验 PR #378（2026-09-17）以 `a91caee` 为 base 重走主控规范流程：codegraph 覆盖三条修复链、3 子代理 audit、CRG 0 affected flows、ocr 20 条全 pre-existing、23 个 G8 单测绿、真 daemon+oi smoke 通过、gate 105 checks ALL PASS |

## 已交付的 dsh 对照批次 B1–B3

2026-09-18 把 dsh 对照 backlog 划成三批，2026-09-19 全部合入 main。验收口径一致：CI test job 全绿 + CRG `detect-changes` 0 affected flow + ocr 逐条裁定 + 真 daemon 二进制 smoke。

| 批次 | PR | squash | 交付 | 余量 |
|---|---|---|---|---|
| **B1** MCP 多传输 + 重连 | #381 | — | stdio + Streamable HTTP 双传输、重连监督（指数退避 + 重试上限熔断 unregister）、per-server timeout/cwd、`fail_on_startup_error`、daemon `orbit_setup` 注入；page-config 表单 + `settings.yaml` 契约锚点 | tool-level filter、server-level env 注入 |
| **B2a** session resume | #382 | — | daemon 重启后 worker 按 `session_id` 回放最近 50 条 user/assistant 历史进 `ctx.messages`（dedupe + 切换清旧 ctx） | checkpoint flush 策略；回放跳过 System/Tool 行 |
| **B2b** LLM 路由 | #382 | — | `orbit::WaterfallLlm`（`[[llm.fallbacks]]` 按序切换 + per-provider `RetryPolicy` 退避 + 已泄 delta/toolcall 不切 provider）+ web Fallback 表单 | token-meter 不做（C8）；DeepSeek/PiAi 官方 adapter 未接 |
| **B3-PR1** jobs + terminal | #385 | `bc24a7e` | `crates/harness/jobs`（`JobRegistry` + `LocalJobRegistry`，`std::thread` + Condvar）+ `crates/harness/terminal`（portable-pty，每会话 reader 线程排空 master，`read` 是 drain 语义）；10 把模型工具经 `OrbitConfig.session_tools` 接入；web 词表 + `chat.yaml` 锚点 | `onJobDone` 回调、pwsh 后端、dsh jobs 其余工具 |
| **B3-PR2** subagent Phase 4 | #387 | `a15171f` | ACP 协议层（JSON-RPC over NDJSON）+ `AcpProvider` 出进程后端（两阶梯 dispose：EOF→SIGKILL，幂等 + exit watcher）+ `RunDisposer` 外化销毁 + runtime run 表 + `subagent_control` 的 `interrupt` + `[[subagent.providers]]` 配置；`OrbitSetup.providers` seam **clean cutover 删除**（装配上移 daemon） | `send_message`/`report`、continuable/background、session-seeding、SIGTERM 中间层、permission option kind、Codex/Claude Code/SDK 三后端 |
| **补审** B1/B2 合并后审查收尾 | #383 | `b20ebc0` | 补齐 ocr 层（36 文件 37 条）+ code-reviewer 逐文件复审 `c009f20..334e5ab` + ROADMAP 同步 | — |

> **B1/B2 的审查时序**：合并时只走了 CRG + code-reviewer，ocr 层缺失；由 #383 在合并后补齐。B3 起严格执行合并前 CRG + ocr 双层。

## 已交付：使用体验 P0 批次（2026-09-20）

判据从「dsh 有什么」翻转为「打开 web 用时哪里卡」后的第一批。5 个 PR 全部 squash 合入，CI 全绿 + CRG + 双 reviewer 两阶段审查（先 spec 合规后代码质量）+ 实证验证。

| PR | squash | 交付 | 遗留 |
|---|---|---|---|
| **#392** session-title | `964973b` | `title_from_first_message` 确定性截词（40 字符预算、markdown 前缀剥刺、emoji 按 chars() 安全、空串占位）；page-workspace `on_send` 首条消息触发标题替换；5 测试 | 侧栏按钮新建会话刷新后标题回退（缺 update RPC，见 F2） |
| **#393** todo+goal 模型 | `3919d29` | `todo.rs`（终态封闭状态机：Done→Cancelled 拒绝、Cancelled 封闭、Done→Open 重开）+ `goal.rs`（单向 link、幂等）；`store.rs` 泛化成三文件共用读路径（`load_records<T>`/`append_line`/`read_locked`/`corrupt_or_trim`，行为保持）；12 测试 | 损坏行/trim 路径零测试覆盖（见 F1） |
| **#394** daemon task.list | `bbe90f0` | `Command::TaskList`（limit 缺省 50 / 0→[] / 脏数据回退不报错）+ `DaemonConfig.data_dir` 全链透传 + `WebDaemon::task_list`；3 个 e2e（排序/limit/空态） | — |
| **#395** web 看板接线 | `92c04ef` | `TaskItem::from_task`（4 态词汇表映射、kind 穷尽 snake_case、priority 直通）+ WP-C effect 第二次 RPC + staleness 守卫覆盖双 signal；3 映射测试 | smoke 只到 oi-web 起服层（无 daemon 二进制构建授权） |
| **#396** 文档同步 | `b262147` | PROGRESS.md | — |

### P0 批次合并后审查结论（PASS_WITH_NITS，无 Critical）

两条 reviewer 报的 Critical 经实证**驳回**：① `trim_start_matches` 被误判为「单次匹配」——rustc 实证 `"> - 标题"` → `"标题"`，复合前缀剥刺正确（残留：该用例无测试钉住）；② 「Goal 状态机缺终态约束」为虚构需求——任务书对 Goal 只要求 `new`/`link`/`unlink`，终态封闭是 Todo 的契约（已实现），Goal.status 为 pub 字段与执行模型 `Task.status` 同模式。

**真实遗留（转 PROGRESS 的 F1–F3）**：① `store.rs` 数据完整性路径（损坏行 trim / CorruptLine）零测试覆盖；② 侧栏新建会话标题刷新回退（用户裁定加 `session.update_title` 增量 RPC——属「protocol 只能加命令」允许的增量）；③ todo/goal 写方（agent 工具）未接。

### 审查实际拦下的真实缺陷（三批合计）

CI 与 ocr/code-reviewer 在合并前拦下的，不是测试瑕疵：

1. **terminal kill 不彻底（#385，`64a6bad`）**：pty 只要还有进程持有 slave 就不关闭，bash 死时不杀子进程 → 被孤立的 `sleep` 攥着 slave，reader 永久停在 `read(master)`，会话"活着"直到孤儿到期。杀 shell 进程组也不够（job control 给每个作业单独 pgrp），真正共享的是**会话 id**（`portable_pty` 在 exec 前调了 `setsid`）。实测 SIGTERM 根本不杀 pty 上的 bash、SIGHUP 杀 shell 但 pty 永不 EOF，只有 SIGKILL 既 reap 又让 pty 在 ~0.4ms 内 EOF。
2. **持锁 notify 丢失唤醒（#385）**：原先 drop guard 后再 notify，而 `read` 进入 `wait_timeout` 时已释放互斥锁，落在窗口内的通知无人接收 → reader 空等满整个 timeout。改为持锁 notify。同批还把 `.lock().unwrap()` 全换成 `lock_recover`（对齐 mcp/daemon 的 poison 容忍惯例）。
3. **pty 回显不可靠（#385）**：`write("echo hi")` 后读端有两份 `hi`（回显 + 真输出），"数出现次数"会因回显跨 read 到达而误判、调用方在命令跑完前就返回。正解是让 shell 拼哨兵字面量（`__DONE_<token>__`），已写进 terminal crate 文档与测试 helper。
4. **`TerminalRegistry::list()` 自死锁（#385）**：持 `sessions` 锁再调 `status()` → `get()` 重入同一把 `std::sync::Mutex`（本地跑测试挂起 >60s 才发现）；抽 `summarize(&TerminalId, &Session)` 直取 `Arc` 修掉。
5. **ACP dispose 双重 reap（#387，`5215bd8`）**：Unix 上 `try_wait` 已 reap，尾部 `child.wait()` 撞 ECHILD；reap 成功时 take handle，尾部只 wait 它真正持有的。
6. **dispose / worker 启动赛跑（#387，`cc5c78a` + `e8ecec2` + `e3bd903`）**：worker 还没 initialize 完 disposer 就拆 stdin → broken pipe。三重门：worker 启动门与 disposer 读同一 abort flag、门拆之后传输失败报 Aborted 而非 Completed。`AcpDisposer` 在 `wait_for_exit` 里重入自己的 Mutex guard 导致全测试死锁，是同一处的第二波（poll 先算 `exited`、guard 出作用域再 take）。
7. **serde enum 字段命名（#387，`242f45b` + `e8ecec2`）**：enum 级 `rename_all` 只重命名变体名、不覆盖 struct-variant 字段 → `PermissionOutcome::Allow` 的 `option_id` 以 snake_case 上线；`InitializeRequest/Response`、`PromptResponse` 同缺变体级 `rename_all`。本地 `cargo check` 抓不到运行时 serde 错误，靠 CI 暴露。
8. **`write_full_config` 抹段与 fallback 收尾（#383，`6661a70` + `ae618a0`）**：整文件重写丢 `[[mcp.servers]]` 导致首次保存丢整张表；fallback 成功后仍以 `TurnEnd{Error}` 结束且 `tool_calls` 被丢弃，改用终态 + `leaked_content` 判据。
9. **daemon seam e2e 的 mock 截断请求体（#385，`1197399`）**：从只含 header 的 buffer 算 body 偏移 → 塌成 0，9401 字节请求只记录 8538，模型从未拿到完整工具表 → 无 `tool_calls` → 循环直接 `agent_end`。既有测试缺陷，非新代码引入。
10. **config cwd 竞态（#385，`6e722f0`）**：`Config::load` 与 `set_current_dir` 都是进程级，三用例在 cargo 并行线程下互抢；加 `cwd_lock()` 串行化（poison 容忍）。

## 边界决定（稳定，勿翻案）

**明确不做**（dsh 有但 omenic 不复刻）：`hooks`（外部 agent hook 协议）、`skill`（SKILL.md 加载）、`guard`（timeout-policy/repeat-reminder）、`lsp`、`sandbox`、`subprocess`、`e2b`、react `ui-*`（40+ 包）、以及 dsh 的 Node ESM loader 层（`vendor/loader`，C6 只对齐 `vendor/cordis`）。

**独有功能保护区**（omenic 独有，dsh 无对应物——任何路线**只读/单向依赖**，不许重构、不许当缺口往里塞）：
- `crates/infra/memory`（jcode：`embed/graph/recall/inject/pipeline`）
- `crates/agent/task`（`runner/graph/store/template` RPC 任务模型；dsh 的 `workflow` 是模型在运行时自己写编排，方向相反，不算对应物）
- `crates/evidence/spec` + `bin/gate`（合规工具；远期归宿 = C6 插件面落地后注册成工具插件，现在不动）

> **2026-09-16 更正**：此前本表把 `crates/agent/subagent` 与 `crates/agent/mcp` 也列为「dsh 无对应物」，经核对 dsh 源码**不成立**——dsh 有 `packages/subagent/`（11 子包：spawn/fork 进程内后端 + ACP/Codex/Claude Code/SDK 四个进程外后端 + control/report 工具）与 `packages/mcp/mcp-client/`（多传输客户端）。omenic 的 subagent 相当于 `subagent-spawn-in-process` 的只读工具简化版；**Phase 1 已接 daemon 生产路径（#380）**，**Phase 4 的 ACP 出进程后端 + interrupt 亦已落地（#387，2026-09-19）**；mcp 客户端自 #381（2026-09-18）起已对齐 dsh 的 stdio + Streamable HTTP 双传输形态。两者移出保护区，按普通缺口排优先级。

**冻结契约**（改动权归首发路线，他路线按冻结类型消费）：`AgentEvent` serde（3.1）、daemon protocol（3.3）、harness trait（6.1）。改 `daemon/protocol.rs` **只能加命令**，不许改既有 `session.*` / `run.list` 语义（task/memory 的 RPC 依赖这些）。

**领域依赖方向**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO。

## dsh 全量对照结论（2026-09-17 子代理调查）

> 2026-09-16 archify 盘点的 10 条「完全缺失」已由 3 个 Explore 子代理逐域核对 dsh 源码确认。以下按「是否影响 daemon/web 生产路径」分级。

### 已确认缺失且需接线进 daemon/orbit

| 域 | dsh 现状（参考文件） | omenic 现状 | 接线优先级 |
|---|---|---|---|
| **subagent 能力 seam** | `SubagentRuntime` 服务（provider registry + one-shot/continuable + 生命周期事件）+ 11 子包：spawn/fork 进程内 + ACP/Codex/Claude Code/SDK 四个进程外后端 + control/report 工具（`subagent/src/index.ts`） | **Phase 1 + Phase 4 起步已落地（#380 + #387，2026-09-19）**：crate `omenic-harness-subagent`（`SubagentProvider`/`SubagentRuntimeService`/`ForkProvider`）；model-facing `subagent`（输出带 `run_id`）+ `subagent_control`（list + interrupt）；daemon `orbit_setup` 注册 fork 与全部配置的 ACP provider；ACP 协议层（JSON-RPC over NDJSON）+ `AcpProvider`（两阶梯 dispose：EOF→grace→SIGKILL，幂等 + exit watcher）+ `RunDisposer` 外化销毁 + runtime run 表（`start_run`/`interrupt`/`finish_run`/`active_runs`）；`[[subagent.providers]]` 配置。**`OrbitSetup.providers` seam 已删除**（装配上移 daemon，clean cutover）。仍缺：continuable/background run、`send_message`/`report`、session-seeding、SIGTERM 中间层、permission option kind、Codex/Claude Code/SDK 三个后端 | **中** — 余量缩小为：三个未接后端 + send_message/report/continuable/session-seeding + SIGTERM 层 |
| **MCP 多传输 + 重连** | `mcp-client` Cordis 插件：stdio + Streamable HTTP 双传输，`RECONNECT_DEFAULTS` 重连，`failOnStartupError` 熔断，per-tool timeout，`cwd` 每服务（`mcp-client/src/{index,transport,connection}.ts`） | **已落地（#381，2026-09-18）**：stdio + Streamable HTTP 双传输、重连监督（指数退避+熔断语义的重试上限）、per-server timeout/cwd、fail_on_startup_error、daemon orbit_setup 注入 | 余量：tool-level filter、server-level env 注入等 dsh 可选项未对齐 |
| **session resume 生产路径** | `SessionPersistence` 抽象（`prepare`/`load`/`inspect`/`readFrom`）+ JSONL/SQLite 双后端 + `session-checkpoint-policy` 在 llm/tools/pre-step 前 flush（`session/session-persistence*/src/index.ts`） | **已落地（#382 B2a，2026-09-18）**：daemon 重启后 worker 按 session 回放最近 50 条 user/assistant 历史（dedupe + 切换清 ctx） | 余量：checkpoint flush 策略；回放跳过 System/Tool 行 |
| **附件全链路** | `AttachmentStore` 抽象 + `LocalAttachmentStore` 内容寻址 + `ui-attachment` 前端 + adapter `resolveAttachments` 注入（`attachment/attachment*/src/index.ts`） | omenic 全仓 grep `attachment`/`image` = 0（仅 `ETXTBSY` 误匹配） | **中** — 前端 UI + 后端存储 + 上下文投影三段全缺 |
| **LLM provider 注册表/路由 + retry + token-meter** | `LlmRuntime` 服务（`registerAdapter`/`stream` waterfall）+ DeepSeek/PiAi 双 adapter + `llm-retry` 插件 + `TokenMeter`（`llm/llm*/src/index.ts`） | **waterfall 已落地（#382 B2b，2026-09-18）**：orbit::WaterfallLlm（[[llm.fallbacks]] 按序切换 + per-provider RetryPolicy 退避 + 中间 Error 吞噬）+ web settings Fallback 表单 | token-meter 不做（C8 裁定维持）；DeepSeek/PiAi 官方 adapter 未接（现走 OpenAI 兼容端点） |
| **jobs 后台作业 + terminal 持久 PTY** | `JobRegistry` 抽象 + `LocalJobRegistry` + `terminal` PTY 后端（bash/pwsh）+ 6 个模型工具（`jobs/jobs*/src/index.ts`、`terminal/terminal*/src/index.ts`） | **已落地（#385，2026-09-19）**：`crates/harness/jobs`（`JobRegistry` + `LocalJobRegistry`，`std::thread` + Condvar）+ `crates/harness/terminal`（portable-pty，drain 语义 read）；10 把模型工具经 `OrbitConfig.session_tools` 接入 daemon；web 工具词表归一化 | 余量：`onJobDone` 回调、pwsh 后端、dsh jobs 其余工具（`jobs_output` 等） |
| **session telemetry/otel + title-llm** | `SessionTelemetryBackend` 抽象 + OTel SDK 导出 + `SessionTitleService`（确定性 fallback + LLM 生成）（`session/session-telemetry*/src/index.ts`、`session/session-title*/src/index.ts`） | omenic 全仓 grep `session_telemetry`/`opentelemetry`/`session-title`/`title-llm` = 0 | **低** — 无 OTel 可观测性管道，无自动 session 标题 |
| **credentials/authorization + identity** | `CredentialProvider` 抽象（分层 env 解析 + YAML 持久化 + 跨进程锁）+ `AuthorizationService` + `AnonymousUserId`（`credentials/*/src/index.ts`、`identity/*/src/index.ts`） | omenic 全仓 grep `credentials`/`identity`/`anonymous-user-id` = 0 | **低** — 无托管凭据存储、无 OAuth 授权流、无稳定匿名身份 |

### 已复刻但仍有功能性偏差（G8 已修生产路径缺陷）

| 域 | 偏差说明 |
|---|---|
| run 收尾时机 / 配置写回抹段 / spill 碰撞 | G8（#377/#378）已修。详见上表 G8 行。 |
| 压缩配对方向 | omenic `cut += 1` 前缩 vs dsh `keepFromIdx -= 1` 后扩，omenic 丢弃更多原文；阈值固定字符而非按窗口比例。 |

### 边界决定（稳定，勿翻案）

**明确不做**（dsh 有但 omenic 不复刻）：`hooks`（外部 agent hook 协议）、`skill`（SKILL.md 加载）、`guard`（timeout-policy/repeat-reminder）、`lsp`、`sandbox`、`subprocess`、`e2b`、react `ui-*`（40+ 包）、以及 dsh 的 Node ESM loader 层（`vendor/loader`，C6 只对齐 `vendor/cordis`）。

**独有功能保护区**（omenic 独有，dsh 无对应物——任何路线**只读/单向依赖**，不许重构、不许当缺口往里塞）：
- `crates/infra/memory`（jcode：`embed/graph/recall/inject/pipeline`）
- `crates/agent/task`（`runner/graph/store/template` RPC 任务模型；dsh 的 `workflow` 是模型在运行时自己写编排，方向相反，不算对应物）
- `crates/evidence/spec` + `bin/gate`（合规工具；远期归宿 = C6 插件面落地后注册成工具插件，现在不动）

> **2026-09-16 更正**：此前本表把 `crates/agent/subagent` 与 `crates/agent/mcp` 也列为「dsh 无对应物」，经核对 dsh 源码**不成立**——dsh 有 `packages/subagent/`（11 子包：spawn/fork 进程内后端 + ACP/Codex/Claude Code/SDK 四个进程外后端 + control/report 工具）与 `packages/mcp/mcp-client/`（多传输客户端）。omenic 的 subagent 相当于 `subagent-spawn-in-process` 的只读工具简化版；**Phase 1 已接 daemon 生产路径（#380）**，**Phase 4 的 ACP 出进程后端 + interrupt 亦已落地（#387，2026-09-19）**；mcp 客户端自 #381（2026-09-18）起已对齐 dsh 的 stdio + Streamable HTTP 双传输形态。两者移出保护区，按普通缺口排优先级。

## 历史锚点

起点 `53419ec` / `906ea2e` / `1b405f0`；设计蓝图 `todo/dsh/README.md`；冻结签名锚点 `todo/archive/dsh/BACKGROUND.md`（已归档，S1–S4 全部交付后的历史锚点）；三路交叉校验记录 `todo/roadmap-verify-2026-09-15.md`。
