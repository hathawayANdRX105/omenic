# omenic ROADMAP

> 长期重构方向 + 并发分工。历史进度见 git log（`53419ec` / `906ea2e` / `1b405f0` 起），`todo/dsh/README.md` 为设计蓝图，`todo/dsh/BACKGROUND.md` 为现状锚点（冻结签名 / `AgentEvent` 契约）。
> 最后更新：2026-09-15（R1/R2/R3 已合并 #339/#343/#346；R4 web #340-#352；G3 验收②③ 与 G4 验收③④ 的自动化载体由 #354-#357 补齐；#358 校正本文件失真。**G4 五项验收①②③④⑤ 全部通过**——①③④⑤ 自动化测试 + ② 用户浏览器实测确认流式输出（2026-09-15）。**G4 已过，G5 开工。**）
> **校验记录**：`todo/roadmap-verify-2026-09-15.md`（三路交叉校验，含 16 条虚报/漏点清单）；本轮路线见 `todo/route-to-g4-2026-09-15.md`。
> **范围裁定（2026-09-15）**：**C7 tag 与 C8 酒馆暂不做**——tag 等 G5 装配完再议；C8 不在 omenic 范围。G5 现阶段只做「composition 真装配 + 删 mock」。
> **G4 验收操作手册**：见本文末「G4 验收指南（用户实测）」一节。
> 小功能行内 ✅ = 已合并 main；🟡 = 部分完成；⬜ = 未开工。

## 重构终点（先写死，agent 据此找缺口并更新路线）

**完成 = 以下能力全部可演示。** 格式：能力 → 小功能 → 文件（omenic 目标 | dsh 参考，dsh 路径相对 `~/projects/harness/deepseek-harness`，已核实）。

### C1 agent 循环 ✅ 已有

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 1.1 循环 + 5 不变式（工具成对/截断不执行/中止不孤儿/压缩失败保原文/max_turns） | `crates/agent/orbit/src/lib.rs`（`run_agent_streaming:359`，**23** 集成测试 = loop.rs 20 + agent_event_serde.rs 3） | `packages/core/agent-loop/src/{agent.ts,tool-calls.ts,invariant.ts}` |
| 1.2 流式 LLM（OpenAI 兼容 + SSE） | `crates/agent/adaptor/src/{openai.rs,sse.rs}` | `packages/llm/llm-pi-ai/src/` |
| 1.3 10 内置工具 + `Guarded` 策略包装 | `crates/agent/tools/src/`（10 文件 + `builtin_tools:319`） | `packages/core/tools/src/` + `packages/fs/` |
| 1.4 内嵌压缩（120k 字符策略，将被 C4 抽插件） | `crates/agent/orbit/src/lib.rs:249-397`（`select_compaction_cut:285`） | — |

### C2 会话持久化 + crash-repair ✅（PR #343）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 2.1 ✅ 会话 CRUD + libSQL 持久化 | `crates/infra/session/src/lib.rs:SessionDb` | `packages/core/session/src/index.ts` |
| 2.2 ✅ 事件词汇表（`WorkerEvent` 拆出 ToolExecutionStart/End/Error 专属变体，Unknown 只留给未识别帧） | `crates/infra/rpc/src/worker.rs` | `packages/core/session/src/{known-event-types.ts,types.ts}` |
| 2.3 ✅ 编解码就绪，⚠️ **零生产写入**（`encode/decode_turn_log` 唯一调用者是 turn_repair.rs 测试；orbit TurnStart/TurnEnd 是内存事件，从未落 TurnRecord JSONL——session 持久化走 libSQL） | `crates/infra/session/src/lib.rs`（`TurnRecord` + encode/decode） | `packages/core/session/src/{chunk-rows.ts,json.ts}` |
| 2.4 ✅ repair 函数就绪，⚠️ **零生产调用**（`interrupted_run_closers` 唯一调用者是测试；`Daemon::start` 无 repair 步骤。用户侧「aborted」实际由 web `run.list` 三态推断承担，见 `infer_session_status`。接线单独处理，不阻塞 G5） | `crates/infra/session/src/lib.rs`（`interrupted_run_closers`） | `packages/core/session/src/{repair.ts,request-header.ts}` |

### C3 事件流可推 web ✅（PR #343）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 3.1 ✅ `AgentEvent` serde DTO（snake_case tag，跨 crate 契约；orbit 已有 derives） | `crates/agent/orbit/src/lib.rs:AgentEvent` | `packages/core/session/src/types.ts` |
| 3.2 ✅ `WorkerEvents` 推形态（`subscribe → Receiver` + pump 线程，保留 `read_event` 轮询兜底） | `crates/infra/rpc/src/worker.rs` | `packages/client/runtime/src/client/sessions/{notifier.ts,service.ts}` |
| 3.3 ✅ daemon `event.subscribe` + `EventBus` 广播（UDS 多订阅者，断连自动清理） | `crates/infra/daemon/src/{protocol.rs,state.rs,dispatch.rs}` | `packages/sdk/server/src/server.ts` + `packages/api/gateway/src/{types.ts,client}` |
| 3.4 ✅ e2e：起 worker → run → 收完整 `TurnStart…TurnEnd` 序列 | `crates/infra/daemon/tests/event_push.rs` | `packages/core/session/src/invariant.ts` |

### C4 compaction 插件 + 指令注入 ✅（PR #346）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 4.1 ✅ `CompactionPolicy` trait + 默认实现（120k 策略平移） | `crates/harness/compaction/src/policy.rs` | `packages/compaction/compaction/src/{index.ts,checkpoint.ts}` |
| 4.2 ✅ tool_call/tool_result 成对裁剪不变式 | 同上（`pairing.rs`） | `packages/compaction/compaction/src/tool-pairing.ts` |
| 4.3 ✅ 预算分区（system/工具/最近 N 条区） | 同上（`region.rs`） | `packages/compaction/compaction-basic/src/{region.ts,types.ts,config.ts}` |
| 4.4 ✅ LLM 摘要钩子（超预算区段 → 摘要替换原文） | 同上（`summarize.rs`） | `packages/compaction/compaction-basic/src/summarizer.ts` |
| 4.5 ✅ orbit maintenance 接缝替换（`select_compaction_cut`+`compact_context` 搬出，loop.rs 20 测试零改动）；⚠️ **接缝实测 56 代码行 / 78 原始行**（lib.rs:265-342），超 R3 ≤20 限额 36 行——见下方「接缝超限」注 | `crates/agent/orbit/src/lib.rs` | — |
| 4.6 ✅ AGENTS.md 向上查找 + 状态缓存 | `crates/harness/instruction/src/files.rs` | `packages/context/agent-instructions/src/{files.ts,state.ts}` |
| 4.7 ✅ 指令 digest 去重 + 渲染 `PromptTemplate` | 同上（`render.rs`） | `packages/context/agent-instructions/src/{render.ts,digest.ts,config.ts}` |
| 4.8 ✅ **orbit system prompt 注入接线（#354）**：`LoopConfig.instruction_cwd` 显式旋钮 → `build_system_prompt` 查找+去重+前置到 TASK 之前；4 测试（注入/空/去重/降级） | `crates/agent/orbit/src/lib.rs`（`build_system_prompt:370`）+ `tests/instruction_prompt.rs` | `packages/context/agent-instructions/src/render.ts` |

### C5 web 页面跑真数据 🟡 5.1/5.2a/5.2b ✅；读侧真数据 + 事件订阅端 + orbit 真运行全通（wildtoken agnes-2.5-flash，用户浏览器实测确认对话成功）；5.3 🟡（谱系待 G4）；5.4 ✅ 由订阅管线承载；5.6 🟡 运行态有、**elapsed 计时未做**；5.9 删 mock 是 G5 的活
> **本轮新增修复**：form/onsubmit→Dioxus onclick/onkeydown 替换（#351，dioxus-liveview 解释器不监听 submit 事件——源码实证）；按会话区分运行状态（#350，停止钮/门禁由当前会话 Active 驱动）；orbit 引擎 turn 非阻塞化（#350，prompt 专用线程 + ack + abort 可达）；聊天滚动区底部 padding 修正（#352）

> **G4 阻塞点清零状态（2026-09-15，#353-#357 后）**：
> 1. **单 worker 事件无会话归属**——仍开放。并发 turn 事件会混写最近 on_send 的会话。G4 联调期约定「单运行」纪律（daemon 一次只跑一轮 turn）；根治需 R2 给 WorkerPrompt/事件帧加 per-run 路由（协议改动权在 R2）。**注**：#356 已让 web 侧的 prompt 带 session_id/run_id 归属。
> 2. ~~断线重连无自动化测试~~ **已清零（#357）**：`client/tests/reconnect.rs` 3 测试覆盖杀→断→起→恢复完整窗口 / 退避输入 / 不白屏。CI 实测：daemon shutdown 后 `next_event` 在超时内返回 `Ok(None)`（不主动 close 已派发订阅流），读线程不挂死、可进入下一轮退避。
> 3. ~~半开 run 显示 aborted~~ **已清零（#356）**：`SessionSummary` 仍无状态字段（协议零改动），web 侧 `run.list` 推断三态 + `run_status_cache` 信号避免每渲染重查；8 个单测。
> 4. **G4 验收①②⑤ 载体齐备**：① 3.4 e2e（event_push.rs，#353 修复后绿）；② 浏览器实测 + subscribe_loopback；⑤ web 全仓零 `read_event` 调用。
> **实测记录（2026-09-14，G4 rig /tmp/g4：daemon + mock-omp + oi-web 三件套）**：服务端事件链路探针 PASS×2；杀 daemon 期间 web 全程 200 不白屏，重启后链路恢复。浏览器实测（2026-09-15）：对话成功，真模型回复流式可见。

> **5.2a 整合注意（功能）**：① 写路径归 daemon——web 经 `session.append`/`worker.prompt` 调用，不直接写 sessions.db（避免双写 libSQL，2.4 repair 语义依赖 daemon 侧写）；② `worker.read_event` 是消费式出队，web 禁止轮询（会抢 CLI/task/memory 事件）；③ G1 冻结后 web 的 `AgentEvent` fixture 换 orbit 冻结 DTO（转译签名不变）；④ `DaemonClient` 同步短连接 × LiveView async → `spawn_blocking`；⑤ oi-web 与 daemon 必须同指一个 data_dir（worktree 各有 `.oi`），设置页加 daemon ping 校验；⑥ 谱系（5.5）无服务端命令，先平铺、G4 后按 `run.list` 组装。

| 小功能 | omenic 文件（拆后 crate） | dsh 参考 |
|---|---|---|
| 5.1 ✅ `AgentEvent` DTO + 转译层（→ UI 状态，纯函数可单测，先行开发不等 C3；mock 流已消费，**15 测试** = convert 4 + ui_state 4 + wire_translate 7） | `crates/web/state/`（含 `AgentEvent`→UI 状态转译 + `memory_link`） | `packages/core/session/src/surface.ts` |
| 5.2a ✅ 已实现（PR #341）：daemon RPC 读客户端（会话列表/历史/messages/search，封装现有 `session.*`/`run.list` 协议 + `SessionSummary/SessionMessage` → state DTO 转换；`DataBackend` 探测失败静默回退 mock） | `crates/web/client/`（新增 `daemon.rs`；`llm.rs` 保留） | `packages/client/runtime/src/client/sessions/{manager.ts,remotes.ts}`（pull 模式已核实与服务端 `session.list/search/create/history` 一一对应） |
| 5.2b ✅ 已实现（订阅接收端：`WireTranslator` wire→AgentEvent 双形状兼容 + 工具同名 LIFO 配对合成 id；读线程断线退避重连 1s→5s；`worker.prompt` 真运行；error 帧→TurnEnd 兜底防输入锁死；Mock 分支原样回退） | `crates/web/client/`（`daemon.rs` 追加订阅 + 重连）、`page-workspace`（订阅管线） | `packages/client/runtime/src/client/sessions/{notifier.ts,service.ts}` |
| 5.3 🟡 会话列表/历史 ← 真数据（Daemon 模式已接 `session.list`/`load_messages`；**状态三态已接 #356**：`run.list` 推断 Idle/Active/Aborted；谱系分组待 G4 后按 `run.list` 组装） | `crates/web/page-workspace/`（原 workspace.rs 1093 行） | `packages/client/runtime/src/client/sessions/{session.ts,lineage.ts}` |
| 5.4 聊天流式：delta 追加 + tool 折叠卡 | `crates/web/components/`（chat.rs 352 行） | `client/conversation/{event-registry.ts,view-registry.ts}` + `sessions/tool-call-tree.ts` |
| 5.5 sidebar 真会话 + 谱系分组 | `crates/web/components/`（sidebar.rs） | `packages/client/runtime/src/client/sessions/lineage.ts` |
| 5.6 statusline 真运行态 ⚠️ **计时未做**（会话三态 + header「运行中」spinner + TurnEnd 结算 tokens/cost/context 已有；dsh `assistant-timing` 的 elapsed 计时零代码，`StatusLine` 无计时字段。另注：本行原指 `components/statusline.rs` **不存在**，状态行内联在 `chat.rs:155`） | `crates/web/components/`（内联 chat.rs:155） | `packages/client/runtime/src/client/sessions/assistant-timing.ts` |
| 5.7 stats 接 token 真数据（无则隐藏该卡；完整需 C8.3） | `crates/web/page-stats/`（原 stats.rs + statsview.rs） | `packages/llm/token-meter/src/{usage-projection.ts,projection.ts}` |
| 5.8 配置页读写 `infra/config`（TOML 往返）⚠️ **零测试**（`load_from_system`/`save_to_file`/`test_connection` 已接线并工作，`crates/web/client/tests/` 无一覆盖 TOML 往返） | `crates/web/page-config/`（原 config_page.rs） | `packages/settings/settings-file/src/index.ts` |
| 5.9 `mock.rs` 删除（mock 已搬独立 crate `crates/web/mock/`，仍是 page-workspace/page-stats 直接依赖、传递进 oi-web 二进制；`grep mock_sessions\|mock_messages` = 0 系改名达成，**真删待 G4 验收后**） | `crates/web/mock/`（906 行） | — |
| 5.10 ✅ ui-validate 契约层（PR #344：`specs/ui/*.yaml` ×7 / 71 锚点 + 3 契约测试；浏览器实测序列见仓库 AGENTS.md「Web UI 契约验收」） | `bin/web/tests/` + `.githooks/spec/` | — |

### C6 插件面四件套 ✅（PR #339）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 6.1 ✅ 服务注册容器（`ServiceRegistry`，静态 `HashMap<TypeId, Arc<dyn Any>>`） | `crates/harness/plugin/src/context.rs` | `vendor/cordis/src/{context.ts,service.ts}` |
| 6.2 ✅ 事件总线（同步顺序派发，插件卸载自动注销） | 同上（`events.rs`） | `vendor/cordis/src/events.ts` |
| 6.3 ✅ 插件生命周期（on_load/on_unload，Drop 逆序） | 同上（`fiber.rs`） | `vendor/cordis/src/fiber.ts` |
| 6.4 ✅ 插件注册表 + 重名拒绝 | 同上（`registry.rs`） | `vendor/cordis/src/registry.ts` |
| 6.5 ⚠️ 组装根存在但是死代码（`assemble()` 全仓零调用、零测试、Cargo.toml 自述 placeholder；daemon/web 启动均绕过它；**插件注册留 G5**） | `crates/composition/src/lib.rs` | `packages/bundle/base/src/index.ts` |
| 6.6 ✅ orbit `run_agent` 服务化接线（daemon orbit worker 模式，PR #349/#350） | `crates/infra/rpc/src/worker.rs`（OrbitEngine） | `packages/core/agent-loop/src/index.ts` |

### C7 tag + ferrite 接线 ⏸️ 暂缓（2026-09-15 裁定）

| 小功能 | 文件 | 说明 |
|---|---|---|
| 7.1 C1–C6 全绿 + 全仓 `cargo test` | `crates/composition/` 统一装配 | G5 触发 |
| 7.2 `tag omenic-harness-v0.1.0` | — | **暂缓**：tag 等 G5 装配完成、删 mock 后再议 |
| 7.3 ferrite 根 `Cargo.toml` 删本地 harness member，加 tag git 依赖；本地 `[patch]` → 阶段 4 后删 | ferrite 仓库 | **暂缓**，随 7.2 |

### C8（可选，酒馆触发）：interaction + token-meter + llm-retry ⏸️ 暂不做

> **2026-09-15 裁定**：C8 不在 omenic 范围，仅占位。R5 路线保持占位状态，不分配工作。5.7 stats 的 token 数据源需求改由「无真数据则隐藏该卡」兜底，不为此做 token-meter。


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
| **R1 插件面** ✅ 已合并（#339） | C6（6.1–6.6） | `crates/harness/plugin/`（新）、`crates/composition/`；orbit ≤30 行 | 无 | G1（6.1+6.5 定型）→ G2（全绿） |
| **R2 事件流+修复** ✅ 已合并（#343） | C2（2.2–2.4）+ C3（3.1–3.4） | `crates/infra/{daemon,session,rpc}/`；orbit 只读 | 3.1 依赖 6.1 定型 | G4 |
| **R3 核心插件** ✅ 已合并（#346；验收②③ 由 #354/#355 补齐） | C4（4.1–4.8） | `crates/harness/{compaction,instruction}/`（新）；orbit maintenance 接缝约 65 行 | 4.5 接缝 + `impl DshPlugin` 需 G1 | G3 |
| **R4 web** ✅ 主线完成（#340-#352 + #356/#357） | C5（5.1–5.10） | `crates/web/{client,state,components,mock,page-workspace,page-stats,page-config}/`（**7** 个 crate，crate 名 `omenic-web-*`；壳=App/launch/build.rs/tailwind 全在 `bin/web/`，bin/web 是入口 crate 不进 crates） | 5.1/5.2a/5.2b/5.3/5.4/5.10 ✅；**5.6 计时未做**（运行态三态有）；**5.9 mock 删除是 G5 的活**；5.5 谱系、5.7 stats、5.8 配置页（已接线零测试）留 G5 | G4 ✅ 已过 |
| **R5（占位，暂不做）** ⏸️ | C8（8.1–8.4） | `crates/harness/{interaction,metering}/`（新）、`adaptor/retry.rs` | 2026-09-15 裁定暂缓 | — |
| **R6 总装（G5）** 🟡 开工中 | 6.5 真装配 + 5.9 删 mock | `crates/composition/`、`crates/web/mock/` 摘依赖、page-stats/page-workspace 换数据源 | G4 已过 | G5（不含 tag） |

**冲突仲裁**（每次合并都处理）：
- 根 `Cargo.toml` members/Cargo.lock：各路线只加自己 crate 一行，合并顺序 R1→R3→R2→R4，后合者 rebase
- `composition` 只 R1 可写；他路线装配需求走 issue
- orbit 两处接缝限额共享：R1 ≤30 行（6.6）+ R3 ≤20 行（4.5），超限额走 issue
  - **2026-09-15 实测**：R3 压缩接缝（`orbit/src/lib.rs:265-342`）= 56 代码行 / 78 原始行，**超 R3 ≤20 限额 36 行**。R1 接缝（6.6，orbit::register）在限额内。当前项目只开 PR 不开 issue（见下「工作流」），该超限随 G5 接 WP-A 一并评估是否再切薄，不单独开 issue
- **删 mock 陷阱（G5）**：`page-workspace:245 statusline()` 初值与 `:904 TaskPanel { tasks: store::tasks() }` 在 **Daemon 模式下也仍吃 mock**（不只是 Mock 分支），删 mock 时必须一并接真数据源，否则删不干净
- 独有功能保护区（见上）：`infra/memory`、`agent/task`、`agent/subagent`、`agent/mcp` 任何路线只读/单向依赖
- `AgentEvent` serde（3.1）、daemon protocol（3.3）、harness trait（6.1）三个契约改动权归首发路线，他路线按冻结类型消费

## 整合门（开发到什么阶段，必须停下来整合什么）

| 门 | 触发 | 整合动作（验证什么功能正常） | 冲突本质 |
|---|---|---|---|
| **G1 契约冻结** ✅ 已过（#339） | R1 的 6.1 + 6.5 完成 | 广播：6.1 插件面 trait 签名 + 3.1 `AgentEvent` serde 为全路线唯一契约。**验证**：`cargo test -p omenic-harness-plugin` 全绿（**4** 个集成测试，`plugin_test.rs`）；`AgentEvent` 各变体 serde 往返一致（`orbit/tests/agent_event_serde.rs` 3 测试）。R3/R4 的 fixture 从此只能用冻结类型。 | EventBus 类型是 R2/R3 共同进口 |
| **G2 C6 收编** ✅ 已过 | R1 全绿 | orbit 6.6 接线合并；**验证**：orbit **23** 测试全量回归（loop.rs 20 + agent_event_serde.rs 3，不改断言）+ `oi task add` → 流式 run → 事件流 → 持久化全链路（C1 验收）仍通；C6 勾满。全链路唯一载体 `m3_e2e.rs` 标 `#[ignore]`（需真实 omp 二进制），日常回归靠 23 测试。 | R1 与 R3 同碰 orbit，接缝限额共享 |
| **G3 插件回归闸** ✅ 已过（#346 + #354/#355 补验收） | R3 完成（C4 全绿） | compaction 切插件 + instruction 注入后；**验证**：① orbit **23** 测试仍绿（接缝未破坏循环）；② >120k 字符长会话压缩端到端不炸（checkpoint 快照 + 失败保原文）——**#355 `orbit/tests/compaction_e2e.rs` 4 测试**：触发 / 成对不变式 / 失败保原文 / 边界；③ AGENTS.md 注入——**#354 `orbit/tests/instruction_prompt.rs` 4 测试**：注入 / 无 AGENTS.md 保纯 TASK / digest 去重 / 静默降级；④ web 页打开一次真实 run，压缩不中断流式渲染（浏览器实测 2026-09-15，对话成功）。 | orbit 行为变更影响所有宿主 |
| **G4 事件流汇合** ✅ 已过（#353-#357 + 用户实测 2026-09-15） | R2 绿 **且** R4 的 5.1 DTO 完成 | R4 删 fixture 切实时 daemon；**验证**：① 3.4 e2e 绿（`daemon/tests/event_push.rs`，#353 修复编译后通过）；② web 聊天页流式 delta 实时追加——**用户浏览器实测确认逐字流式（2026-09-15）**；③ 断线重连不白屏（`client/tests/reconnect.rs` 3 测试）；④ 半开 run 标 `aborted`（`state/tests/run_status.rs` 8 测试）；⑤ web 全仓零 `read_event` 调用。 | **最大冲突点**：daemon protocol 改动权在 R2 |
| **G5 总装（不含 tag）** 🟡 开工中 | G1–G4 全过 | composition 真装配 + 删 mock；**验证**：① C1–C7 除 C7 tag 外全勾；② 全仓 `cargo test` 绿；③ `oi-web` 起在 8026，硬刷新后无 mock 残留（`curl localhost:8026 \| grep mock` = 0）。**tag 与 ferrite 接线暂缓**（2026-09-15 裁定）。 | 装配根只在 G5 集中改 |

**顺序铁律**：G1 之前任何 `register`/`provide` 代码不得合入；G4 之前 R4 不得合入依赖 daemon 新协议（3.3）的实际请求路径——读侧（现有 `session.*`/`run.list`）与既有 `worker.prompt` 调用不算 3.3 依赖，允许先行。

## 开新会话须知

- **起点**：`1b405f0`（harness 四 crate 可运行，agent loop 完整；阶段 0/1 见 git log）
- **harness trait 签名已冻结**：`Provider` / `Tool` / `PromptRenderer` / `RunState`（详见 `todo/dsh/BACKGROUND.md` §1 锚点）。改签名必须广播所有会话
- **认领路线**：读本文件「并发路线」表认领 R1–R5 一条，只做独占文件；小功能编号即任务编号（如 R2 = 做 2.2/2.3/2.4/3.1–3.4）
- **依赖铁律**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO
- **工作目录**：`.wt/<branch>`（git worktree add 必须在仓库根执行，防嵌套）
- **测试**：放同层 `tests/`，不在 src/ 写 `#[cfg(test)]`；重型测试推 CI
- **编译**：cargo 命令套 `cpulimit -l 70 -i --`，只跑 `-p <crate>`，禁止全仓 build

---

## G4 验收指南（用户实测）

> 五项验收中 ①②③④⑤ 的自动化载体已全部落地（见整合门表）。①③④⑤ 有自动化测试覆盖，**② 是唯一需要你浏览器实测确认的一项**——因为它断言的是页面级流式渲染的实时性，自动化测试只能覆盖订阅管线的数据正确性，覆盖不到「肉眼看见流式输出」。
>
> 你做完 ② 之后，我就能把 G4 标为已过并开 G5。

### 前置：起服务（约 3 分钟）

```bash
cd crates/web && npm install            # 确保 node_modules 在
cd <仓库根>
touch crates/web/assets/tailwind-input.css   # 强制重跑 build.rs
cpulimit -l 65 -i -- cargo build --bin oi-web
pkill -x oi-web; sleep 1
nohup ./target/debug/oi-web > /tmp/oi-web.log 2>&1 &   # 默认 8026
curl -s localhost:8026/ | grep -c -- --color-accent   # 应 > 0
```

起 daemon（另一终端）：

```bash
cpulimit -l 65 -i -- cargo build --bin daemon --bin oi
./target/debug/oi daemon start    # 自动找同目录的 daemon 二进制；已运行会提示
./target/debug/oi daemon status   # 确认在跑
```

浏览器**硬刷新** `http://localhost:8026`（Ctrl+Shift+R，LiveView 缓存 wasm）。

### ② 流式 delta 实测（核心，唯一需要你做的）

在聊天页发一条消息，观察回复：

- [ ] **逐字流式出现**，不是等几秒一次性整段出现
- [ ] 回复过程中「工作过程」折叠区实时计数（工具调用次数）
- [ ] 回复完整结束后，状态行显示 model / tokens 等结算信息
- [ ] 期间侧栏会话列表的状态点是 accent（运行中），结束后变 dim

**如果不流式**（一次性出现）：截图给我，我看 `/tmp/oi-web.log` 排查。

### ③ 顺带可做的断线重连（可选，自动化测试已覆盖）

网页开着的时候，另一终端 `pkill -x oi-daemon`（或 kill daemon 进程）：

- [ ] 页面**不白屏**，转圈或保留最后内容
- [ ] `oi daemon start` 重启后，不刷新页面，发新消息能继续收到回复

### ④ 半开 run（可选，自动化测试已覆盖）

发一条消息，**在回复流式到一半时 kill daemon**，然后刷新页面：

- [ ] 会话列表该会话标 `aborted`（danger 色状态点）而不是消失或显示正常
- [ ] 注：daemon 现行实现在 prompt 返回时即写 finish，干净的 UI 中断可能不留半开记录；**稳定复现方式是 kill daemon 后重启再刷新**（走 2.4 repair 语义）

### 验收完告诉我

- ② 过了 → 我关 G4，开 G5（composition 真装配 + 删 mock + tag）
- ② 没过 → 截图 + `/tmp/oi-web.log` 给我，我修

