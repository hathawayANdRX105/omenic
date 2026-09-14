# omenic ROADMAP

> 长期重构方向 + 并发分工。历史进度见 git log（`53419ec` / `906ea2e` / `1b405f0` 起），`todo/dsh/README.md` 为设计蓝图，`todo/dsh/BACKGROUND.md` 为现状锚点（冻结签名 / `AgentEvent` 契约）。
> 最后更新：2026-09-13

## 重构终点（先写死，agent 据此找缺口并更新路线）

**完成 = 以下能力全部可演示。** 格式：能力 → 小功能 → 文件（omenic 目标 | dsh 参考，dsh 路径相对 `~/projects/harness/deepseek-harness`，已核实）。

### C1 agent 循环 ✅ 已有

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 1.1 循环 + 5 不变式（工具成对/截断不执行/中止不孤儿/压缩失败保原文/max_turns） | `crates/agent/orbit/src/lib.rs`（`run_agent_streaming:427`，22 集成测试） | `packages/core/agent-loop/src/{agent.ts,tool-calls.ts,invariant.ts}` |
| 1.2 流式 LLM（OpenAI 兼容 + SSE） | `crates/agent/adaptor/src/{openai.rs,sse.rs}` | `packages/llm/llm-pi-ai/src/` |
| 1.3 10 内置工具 + `Guarded` 策略包装 | `crates/agent/tools/src/`（10 文件 + `builtin_tools:319`） | `packages/core/tools/src/` + `packages/fs/` |
| 1.4 内嵌压缩（120k 字符策略，将被 C4 抽插件） | `crates/agent/orbit/src/lib.rs:249-397`（`select_compaction_cut:285`） | — |

### C2 会话持久化 + crash-repair 🟡 存储有、repair 无

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 2.1 会话 CRUD + libSQL 持久化 | `crates/infra/session/src/lib.rs:SessionDb:181`（727 行，已有） | `packages/core/session/src/index.ts` |
| 2.2 事件词汇表（`WorkerEvent` 去 `Unknown(Value)` 兜底） | `crates/infra/rpc/src/worker.rs:WorkerEvent:47` | `packages/core/session/src/{known-event-types.ts,types.ts}` |
| 2.3 无损分块编解码 | `crates/infra/session/src/lib.rs`（追加） | `packages/core/session/src/{chunk-rows.ts,json.ts}` |
| 2.4 crash-repair（半程 run 标 Aborted 不丢） | `crates/infra/session/src/lib.rs`（追加 repair 段） | `packages/core/session/src/{repair.ts,request-header.ts}` |

### C3 事件流可推 web/tui ⬜ 预留未实现

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 3.1 `AgentEvent` serde DTO 定稿（snake_case tag，跨 crate 契约） | `crates/agent/orbit/src/lib.rs:AgentEvent:81`（加 serde derives） | `packages/core/session/src/types.ts` |
| 3.2 `WorkerEvents` 推形态（`subscribe → Receiver`，保留轮询兜底） | `crates/infra/rpc/src/worker.rs:WorkerEvents:235` | `packages/client/runtime/src/client/sessions/{notifier.ts,service.ts}` |
| 3.3 daemon `event.subscribe` 命令 + notifier 广播（UDS 多订阅者，断连清理） | `crates/infra/daemon/src/{protocol.rs:185,state.rs:295,dispatch.rs:411}` | `packages/sdk/server/src/server.ts` + `packages/api/gateway/src/{types.ts,client}` |
| 3.4 e2e：起 worker → run → 收完整 `TurnStart…TurnEnd` 序列 | `crates/infra/daemon/tests/`（新） | `packages/core/session/src/invariant.ts` |

### C4 compaction 插件 + 指令注入 🟡 compaction 内嵌、instruction 无

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 4.1 `CompactionPolicy` trait + 默认实现（120k 策略平移） | `crates/harness/compaction/src/`（新 crate） | `packages/compaction/compaction/src/{index.ts,checkpoint.ts}` |
| 4.2 tool_call/tool_result 成对裁剪不变式 | 同上（`pairing.rs`） | `packages/compaction/compaction/src/tool-pairing.ts` |
| 4.3 预算分区（system/工具/最近 N 条区） | 同上（`region.rs`） | `packages/compaction/compaction-basic/src/{region.ts,types.ts,config.ts}` |
| 4.4 LLM 摘要钩子（超预算区段 → 摘要替换原文） | 同上（`summarize.rs`） | `packages/compaction/compaction-basic/src/summarizer.ts` |
| 4.5 orbit maintenance 接缝替换（`select_compaction_cut`+`compact_context` 搬出，≤20 行，22 测试不改断言） | `crates/agent/orbit/src/lib.rs:285-397` | — |
| 4.6 AGENTS.md 向上查找 + 状态缓存 | `crates/harness/instruction/src/`（新 crate） | `packages/context/agent-instructions/src/{files.ts,state.ts}` |
| 4.7 指令 digest 去重 + 渲染 `PromptTemplate` | 同上（`render.rs`） | `packages/context/agent-instructions/src/{render.ts,digest.ts,config.ts}` |

### C5 web 页面跑真数据 🟡 5.1/5.2a/5.2b ✅（#340/#341/#345）：读侧真数据 + 事件订阅端 + 真运行全通；5.3 🟡（谱系待 G4）；5.4/5.6 由订阅管线承载（展示层已就位）、5.9 删 mock 待 G4 验收后

> **G4 联调阻塞点（R4 侧已就绪，剩余整合项）**：
> 1. **单 worker 事件无会话归属**——并发 turn 事件会混写最近 on_send 的会话。处理：G4 联调期约定「单运行」纪律（daemon 一次只跑一轮 turn）；根治需 R2 给 WorkerPrompt/事件帧加 per-run 路由（协议改动权在 R2）。
> 2. **断线重连无自动化测试**——读线程退避重连（1s→5s 封顶）已实现，重连后 translator 重建语义有单测；G4 联调实测「杀 daemon 再起，web 自动重连不白屏」（验收③）。
> 3. **半开 run 显示 aborted**——`SessionSummary` 无状态字段（convert 统一映射 Idle）。处理：G4 联调实测 2.4 repair 后 list 侧暴露 run 状态；如 R2 未加字段，web 侧先用 `read_from_cursor` 的 run 记录组装。
> 4. **G4 验收①⑤已具备载体**：① 3.4 e2e（R2 的 event_push.rs）+ ⑤ web 走 `worker.prompt`/订阅，不做 `worker.read_event` 轮询。
> **实测记录（2026-09-14，G4 rig /tmp/g4：daemon + mock-omp + oi-web 三件套）**：服务端事件链路探针 PASS×2（subscribe attach → worker.prompt → `agent_start…agent_end` 全序列推送，daemon 重启前后各一轮）；杀 daemon 期间 web 全程 200 不白屏，重启后链路恢复——验收③服务端部分通过；页面级实时 delta 由验收②载体（订阅消费端）承载，待浏览器实测确认。

> **5.2a 整合注意（功能）**：① 写路径归 daemon——web 经 `session.append`/`worker.prompt` 调用，不直接写 sessions.db（避免双写 libSQL，2.4 repair 语义依赖 daemon 侧写）；② `worker.read_event` 是消费式出队，web 禁止轮询（会抢 CLI/task/memory 事件）；③ G1 冻结后 web 的 `AgentEvent` fixture 换 orbit 冻结 DTO（转译签名不变）；④ `DaemonClient` 同步短连接 × LiveView async → `spawn_blocking`；⑤ oi-web 与 daemon 必须同指一个 data_dir（worktree 各有 `.oi`），设置页加 daemon ping 校验；⑥ 谱系（5.5）无服务端命令，先平铺、G4 后按 `run.list` 组装。

| 小功能 | omenic 文件（拆后 crate） | dsh 参考 |
|---|---|---|
| 5.1 ✅ `AgentEvent` DTO + 转译层（→ UI 状态，纯函数可单测，先行开发不等 C3；mock 流已消费，4 测试） | `crates/web/state/`（新，含 `AgentEvent`→UI 状态转译 + `memory_link`） | `packages/core/session/src/surface.ts` |
| 5.2a ✅ 已实现（PR #341）：daemon RPC 读客户端（会话列表/历史/messages/search，封装现有 `session.*`/`run.list` 协议 + `SessionSummary/SessionMessage` → state DTO 转换；`DataBackend` 探测失败静默回退 mock） | `crates/web/client/`（新增 `daemon.rs`；`llm.rs` 保留） | `packages/client/runtime/src/client/sessions/{manager.ts,remotes.ts}`（pull 模式已核实与服务端 `session.list/search/create/history` 一一对应） |
| 5.2b ✅ 已实现（订阅接收端：`WireTranslator` wire→AgentEvent 双形状兼容 + 工具同名 LIFO 配对合成 id；读线程断线退避重连 1s→5s；`worker.prompt` 真运行；error 帧→TurnEnd 兜底防输入锁死；Mock 分支原样回退） | `crates/web/client/`（`daemon.rs` 追加订阅 + 重连）、`page-workspace`（订阅管线） | `packages/client/runtime/src/client/sessions/{notifier.ts,service.ts}` |
| 5.3 🟡 会话列表/历史 ← 真数据（Daemon 模式已接 `session.list`/`load_messages`；谱系分组待 G4 后按 `run.list` 组装） | `crates/web/page-workspace/`（原 workspace.rs 1093 行） | `packages/client/runtime/src/client/sessions/{session.ts,lineage.ts}` |
| 5.4 聊天流式：delta 追加 + tool 折叠卡 | `crates/web/components/`（chat.rs 352 行） | `client/conversation/{event-registry.ts,view-registry.ts}` + `sessions/tool-call-tree.ts` |
| 5.5 sidebar 真会话 + 谱系分组 | `crates/web/components/`（sidebar.rs） | `packages/client/runtime/src/client/sessions/lineage.ts` |
| 5.6 statusline 真运行态 + 计时 | `crates/web/components/`（statusline.rs） | `packages/client/runtime/src/client/sessions/assistant-timing.ts` |
| 5.7 stats 接 token 真数据（无则隐藏该卡；完整需 C8.3） | `crates/web/page-stats/`（原 stats.rs + statsview.rs） | `packages/llm/token-meter/src/{usage-projection.ts,projection.ts}` |
| 5.8 配置页读写 `infra/config`（TOML 往返） | `crates/web/page-config/`（原 config_page.rs） | `packages/settings/settings-file/src/index.ts` |
| 5.9 `mock.rs` 删除（全仓 grep `mock_sessions\|mock_messages` 命中 = 0） | `crates/web/client/src/mock.rs`（931 行，过渡存放于此） | — |
| 5.10 ui-validate 验收（`specs/ui/*.yaml` + 浏览器实测，序列见仓库 AGENTS.md） | `bin/web/tests/` + `specs/ui/` | — |

### C6 插件面四件套 ⬜ 零代码

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 6.1 服务注册容器（静态 `HashMap<TypeId, Arc<dyn Any>>`，无 reflect 代理） | `crates/harness/plugin/src/`（新 crate，`context.rs`+`service.rs`） | `vendor/cordis/src/{context.ts,service.ts}` |
| 6.2 事件总线（同步顺序派发，插件卸载自动注销 handler） | 同上（`events.rs`） | `vendor/cordis/src/events.ts` |
| 6.3 插件生命周期（on_load/on_unload，Drop 逆序，无 effect unwind） | 同上（`fiber.rs`） | `vendor/cordis/src/fiber.ts` |
| 6.4 插件注册表 + `plugins()` 列举（重名拒绝） | 同上（`registry.rs`） | `vendor/cordis/src/registry.ts` |
| 6.5 组装根（按 manifest 顺序 register orbit/adaptor/tools） | `crates/composition/src/lib.rs`（现 3 行占位 → 扩） | `packages/bundle/base/src/index.ts` |
| 6.6 orbit `run_agent` 服务化接线（≤30 行薄层，循环逻辑零改动） | `crates/agent/orbit/src/lib.rs:617` | `packages/core/agent-loop/src/index.ts` |

### C7 tag + ferrite 接线 ⬜

| 小功能 | 文件 | 说明 |
|---|---|---|
| 7.1 C1–C6 全绿 + 全仓 `cargo test` | `crates/composition/` 统一装配 | G5 触发 |
| 7.2 `tag omenic-harness-v0.1.0` | — | ferrite 侧 git 依赖按 tag 锁定 |
| 7.3 ferrite 根 `Cargo.toml` 删本地 harness member，加 tag git 依赖；本地 `[patch]` → 阶段 4 后删 | ferrite 仓库 | 对照旧「阶段 3」 |

### C8（可选，酒馆触发）：interaction + token-meter + llm-retry


| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 8.1 ask-user / user-questions trait（web 消费） | `crates/harness/interaction/src/`（新） | `packages/interaction/{user-questions,tool-ask-user}/src/` |
| 8.2 approval 策略 + 权限预设 | 同上（`approval.rs`） | `packages/interaction/{user-approval,permission-presets}/src/` |
| 8.3 token-meter 投影（C5.7 stats 完整数据源） | `crates/harness/metering/src/`（新） | `packages/llm/token-meter/src/{usage-projection.ts,projection.ts,client.ts}` |
| 8.4 llm-retry（流式失败重试） | `crates/agent/adaptor/src/retry.rs`（追加） | `packages/llm/llm-retry/src/{history.ts,types.ts}` |

**明确不做**（dsh 有但 omenic 不复刻）：`hooks`（外部 agent hook 协议）、`skill`（SKILL.md 加载）、`guard`（timeout-policy/repeat-reminder）、`lsp`、`sandbox`、`subprocess`、`e2b`、react `ui-*`（40+ 包）。

**独有功能保护区**（omenic 独有，dsh 无对应物——任何路线**只读/单向依赖**，不许重构、不许当缺口往里塞）：
- `crates/infra/memory`（jcode：`embed/graph/recall/inject/pipeline`）— 记忆层，C1 已有
- `crates/agent/task`（`runner/graph/store/template` RPC 任务模型）— C1 已有
- `crates/agent/subagent`、`crates/agent/mcp` — C1 已有
- `crates/evidence/spec` + `bin/gate`（合规工具：spec 表生成/校验 + GitHub 产物规则，被 gate/cli 两入口依赖；dsh 无对应物，远期归宿 = C6 插件面落地后注册成工具插件，现在不动）— 任何路线只读/单向依赖
- R2 改 `daemon/protocol.rs` **只能加命令**（`event.subscribe`），**不许改既有 `session.*` / `run.list` 命令语义**（task/memory 的 RPC 依赖这些命令）

## 并发路线（小功能可并行的工作包，文件所有权互斥）

> tui 已删（C6 原 tui 冒烟取消）；酒馆（C8）不在 omenic 范围，仅占位。实际并发 = R1–R4。


| 路线 | 覆盖小功能 | 独占文件 | 前置 | 完成触发 |
|---|---|---|---|---|
| **R1 插件面** | C6（6.1–6.6） | `crates/harness/plugin/`（新）、`crates/composition/`；orbit ≤30 行 | 无 | G1（6.1+6.5 定型）→ G2（全绿） |
| **R2 事件流+修复** | C2（2.2–2.4）+ C3（3.1–3.4） | `crates/infra/{daemon,session,rpc}/`；orbit 只读 | 3.1 依赖 6.1 定型 | G4 |
| **R3 核心插件** | C4（4.1–4.7） | `crates/harness/{compaction,instruction}/`（新）；orbit maintenance 接缝 ≤20 行 | 4.5 接缝 + `impl DshPlugin` 需 G1 | G3 |
| **R4 web** | C5（5.1–5.10） | `crates/web/{client,state,components,page-workspace,page-stats,page-config}/`（6 个 crate，crate 名 `omenic-web-*`；壳=App/launch/build.rs/tailwind 全在 `bin/web/`，bin/web 是入口 crate 不进 crates） | 5.1 ✅；5.2a/5.3/5.8 可先行（现有 `session.*`/`run.list` 协议，不算 3.3 依赖）；5.2b/5.4/5.6/5.7/5.9 等 3.3 定稿 | G4 联调 |
| **R5（占位，酒馆触发，不在 omenic 范围）** | C8（8.1–8.4） | `crates/harness/{interaction,metering}/`（新）、`adaptor/retry.rs` | G1 + 酒馆需求确认 | — |

**冲突仲裁**（每次合并都处理）：
- 根 `Cargo.toml` members/Cargo.lock：各路线只加自己 crate 一行，合并顺序 R1→R3→R2→R4，后合者 rebase
- `composition` 只 R1 可写；他路线装配需求走 issue
- orbit 两处接缝限额共享：R1 ≤30 行（6.6）+ R3 ≤20 行（4.5），超限额走 issue
- 独有功能保护区（见上）：`infra/memory`、`agent/task`、`agent/subagent`、`agent/mcp` 任何路线只读/单向依赖
- `AgentEvent` serde（3.1）、daemon protocol（3.3）、harness trait（6.1）三个契约改动权归首发路线，他路线按冻结类型消费

## 整合门（开发到什么阶段，必须停下来整合什么）

| 门 | 触发 | 整合动作（验证什么功能正常） | 冲突本质 |
|---|---|---|---|
| **G1 契约冻结** | R1 的 6.1 + 6.5 完成 | 广播：6.1 插件面 trait 签名 + 3.1 `AgentEvent` serde 为全路线唯一契约。**验证**：`cargo test -p omenic-harness-plugin` 全绿（6 个集成测试）；`AgentEvent` 各变体 serde 往返一致（`serde_json` round-trip 测试）。R3/R4 的 fixture 从此只能用冻结类型。 | EventBus 类型是 R2/R3 共同进口 |
| **G2 C6 收编** | R1 全绿 | orbit 6.6 接线合并；**验证**：orbit 22 测试全量回归（不改断言）+ `oi task add` → 流式 run → 事件流 → 持久化全链路（C1 验收）仍通；C6 勾满。 | R1 与 R3 同碰 orbit，接缝限额共享 |
| **G3 插件回归闸** | R3 完成（C4 全绿） | compaction 切插件 + instruction 注入后；**验证**：① orbit 22 测试仍绿（接缝未破坏循环）；② `oi` 起一次 >120k 字符长会话，压缩路径端到端不炸（checkpoint 快照 + 失败保原文）；③ AGENTS.md 注入：在含 AGENTS.md 的目录跑 run，system prompt 里出现该片段；④ web 页打开一次真实 run，压缩不中断流式渲染。 | orbit 行为变更影响所有宿主 |
| **G4 事件流汇合** | R2 绿 **且** R4 的 5.1 DTO 完成 | R4 删 fixture 切实时 daemon；**验证**：① 3.4 e2e 绿（worker→run→收完整 `TurnStart…TurnEnd`）；② web 聊天页流式 delta 实时追加（非轮询）；③ 断线重连：杀 daemon 再起，web 自动重连不白屏；④ 半开 run 显示：手动中断 run 后刷新页面，会话列表里该 run 标 `aborted` 而非消失（对齐 2.4 repair 语义）；⑤ web 经 `worker.prompt` 起 run 与 CLI 事件互不抢占（web 不做 `worker.read_event` 轮询——那是消费式出队）。 | **最大冲突点**：daemon protocol 改动权在 R2 |
| **G5 总装/tag** | G1–G4 全过 | composition 统一装配四路线产物；**验证**：① C1–C7 全勾；② 全仓 `cargo test` 绿；③ `oi-web` 起在 8026，硬刷新后无 mock 残留（`curl localhost:8026 \| grep mock` = 0）；④ `tag omenic-harness-v0.1.0` 后 ferrite 侧 `git pull` 编译过。 | 装配根只在 G5 集中改 |

**顺序铁律**：G1 之前任何 `register`/`provide` 代码不得合入；G4 之前 R4 不得合入依赖 daemon 新协议（3.3）的实际请求路径——读侧（现有 `session.*`/`run.list`）与既有 `worker.prompt` 调用不算 3.3 依赖，允许先行。

## 开新会话须知

- **起点**：`1b405f0`（harness 四 crate 可运行，agent loop 完整；阶段 0/1 见 git log）
- **harness trait 签名已冻结**：`Provider` / `Tool` / `PromptRenderer` / `RunState`（详见 `todo/dsh/BACKGROUND.md` §1 锚点）。改签名必须广播所有会话
- **认领路线**：读本文件「并发路线」表认领 R1–R5 一条，只做独占文件；小功能编号即任务编号（如 R2 = 做 2.2/2.3/2.4/3.1–3.4）
- **依赖铁律**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO
- **工作目录**：`.wt/<branch>`（git worktree add 必须在仓库根执行，防嵌套）
- **测试**：放同层 `tests/`，不在 src/ 写 `#[cfg(test)]`；重型测试推 CI
- **编译**：cargo 命令套 `cpulimit -l 70 -i --`，只跑 `-p <crate>`，禁止全仓 build
