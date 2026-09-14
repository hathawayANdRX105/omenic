# omenic ROADMAP

> 长期重构方向 + 并发分工。历史进度见 git log；`todo/dsh/README.md` 为设计蓝图，`todo/dsh/BACKGROUND.md` 为现状锚点（冻结签名 / `AgentEvent` 契约）——todo/ 已 gitignore，仅本地保留。
> 最后更新：2026-09-14（R1/R2/R3 已合并：PR #339/#343/#346；web 侧 #340/#341）

## 重构终点（先写死，agent 据此找缺口并更新路线）

**完成 = 以下能力全部可演示。** 格式：能力 → 小功能 → 文件（omenic 目标 | dsh 参考，dsh 路径相对 `~/projects/harness/deepseek-harness`，已核实）。
小功能行内 ✅ = 已合并 main；⚠️ = 部分完成；⬜ = 未开工。

### C1 agent 循环 ✅ 已有

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 1.1 ✅ 循环 + 5 不变式（工具成对/截断不执行/中止不孤儿/压缩失败保原文/max_turns） | `crates/agent/orbit/src/lib.rs`（`run_agent_streaming`；`tests/loop.rs` 20 集成测试，断言零改动） | `packages/core/agent-loop/src/{agent.ts,tool-calls.ts,invariant.ts}` |
| 1.2 ✅ 流式 LLM（OpenAI 兼容 + SSE） | `crates/agent/adaptor/src/{openai.rs,sse.rs}` | `packages/llm/llm-pi-ai/src/` |
| 1.3 ✅ 10 内置工具 + `Guarded` 策略包装 | `crates/agent/tools/src/`（10 文件 + `builtin_tools`） | `packages/core/tools/src/` + `packages/fs/` |
| 1.4 ✅ 压缩已抽插件（120k 字符策略平移至 C4 crate，orbit 只留接缝） | `crates/harness/compaction/src/policy.rs` | — |

### C2 会话持久化 + crash-repair ✅（PR #343）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 2.1 ✅ 会话 CRUD + libSQL 持久化 | `crates/infra/session/src/lib.rs:SessionDb` | `packages/core/session/src/index.ts` |
| 2.2 ✅ 事件词汇表（`WorkerEvent` 拆出 ToolExecutionStart/End/Error 专属变体，Unknown 只留给未识别帧；轮询模式语义不变） | `crates/infra/rpc/src/worker.rs` | `packages/core/session/src/{known-event-types.ts,types.ts}` |
| 2.3 ✅ 无损分块编解码（turn log JSONL 往返，malformed 行拒绝整log） | `crates/infra/session/src/lib.rs`（`TurnRecord` + encode/decode_turn_log） | `packages/core/session/src/{chunk-rows.ts,json.ts}` |
| 2.4 ✅ crash-repair（半程 run 补 Aborted closer，空/完整日志幂等 no-op） | `crates/infra/session/src/lib.rs:768`（`interrupted_run_closers`，6 测试） | `packages/core/session/src/{repair.ts,request-header.ts}` |

### C3 事件流可推 web ✅（PR #343）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 3.1 ✅ `AgentEvent` serde DTO 定稿（tag="type" snake_case，round-trip 3 测试） | `crates/agent/orbit/src/lib.rs:AgentEvent` + `tests/agent_event_serde.rs` | `packages/core/session/src/types.ts` |
| 3.2 ✅ `Worker::subscribe(topic)` 推形态（std::sync::mpsc 泵线程，20ms 轮询转发，死接收器就地移除，命令按 id 匹配；轮询模式保留） | `crates/infra/rpc/src/worker.rs` + `tests/subscribe_pump.rs` | `packages/client/runtime/src/client/sessions/{notifier.ts,service.ts}` |
| 3.3 ✅ daemon `event.subscribe`/`event.unsubscribe` 命令 + EventFrame 推送（纯加法，既有命令语义零变化；server 连接拆 reader/writer，per-conn inbox 串行写帧；死订阅者就地清理） | `crates/infra/daemon/src/{protocol.rs,state.rs,server.rs,dispatch.rs,client.rs}` | `packages/sdk/server/src/server.ts` + `packages/api/gateway/src/` |
| 3.4 ✅ e2e：真实 daemon + mock omp，双订阅者收完整事件序列，断开其一不影响另一个，unsubscribe 精确移除 | `crates/infra/daemon/tests/event_push.rs` | `packages/core/session/src/invariant.ts` |

### C4 compaction 插件 + 指令注入 ✅（PR #346）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 4.1 ✅ `CompactionPolicy` + 120k 字符默认策略平移（cut 算法/kept 守卫，9 预算测试） | `crates/harness/compaction/src/policy.rs` | `packages/compaction/compaction/src/{index.ts,checkpoint.ts}` |
| 4.2 ✅ tool_call/tool_result 成对裁剪平衡谓词（前缀 call 集 ∩ 后缀 result 集 = ∅，3 组对 × 全预算孤儿检查） | 同上 `pairing.rs` | `packages/compaction/compaction/src/tool-pairing.ts` |
| 4.3 ✅ 预算分区 | 同上 `region.rs` | `packages/compaction/compaction-basic/src/{region.ts,types.ts,config.ts}` |
| 4.4 ✅ 摘要钩子（Summarizer trait，默认 no-op；orbit 侧 LlmSummarizer 桥接真后端） | 同上 `summarize.rs` | `packages/compaction/compaction-basic/src/summarizer.ts` |
| 4.5 ✅ orbit 接缝替换（策略逻辑零残留，只留 DTO 重铸 + 桥；20 测试断言零改动全绿） | `crates/agent/orbit/src/lib.rs`（接缝段） | — |
| 4.6 ✅ AGENTS.md 向上逐级发现 + mtime 缓存（3 测试） | `crates/harness/instruction/src/files.rs` | `packages/context/agent-instructions/src/{files.ts,state.ts}` |
| 4.7 ✅ digest 去重 + 渲染 `PromptTemplate`（SipHash，不持久化；4 测试） | 同上 `render.rs` | `packages/context/agent-instructions/src/{render.ts,digest.ts,config.ts}` |
| — | 附带：`harness-core` 新增 `chat.rs` DTO 镜像（与 adaptor::Message serde 字节等价，绕开 harness→agent 禁令） | — |

### C5 web 页面跑真数据 🟡 5.1/5.2a ✅（#340/#341），读侧可推进，事件侧 5.2b 已解锁（3.3 落地）

> **整合注意（功能，R4 会话必读）**：① 写路径归 daemon——web 经 `session.append`/`worker.prompt` 调用，不直接写 sessions.db（避免双写 libSQL，2.4 repair 语义依赖 daemon 侧写）；② `worker.read_event` 是消费式出队，web 禁止轮询（会抢 CLI/task/memory 事件）——推送用 `event.subscribe`；③ web 的 `AgentEvent` fixture 已可换 orbit 冻结 DTO（转译签名不变）；④ `DaemonClient` 同步短连接 × LiveView async → `spawn_blocking`；⑤ oi-web 与 daemon 必须同指一个 data_dir（worktree 各有 `.oi`），设置页加 daemon ping 校验；⑥ 谱系（5.5）无服务端命令，先平铺、G4 后按 `run.list` 组装。

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 5.1 ✅ `AgentEvent` DTO + 转译层（mock 流已消费，4 测试） | `crates/web/state/` | `packages/core/session/src/surface.ts` |
| 5.2a ✅ daemon RPC 读客户端（会话列表/历史/search，`DaemonClient::subscribe`/`event_unsubscribe` 亦已就位） | `crates/web/client/`（`daemon.rs`） | `packages/client/runtime/src/client/sessions/{manager.ts,remotes.ts}` |
| 5.2b ⬜ `event.subscribe` 消费端：订阅接收端替换 `mock::stream_reply`，断线重连，页面零改动 | `crates/web/client/`（`daemon.rs` 追加） | `packages/client/runtime/src/client/sessions/{notifier.ts,service.ts}` |
| 5.3 ⬜ 会话列表/历史 ← 真数据（替换 `mock_sessions`） | `crates/web/page-workspace/` | `packages/client/runtime/src/client/sessions/{session.ts,lineage.ts}` |
| 5.4 ⬜ 聊天流式：delta 追加 + tool 折叠卡 | `crates/web/components/`（chat.rs） | `client/conversation/{event-registry.ts,view-registry.ts}` + `sessions/tool-call-tree.ts` |
| 5.5 ⬜ sidebar 真会话 + 谱系分组 | `crates/web/components/`（sidebar.rs） | `packages/client/runtime/src/client/sessions/lineage.ts` |
| 5.6 ⬜ statusline 真运行态 + 计时 | `crates/web/components/`（statusline.rs） | `packages/client/runtime/src/client/sessions/assistant-timing.ts` |
| 5.7 ⬜ stats 接 token 真数据（无则隐藏该卡；完整需 C8.3） | `crates/web/page-stats/` | `packages/llm/token-meter/src/{usage-projection.ts,projection.ts}` |
| 5.8 ⬜ 配置页读写 `infra/config`（TOML 往返） | `crates/web/page-config/` | `packages/settings/settings-file/src/index.ts` |
| 5.9 ⬜ `mock.rs` 删除（全仓 grep `mock_sessions\|mock_messages` = 0） | `crates/web/client/src/mock.rs`（过渡存放） | — |
| 5.10 ⬜ ui-validate 验收（`specs/ui/*.yaml` + 浏览器实测，序列见仓库 AGENTS.md） | `bin/web/tests/` + `specs/ui/` | — |

### C6 插件面四件套 ✅（PR #339）

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 6.1 ✅ 服务注册容器（TypeId 静态注册，误型/误 key 返 None 不 panic） | `crates/harness/plugin/src/context.rs` | `vendor/cordis/src/{context.ts,service.ts}` |
| 6.2 ✅ 事件总线（同步顺序派发 + Subscription token 退订） | 同上 `events.rs` | `vendor/cordis/src/events.ts` |
| 6.3 ✅ 插件生命周期（on_load/on_unload，Drop 逆序 LIFO，手动 unload 后 Drop 幂等） | 同上 `fiber.rs` | `vendor/cordis/src/fiber.ts` |
| 6.4 ✅ 插件注册表（重名在 register 执行前拒绝；`plugins()` 列举） | 同上 `registry.rs` | `vendor/cordis/src/registry.ts` |
| 6.5 ✅ 组装根（`assemble(config, plugins)` 按序注册 harness 家族 + orbit 薄层 + 宿主插件） | `crates/composition/src/lib.rs` | `packages/bundle/base/src/index.ts` |
| 6.6 ✅ orbit `register(ctx)` 薄层（12 行，循环逻辑零改动） | `crates/agent/orbit/src/lib.rs` 末段 | `packages/core/agent-loop/src/index.ts` |

### C7 tag + ferrite 接线 ⬜（等 C5 完成 → G5）

| 小功能 | 文件 | 说明 |
|---|---|---|
| 7.1 ⬜ C1–C6 全绿 + 全仓 `cargo test` | `crates/composition/` 统一装配 | G5 触发（C1–C4/C6 已绿，只差 C5） |
| 7.2 ⬜ `tag omenic-harness-v0.1.0` | — | ferrite 侧 git 依赖按 tag 锁定 |
| 7.3 ⬜ ferrite 根 `Cargo.toml` 删本地 harness member，加 tag git 依赖；本地 `[patch]` → 阶段 4 后删 | ferrite 仓库 | 对照旧「阶段 3」 |

### C8（可选，酒馆触发）：interaction + token-meter + llm-retry

| 小功能 | omenic 文件 | dsh 参考 |
|---|---|---|
| 8.1 ⬜ ask-user / user-questions trait（web 消费） | `crates/harness/interaction/src/`（新） | `packages/interaction/{user-questions,tool-ask-user}/src/` |
| 8.2 ⬜ approval 策略 + 权限预设 | 同上（`approval.rs`） | `packages/interaction/{user-approval,permission-presets}/src/` |
| 8.3 ⬜ token-meter 投影（C5.7 stats 完整数据源） | `crates/harness/metering/src/`（新） | `packages/llm/token-meter/src/{usage-projection.ts,projection.ts,client.ts}` |
| 8.4 ⬜ llm-retry（流式失败重试） | `crates/agent/adaptor/src/retry.rs`（追加） | `packages/llm/llm-retry/src/{history.ts,types.ts}` |

**明确不做**（dsh 有但 omenic 不复刻）：`hooks`（外部 agent hook 协议）、`skill`（SKILL.md 加载）、`guard`（timeout-policy/repeat-reminder）、`lsp`、`sandbox`、`subprocess`、`e2b`、react `ui-*`（40+ 包）。

**独有功能保护区**（omenic 独有，dsh 无对应物——任何路线**只读/单向依赖**，不许重构、不许当缺口往里塞）：
- `crates/infra/memory`（jcode：`embed/graph/recall/inject/pipeline`）— 记忆层，C1 已有
- `crates/agent/task`（`runner/graph/store/template` RPC 任务模型）— C1 已有；task/runner.rs 仅允许 WorkerEvent match 消费方适配
- `crates/agent/subagent`、`crates/agent/mcp` — C1 已有
- `crates/evidence/spec` + `bin/gate`（合规工具；远期归宿 = 注册成插件面工具插件，现在不动）— 只读/单向依赖
- `daemon/protocol.rs` 只加命令不改既有语义（已守住：R2 diff 零删除行）

## 并发路线（R1–R3 已完成；R4 进行中）

| 路线 | 覆盖小功能 | 状态 |
|---|---|---|
| **R1 插件面** | C6（6.1–6.6） | ✅ PR #339（`b9d0928`，审查记录在 PR 正文） |
| **R2 事件流+修复** | C2（2.2–2.4）+ C3（3.1–3.4） | ✅ PR #343（`0be85de`） |
| **R3 核心插件** | C4（4.1–4.7） | ✅ PR #346（`3e5c668`） |
| **R4 web** | C5（5.1–5.10） | 🟡 5.1 ✅ #340、5.2a ✅ #341（zcode 会话）；**5.2b 已解锁**（3.3 落地，`DaemonClient::subscribe` 可用）→ 5.3–5.9 按序推进 |
| **R5（占位，酒馆触发）** | C8（8.1–8.4） | ⬜ 不在 omenic 范围 |

> R4 独占 `crates/web/{client,state,components,page-*}/` + `bin/web/`（入口 crate：App/launch/build.rs/tailwind）。所有契约（`AgentEvent` serde、daemon protocol、harness trait、插件面 API）已冻结，按 main 消费。

## 整合门状态

| 门 | 状态 | 说明 |
|---|---|---|
| **G1 契约冻结** | ✅ 过 | 插件面 trait + `AgentEvent` serde 已广播（3.1 round-trip 测试绿） |
| **G2 C6 收编** | ✅ 过 | orbit 20 测试全量回归（断言零改动）+ 六 crate 16 项 test result 全 ok；`oi task add` 全链路冒烟留真实 key 场景（G5 补） |
| **G3 插件回归闸** | 🟡 ①✅ | ① orbit 20 测试绿 ✅；② >120k 长会话压缩端到端 ③ AGENTS.md 注入 system prompt ④ web 真实 run 压缩不中断——三项需真实 LLM key + R4 联调时验 |
| **G4 事件流汇合** | 🟡 前置就绪 | R2 绿 ✅ + web 5.1 DTO ✅；等 R4 切实时数据后验证：流式非轮询 / 断线重连不白屏 / 半开 run 标 aborted / `worker.prompt` 事件互不抢占 |
| **G5 总装/tag** | ⬜ | 等 C5 完：全仓 `cargo test` + `curl 8026 \| grep mock` = 0 + tag + ferrite 编译 |

**顺序铁律（存量）**：daemon 既有命令语义零变化（已守住）；web 禁止 `worker.read_event` 轮询（消费式出队，走 `event.subscribe` 推送）。

## 开新会话须知

- **起点**：main（含 R1/R2/R3 + web 5.1/5.2a；见 git log `3e5c668` 起）
- **当前唯一活跃 lane = R4（web 去 mock）**：从 5.2b 开始（订阅消费端替换 mock 流），按 5.3→5.4→5.5→5.6→5.7→5.8→5.9→5.10 推进；读侧（5.3/5.8）与推送侧（5.2b/5.4/5.6）可并行
- **契约（已冻结，按 main 消费）**：`AgentEvent` serde（orbit）/ daemon protocol（含 `event.subscribe`）/ harness trait（Provider/Tool/PromptRenderer/RunState）/ 插件面 API（DshPlugin/PluginContext/Fiber）。改任何契约必须先提 issue 广播
- **依赖铁律**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO
- **工作目录**：`.wt/<branch>`（仓库根执行 git worktree add）；提交用 `git commit -F`（COMMIT_EDITMSG 被并发会话污染会让 gate CM-01 误报）
- **测试**：放同层 `tests/`，不在 src/ 写 `#[cfg(test)]`
- **编译**：`cpulimit -l 70 -i -- cargo …`，只 `-p <crate>`，禁止全仓 build
