# omenic PROGRESS（当前进度与规划）

> 已落地的背景见 [ROADMAP.md](ROADMAP.md)（已完成档）。本文只讲**还没做的**和**接下来做什么**——两份文档状态不同，ROADMAP 是已完成的沉淀，本文是未完成的规划。
>
> 编号体系沿用 ROADMAP：`C1–C8` / `R1–R7` / `G1–G8`。

## 当前位置（2026-09-19）

main 现为 `b20ebc0`（#383 B1/B2 合并后审查收尾 squash 合并，2026-09-19）；B1（#381，main `f514b91`）与 B2（#382）已合入，#383 补齐了两批的审查层与文档同步。

**G1–G8 全部已过，main 干净（`8d862e4`）。subagent 能力 seam Phase 1 已落地（#380，main `73493b5`，2026-09-17）：harness 容器现在有 `harness.subagents` 服务 + model-facing `subagent` 工具 + fork backend（复用 `subagent::runner`），daemon 生产路径可解析。** G6 之前的 6 条开放缺口**全部关闭**（1/2/3/5 由 G6，4/6 由 G7）：

- G6（#373/#374）把装配容器接进 daemon→web 生产路径，C4 的 AGENTS.md 注入与压缩第一次对真实用户生效，并补了真链路 e2e。
- G7（#376）关掉最后两条：**谱系分组**（5.3/5.5，数据模型 + 侧栏树）与 **per-run 事件归属**（协议帧 run_id + 订阅端过滤）。
- G8（#377/#378，2026-09-16/17）修三处「单测绿、生产失效」的缺陷（见下表），让三态生命周期在真实 orbit run 下可观测。**验收**：#377 gate merge --dry-run ALL PASS（119 checks）；PR CI 三连绿；合并后 main CI `35134561055` SUCCESS；真二进制 smoke 7/7；终审 ocr 34 条裁定 7 真阳性全部已修。#378（2026-09-17）以 `a91caee` 为 base 重走主控规范：codegraph 覆盖三条修复链、3 子代理 audit、CRG 0 affected flows、ocr 20 条全 pre-existing、23 个 G8 单测绿、gate 105 checks ALL PASS。
- C7（tag）前置条件全满足，**等用户拍板时机**。
- C8 已裁定不做。

**B1 批次已完成（#381，MCP 多传输 + 重连监督 + daemon 注入，2026-09-18，main 前基线 `c009f20`）：**

- **后端**：`StreamableHTTPClientTransport`（stdio + Streamable HTTP 双传输，dsh `mcp-client` 对照）；`McpServerConfig` 扩 `cwd`/`toolCallTimeoutMs`/`reconnect`/`failOnStartupError` 四字段；`Mcp::spawn()` 改 `McpClient::connect()` + 重连监督（指数退避 500ms 起步翻倍，`max_attempts` 封顶，超限熔断 unregister 工具）；MCP 服务注册进 daemon `orbit_setup` 容器（照 `harness.tools` resolve/fallback 模式）；task CLI 路径同步切新 API 不双轨。
- **前端**：`page-config` MCP 表单扩字段；`client/tests/config_roundtrip.rs` 补 `[mcp]` 段新键断言（G8-B 守门测试）；`.githooks/spec/settings.yaml` 契约加锚点（进 `SPEC_FILES`）；`chat.rs` 工具折叠卡验 MCP 工具 `tool_call_error` 帧形态（LIFO 配对现成管线照走）。
- **验收**：CI 全量绿；smoke 真实 Streamable HTTP MCP server 起服 + daemon 注册 + 模型侧 `mcp_*` 工具可调。
- **余量（不阻塞）**：dsh `mcp-client` 其余可选项（tool-level filter、server-level env 注入）未对齐。

**B2 批次已完成（#382，session resume + LLM waterfall + web settings 捆绑，2026-09-18，main 前基线 `f514b91`）：**

- **B2a session resume（纯后端，web 零改动）**：daemon 重启后 orbit worker 按 `session_id` 从 session DB 加载**最近 50 条** user/assistant 历史回放进 `ctx.messages`（`Worker::resume_session` + `OrbitEngine.resumed_session` 去重 + `dispatch WorkerPrompt` 分支 best-effort 接线）；session 切换时清空旧历史防跨 session 残留。
- **B2b LLM 路由**：`orbit::WaterfallLlm`（primary + `[[llm.fallbacks]]` 按序 waterfall，每个 provider 内沿用 `RetryPolicy` 指数退避；已泄 delta/toolcall 不切 provider、abort 即停、全败补发带序号 Error）+ `config::LlmFallbackConfig` + daemon 有 fallbacks 时切 `WaterfallLlm` backend、无则保持 `HttpLlm` 零变化；web 设置页「模型与渠道」新增 Fallback Provider 表单（`LlmFallbackForm` 四字段文本表单，按 model 匹配增量写 `[[llm.fallbacks]]`）+ `config_roundtrip` 断言 + `settings.yaml` 2 契约锚点。
- **验收**：CI 全量绿；CRG `detect-changes --base f514b91` 0 affected flow；code-reviewer 逐文件审查（1 med 已修：跨 session 残留）；smoke 真实 daemon 双场景（重启 resume 历史进 LLM 请求体 + 主 provider 500 切 fallback 成功服务）通过。
- **余量（不阻塞）**：resume 回放跳过 `System`/`Tool` 行；全败 Error 不聚合各 provider 原始错误文本；statusline fallback 切换 `model` 显示口径保持 `config.model` 不变。
- **对照**：B1/B2 的验收口径一致——CI 全量 + CRG + smoke 真实 daemon。

**#383 合并后审查收尾已完成（2026-09-19，main `b20ebc0`）：**

- **三层审查补齐**：B1/B2 合并时只走了 CRG + code-reviewer，ocr 层缺失——#383 补跑了 ocr（36 文件，首跑 17 条 + `--resume` 补跑，共 37 条：10 HIGH / 18 MED / 9 LOW）。
- **真阳性 3 条已修**：`ae618a0`（fallback 成功后仍以 `TurnEnd{Error}` 结束、`tool_calls` 被丢弃 —— 改用终态 + `leaked_content` 判据）、`2149035`（config 在 merge 边界统一 trim name/command/url + 空转 fallback 告警）、`6661a70`（`write_full_config` 漏写 `[[mcp.servers]]` 导致首次保存丢整张表）、`862472a`（mcp 重连注释范围点明只针对 `Timeout` 变体）。
- **驳回 4 条（有裁定理由）**：page-config 跨 tab 挂载快照（设计意图，改动要动 Dioxus 状态提升）、worker.rs 持锁回放（临界区仅 ≤50 条内存 push、无 I/O）、llm.rs「只有 name 的 server 能保存」（误报，基线已有 `no_transport` 校验）、page-config 本地 signal 不随 prop 更新（真，tech-debt 记档）。
- **验收**：CI 全量绿（run 35425569919，1m35s）；`gate merge --dry-run` 118 checks ALL PASS；审查记录三条（round 1 / round 2 / CRG Review）落在 PR conversation。

**G8 之后没有排队中的整合点。** 接下来走 dsh 全量对照 backlog：2026-09-18 已划分三批（B1 MCP 捆绑 web 契约 / B2 session resume + LLM 路由 / B3 jobs + subagent P4），附件与遥测类不排批次（见「后续 backlog」）。B1（#381）/ B2（#382）/ 审查收尾（#383）已合入 main（`b20ebc0`），见上文「当前位置」；**B3 未开工，是下一步。**

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

## P0 #1+#2 插件生命周期（#379，已合并）

- **#1** per-plugin Config schema + inject 依赖门控：`DshPlugin::validate_config` 默认方法 + `PluginError::InvalidConfig` + `PluginRegistry::register_with_config` + `do_register`；schema 校验失败在 `plugin.register` 之前拦截，不污染 context。
- **#2** 单插件卸载：`PluginRegistry::unregister(name)` + `Fiber::unload_named(name)` + `PluginLifecycle::name()` 默认实现；卸载后 `on_unload` 自动跑，剩余插件不受影响。
- **测试**：`plugin_test.rs` 新增 8 例（invalid config rejection / valid config pass / unregister + unload named / re-register / full LIFO）；`assemble.rs` 补充 `InvalidConfig` 穷尽匹配。
- **CRI/CI/gate**：CRG 0 affected flows；CI test job PASS；`gate merge --dry-run` PASS（自定义 checklist 通过）。
- **合并**：2026-09-17 `8d862e4` squash merge，远程分支已删。

## 后续 backlog

> 2026-09-18 批次划分（本会话上限三批）：B1 MCP 后端 + web 契约捆绑 / B2 session resume + LLM 路由 / B3 jobs + subagent P4。附件全链路（原中优先）与 telemetry / title-llm / credentials 移入「不排批次」——附件 web 前端与后端三段耦合重，超出三批上限。

### 批次 B1：MCP 多传输 + 重连监督 + daemon 注入（捆绑 web 契约，2–3 PR）（已完成，见上文「当前位置」）

后端：新增 `StreamableHTTPClientTransport`；`McpServerConfig` 扩 `cwd`/`toolCallTimeoutMs`/`reconnect`/`failOnStartupError` 四字段；`Mcp::spawn()` 改 `McpClient::connect()` + 重连监督（指数退避 500ms 起步翻倍，`max_attempts` 封顶，超限熔断 unregister 工具，dsh `packages/mcp/mcp-client/src/connection.ts` 可对照）；MCP 服务注册进 `daemon::orbit_setup`（照 `harness.tools` 的 resolve/fallback 模式）；task CLI 路径同步切新 API 不双轨。

Web 必须同批（否则 TOML 能改、UI 看不到、工具卡渲染不出来）：`page-config` MCP 表单扩字段；`client/tests/config_roundtrip.rs` 补 `[mcp]` 段新键断言（G8-B 守门测试）；`.githooks/spec/settings.yaml` 加锚点（进 `SPEC_FILES`）；`chat.rs` 工具折叠卡验 MCP 工具的 `tool_call_error` 帧形态（LIFO 配对现成管线照走）。

PR 拆分：PR1 后端（transport + 重连 + 熔断 + 配置扩展）；PR2 daemon 注入 + task CLI 同步；PR3 web 表单 + 契约 + 聊天工具卡。

### 批次 B2：session resume（纯后端，1–2 PR）+ LLM provider 注册表 + retry（捆绑 settings，2 PR）（已完成，见上文「当前位置」）

**B2a session resume（纯后端，web 零改动）**：worker 启动按 session id 调 `load_turn_log` → 解 turn codec → 重建 `ctx.messages`，daemon 重启后不再只剩最新一条消息；checkpoint flush 策略（dsh `session-checkpoint-policy`）本批可省，余量记「余量」节。

**B2b LLM 路由（捆绑 web settings）**：`LlmRuntime` 抽象（`registerAdapter`/`stream` waterfall，dsh `packages/llm/llm/src/index.ts:311`）+ 第二 provider adapter（DeepSeek/PiAi 二选一或双接）+ `llm-retry` 指数退避路由；token-meter 明确不做（C8 裁定维持）。Web 面：`page-config` LLM 区 provider 路由 UI（双 provider 选择 + fallback 顺序）+ `settings.yaml` 契约加锚点 + statusline 在 fallback 切 provider 时 `model` 显示口径要定（口径定在文档里）。

PR 拆分：B2a 独立 1 个 PR（纯后端）；B2b 拆 2 个——registry + retry 后端 / settings 表单 + 契约。

### 批次 B3：jobs + terminal（Rust 侧）+ subagent Phase 4（1–2 PR）

- **jobs + terminal 持久 PTY**（dsh `packages/jobs/jobs*` + `terminal/terminal*` 可对照）：`JobRegistry` trait（`start`/`list`/`kill`/`wait`/`onJobDone`）+ `LocalJobRegistry` 内存实现；PTY 后端 bash 起步（pwsh 可选）；6 个模型工具中先做 `jobs_wait`/`jobs_kill` 两把最刚需的，其余（`jobs_list` 等）可余量。解决长命令 30s 超时被杀、模型无跨 tool call 交互 shell。
- **subagent Phase 4 余量**（消费 #380 预留的 `OrbitSetup.providers` seam）：`interrupt_subagent` 实现 + 一个出进程后端起步（ACP 优先，dsh `packages/subagent/subagent-acp` 可对照）；`send_message`/`report`、continuable/background run、session-seeding 本批可省记「余量」。
- **Web 面小改动**（顺带带上，不独立 PR）：`chat.rs` 工具折叠卡加 `jobs_wait`/`jobs_kill` 工具名进词表（否则 LIFO 当未知折叠掉）；`subagent_report` 新帧进词表；侧栏 `group_sessions` 已有树不用动。

PR 拆分：PR1 jobs + terminal + web 工具词表；PR2 subagent P4（ACP + interrupt，出进程后端是跨系统协议验证，单独一个 PR 便于隔离）。

### 不排批次（2026-09-18 裁定）

| 项 | 理由 |
|---|---|
| **附件全链路** | 前端 composer 上传入口 + 内容寻址存储 + 上下文投影三段全缺，web 前端与后端耦合重，超出三批上限；单做后端无意义。等三批过完再独立排期 |
| **session telemetry / OTel** | 无生产需求，低优先（ROADMAP 已标「低」） |
| **session-title / title-llm** | 同上 |
| **credentials / authorization / identity** | 同上；现有配置走 TOML 文件，够用 |
| 元工具与治理整片（goal/todo/plan-mode/schedule/skill/lsp/hooks/guard/feedback/workflow） | 范围外，独立规划 |
| C7（tag + ferrite 接线） | 等用户拍板时机，不占批次 |

### 中期（composition-root + 新 crate，2026-09-18 批次划分见 B1–B3 节）

| 域 | dsh 现状 | omenic 现状 | 缺口（批次） |
|---|---|---|---|
| **subagent 能力 seam** | `SubagentRuntime` 服务（provider registry + one-shot/continuable + 生命周期事件）+ 11 子包（spawn/fork 进程内 + ACP/Codex/Claude Code/SDK 四个进程外后端 + control/report 工具） | **Phase 1 已落地（#380）**：新增 crate `omenic-harness-subagent`（`SubagentProvider` trait / `SubagentRuntimeService` / `ForkProvider` in-process backend 复用 `subagent::runner` / model-facing `subagent` + `subagent_control`（list only）工具）；`ToolCatalog` 内建可变；composition 注册三个插件；daemon `orbit_setup` 注册 fork provider（只读工具子集）；e2e smoke 通过 | **B3**：interrupt + ACP 出进程后端；余量：`send_message`/`report`、continuable/background run + 生命周期事件、session-seeding |
| **MCP 多传输 + 重连** | `mcp-client` Cordis 插件：stdio + Streamable HTTP 双传输，`RECONNECT_DEFAULTS` 重连监督，`failOnStartupError` 启动失败即熔断，per-tool call timeout，`cwd` 每服务 | **已完成（#381，2026-09-18；#383 补审 2026-09-19）**：stdio + Streamable HTTP 双传输、重连监督（指数退避 + 重试上限熔断）、per-server timeout/cwd、`fail_on_startup_error`、daemon `orbit_setup` 注入；`write_full_config` 丢 `[[mcp.servers]]` 已在 #383 修（`6661a70`） | 余量：dsh `mcp-client` 其余可选项（tool-level filter、server-level env 注入）未对齐 |
| **session resume 生产路径** | `SessionPersistence` 抽象（`prepare`/`load`/`inspect`/`readFrom`）+ JSONL/SQLite 双后端 + `session-checkpoint-policy` 在 llm/tools/pre-step 前自动 flush durable log | **已完成（#382 B2a，2026-09-18）**：daemon 重启后 orbit worker 按 `session_id` 从 session DB 加载最近 50 条 user/assistant 历史回放进 `ctx.messages`（dedupe + 切换清 ctx） | 余量：checkpoint flush 策略（dsh `session-checkpoint-policy`）；回放跳过 `System`/`Tool` 行 |
| **附件全链路** | `AttachmentStore` 抽象（`validateImage`/`saveImage`/`readImage`/`readImageRequest`）+ `LocalAttachmentStore` 内容寻址 + `ui-attachment` 前端 + adapter `resolveAttachments` 注入 | omenic 全仓 grep `attachment`/`image` = 0 命中（仅 `ETXTBSY` 误匹配） | **不排批次**（超三批上限；三段全缺，见「不排批次」节） |
| **LLM provider 注册表/路由 + retry** | `LlmRuntime` 服务（`registerAdapter`/`registerConfigurableProviders`/`stream` waterfall）+ DeepSeek/PiAi 双 adapter + `llm-retry` 插件（provider 路由指数退避）；`TokenMeter` | **waterfall 已完成（#382 B2b，2026-09-18；#383 补审 2026-09-19）**：`orbit::WaterfallLlm`（`[[llm.fallbacks]]` 按序切换 + per-provider `RetryPolicy` 退避 + 中间 Error 判据改用终态/`leaked_content`）+ web settings Fallback 表单 | token-meter 不做（C8 裁定维持）；DeepSeek/PiAi 官方 adapter 未接（现走 OpenAI 兼容端点） |
| **jobs 后台作业 + terminal 持久 PTY** | `JobRegistry` 抽象（`start`/`list`/`kill`/`wait`/`onJobDone`）+ `LocalJobRegistry` 内存实现 + `terminal` PTY 后端（bash/pwsh）+ 6 个模型工具 | 无 `jobs`/`terminal`/`pty` crate；`Cargo.toml` 无相关依赖；grep `pty` 仅误匹配 `subagent`/`opportunity` | **B3**：trait + 内存实现 + bash PTY；模型工具先 `jobs_wait`/`jobs_kill`，余量其余 4 个 + web 工具词表 |
| **session telemetry/otel + title-llm** | `SessionTelemetryBackend` 抽象 + OTel SDK 导出 + `SessionTitleService`（确定性 fallback + LLM 生成） | omenic 全仓 grep `session_telemetry`/`opentelemetry`/`session-title`/`title-llm` = 0 | **不排批次**（无生产需求） |
| **credentials/authorization + identity** | `CredentialProvider` 抽象（分层 env 解析 + YAML 持久化 + 跨进程锁）+ `AuthorizationService`（one-attempt-per-key）+ `AnonymousUserId` | omenic 全仓 grep `credentials`/`identity`/`anonymous-user-id` = 0 | **不排批次**（现有 TOML 文件配置够用） |

### 范围外（维持现状）

| 项 | 原因 |
|---|---|
| 元工具与治理整片（goal/todo/plan-mode/schedule/skill/lsp/hooks/guard/feedback/workflow，约 60 子包） | 复刻差距最大的功能域，不与 C1–C8 主线耦合，独立规划 |
| core scope + agent-tool-presentation | dsh 作用域隔离与工具结果呈现策略，不与主线耦合 |
| acp 协议 + boot/bundle 声明式装配 | omenic `composition` 已对齐 cordis 运行时；声明式层（profile/bundle）留给 C7 或独立路线 |
| C7（tag omenic-harness-v0.1.0 + ferrite 接线） | 前置全满足，等用户拍板时机 |
| C8 interaction + token-meter | 已裁定不做 |

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
