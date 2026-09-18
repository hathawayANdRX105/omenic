# omenic PROGRESS（当前进度与规划）

> 已落地的背景见 [ROADMAP.md](ROADMAP.md)（已完成档）。本文只讲**还没做的**和**接下来做什么**——两份文档状态不同，ROADMAP 是已完成的沉淀，本文是未完成的规划。
>
> 编号体系沿用 ROADMAP：`C1–C8` / `R1–R7` / `G1–G8`。

## 当前位置（2026-09-17）

**G1–G8 全部已过，main 干净（`8d862e4`）。subagent 能力 seam Phase 1 已落地（#380，main `73493b5`，2026-09-17）：harness 容器现在有 `harness.subagents` 服务 + model-facing `subagent` 工具 + fork backend（复用 `subagent::runner`），daemon 生产路径可解析。** G6 之前的 6 条开放缺口**全部关闭**（1/2/3/5 由 G6，4/6 由 G7）：

- G6（#373/#374）把装配容器接进 daemon→web 生产路径，C4 的 AGENTS.md 注入与压缩第一次对真实用户生效，并补了真链路 e2e。
- G7（#376）关掉最后两条：**谱系分组**（5.3/5.5，数据模型 + 侧栏树）与 **per-run 事件归属**（协议帧 run_id + 订阅端过滤）。
- G8（#377/#378，2026-09-16/17）修三处「单测绿、生产失效」的缺陷（见下表），让三态生命周期在真实 orbit run 下可观测。**验收**：#377 gate merge --dry-run ALL PASS（119 checks）；PR CI 三连绿；合并后 main CI `35134561055` SUCCESS；真二进制 smoke 7/7；终审 ocr 34 条裁定 7 真阳性全部已修。#378（2026-09-17）以 `a91caee` 为 base 重走主控规范：codegraph 覆盖三条修复链、3 子代理 audit、CRG 0 affected flows、ocr 20 条全 pre-existing、23 个 G8 单测绿、gate 105 checks ALL PASS。
- C7（tag）前置条件全满足，**等用户拍板时机**。
- C8 已裁定不做。

**B2 批次已完成（#382，session resume + LLM waterfall + web settings 捆绑，2026-09-18，main 前基线 `f514b91`）：**

- **B2a session resume（纯后端，web 零改动）**：daemon 重启后 orbit worker 按 `session_id` 从 session DB 加载**最近 50 条** user/assistant 历史回放进 `ctx.messages`（`Worker::resume_session` + `OrbitEngine.resumed_session` 去重 + `dispatch WorkerPrompt` 分支 best-effort 接线）；session 切换时清空旧历史防跨 session 残留。
- **B2b LLM 路由**：`orbit::WaterfallLlm`（primary + `[[llm.fallbacks]]` 按序 waterfall，每个 provider 内沿用 `RetryPolicy` 指数退避；已泄 delta/toolcall 不切 provider、abort 即停、全败补发带序号 Error）+ `config::LlmFallbackConfig` + daemon 有 fallbacks 时切 `WaterfallLlm` backend、无则保持 `HttpLlm` 零变化；web 设置页「模型与渠道」新增 Fallback Provider 表单（`LlmFallbackForm` 四字段文本表单，按 model 匹配增量写 `[[llm.fallbacks]]`）+ `config_roundtrip` 断言 + `settings.yaml` 2 契约锚点。
- **验收**：CI 全量绿；CRG `detect-changes --base f514b91` 0 affected flow；code-reviewer 逐文件审查（1 med 已修：跨 session 残留）；smoke 真实 daemon 双场景（重启 resume 历史进 LLM 请求体 + 主 provider 500 切 fallback 成功服务）通过。
- **余量（不阻塞）**：resume 回放跳过 `System`/`Tool` 行；全败 Error 不聚合各 provider 原始错误文本；statusline fallback 切换 `model` 显示口径保持 `config.model` 不变。

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

### 近期（同一 trait / registry / fiber 合并做）

| 优先 | 项 | 说明 |
|---|---|---|
| 1 | ✅ **插件 per-plugin Config schema + inject 依赖门控**（#379） | `DshPlugin` trait 现在只有 `name()` + `register()`（`registry.rs:14-19`）。dsh cordis `Plugin.Base` 有 `Config?: StandardSchemaV1`，`resolveConfig()` 在插件启动前跑标准 schema 校验，失败直接 `ValidationError` 拒注册（`fiber.ts:50-62`）。omenic 要补：① trait 加关联类型或 serde 结构；② registry 校验失败返回 `PluginError::InvalidConfig` 而非 panic；③ 主文档切插件子文档能力。两项改同一组 trait + registry + fiber，合并做省一半。 |
| 2 | ✅ **插件反注册（单插件卸载）**（#379） | `Fiber::unload()`（`fiber.rs:82-88`）只能逆序拆全部；dsh 每个插件有独立 `Fiber` 实例，`dispose()` 只回收该插件，`RegistryService.delete(plugin)` 定向移除。omenic 要补：① `Fiber::unload(name)` 只移除指定插件并跑 `on_unload`；② `EventBus::unsubscribe` 已有（`events.rs`）但未暴露给 `PluginLifecycle`；③ `Fiber::resolve` 卸载后返回 `None`，避免悬垂句柄。 |

### 中期（composition-root + 新 crate）

| 域 | dsh 现状 | omenic 现状 | 缺口 |
|---|---|---|---|
| **subagent 能力 seam** | `SubagentRuntime` 服务（provider registry + one-shot/continuable + 生命周期事件）+ 11 子包（spawn/fork 进程内 + ACP/Codex/Claude Code/SDK 四个进程外后端 + control/report 工具） | **Phase 1 已落地（#380）**：新增 crate `omenic-harness-subagent`（`SubagentProvider` trait / `SubagentRuntimeService` / `ForkProvider` in-process backend 复用 `subagent::runner` / model-facing `subagent` + `subagent_control`（list only）工具）；`ToolCatalog` 内建可变；composition 注册三个插件；daemon `orbit_setup` 注册 fork provider（只读工具子集）；e2e smoke 通过 | Phase 4 余量：出进程后端（SDK/ACP/Codex/Claude Code）消费 `OrbitSetup.providers` 预留 seam；`interrupt_subagent`；`inherits_parent_context` session-seeding；continuable/background run + 生命周期事件 |
| **MCP 多传输 + 重连** | `mcp-client` Cordis 插件：stdio + Streamable HTTP 双传输，`RECONNECT_DEFAULTS` 重连监督，`failOnStartupError` 启动失败即熔断，per-tool call timeout，`cwd` 每服务 | `crates/agent/mcp/`：stdio-only `StdioTransport`，`Mcp::spawn()` 一次性，掉线即工具死亡；`McpServerConfig` 无 `cwd`/`toolCallTimeoutMs`/`reconnect`/`failOnStartupError`；**仅 task CLI 路径接线**（`task/runner.rs:203-209`），daemon/orbit 路径零 MCP | 需补 `StreamableHTTPClientTransport` + reconnect supervisor + 启动失败熔断 + per-server timeout/cwd，并将 MCP 服务注入 daemon `orbit_setup` 容器 |
| **session resume 生产路径** | `SessionPersistence` 抽象（`prepare`/`load`/`inspect`/`readFrom`）+ JSONL/SQLite 双后端 + `session-checkpoint-policy` 在 llm/tools/pre-step 前自动 flush durable log | **已落地（#382 B2a，2026-09-18）**：daemon 重启后 orbit worker 按 `session_id` 从 session DB 加载最近 50 条 user/assistant 历史回放进 `ctx.messages`（`Worker::resume_session` + `OrbitEngine.resumed_session` 去重 + `dispatch WorkerPrompt` 接线），session 切换清空旧历史 | 余量：checkpoint flush 策略（dsh `session-checkpoint-policy`）；回放跳过 `System`/`Tool` 行 |
| **附件全链路** | `AttachmentStore` 抽象（`validateImage`/`saveImage`/`readImage`/`readImageRequest`）+ `LocalAttachmentStore` 内容寻址 + `ui-attachment` 前端 + adapter `resolveAttachments` 注入 | omenic 全仓 grep `attachment`/`image` = 0 命中（仅 `ETXTBSY` 误匹配） | 前端上传 UI + 后端持久化 + 上下文投影注入三段全缺 |
| **LLM provider 注册表/路由 + retry + token-meter** | `LlmRuntime` 服务（`registerAdapter`/`registerConfigurableProviders`/`stream` waterfall）+ DeepSeek/PiAi 双 adapter + `llm-retry` 插件（provider 路由指数退避）+ `TokenMeter`（event tail 回放算用量） | **waterfall 已落地（#382 B2b，2026-09-18）**：`orbit::WaterfallLlm`（primary + `[[llm.fallbacks]]` 按序切换，per-provider `RetryPolicy` 指数退避，已泄内容不切 provider、abort 即停、全败带序号 Error）+ web settings Fallback Provider 表单 + `[[llm.fallbacks]]` roundtrip + 契约锚点；无 fallbacks 时保持 `HttpLlm` 零变化 | 余量：token-meter（C8 裁定维持不做）；DeepSeek/PiAi 官方 adapter（现走 OpenAI 兼容端点，`[[llm.fallbacks]]` 已可配任意 OpenAI 兼容 provider） |
| **jobs 后台作业 + terminal 持久 PTY** | `JobRegistry` 抽象（`start`/`list`/`kill`/`wait`/`onJobDone`）+ `LocalJobRegistry` 内存实现 + `terminal` PTY 后端（bash/pwsh）+ 6 个模型工具 | 无 `jobs`/`terminal`/`pty` crate；`Cargo.toml` 无相关依赖；grep `pty` 仅误匹配 `subagent`/`opportunity` | 长命令被 30s 超时杀掉；模型无法维持跨 tool call 的交互 shell 状态 |
| **session telemetry/otel + title-llm** | `SessionTelemetryBackend` 抽象 + OTel SDK 导出 + `SessionTitleService`（确定性 fallback + LLM 生成） | omenic 全仓 grep `session_telemetry`/`opentelemetry`/`session-title`/`title-llm` = 0 | 无 OTel 可观测性管道，无自动 session 标题 |
| **credentials/authorization + identity** | `CredentialProvider` 抽象（分层 env 解析 + YAML 持久化 + 跨进程锁）+ `AuthorizationService`（one-attempt-per-key）+ `AnonymousUserId` | omenic 全仓 grep `credentials`/`identity`/`anonymous-user-id` = 0 | 无托管凭据存储、无 OAuth 交互授权流、无稳定匿名身份 |

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
