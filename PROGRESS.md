# omenic PROGRESS（当前进度与规划）

> 已落地的背景见 [ROADMAP.md](ROADMAP.md)（已完成档）。本文只讲**还没做的**和**接下来做什么**——两份文档状态不同，ROADMAP 是已完成的沉淀，本文是未完成的规划。
>
> 编号体系沿用 ROADMAP：`C1–C8` / `R1–R7` / `G1–G8`。

## 当前位置（2026-09-20）

main 现为 `a15171f`。B1（#381）/ B2（#382）/ 审查收尾（#383，`b20ebc0`）已合入；B3 两块亦已合入：PR1 #385（jobs + terminal + web 词表，squash `bc24a7e`）与 PR2 #387（subagent Phase 4：ACP 出进程后端 + interrupt，squash `a15171f`）。三批全部完成，见下文各节与 ROADMAP 的 dsh 对照表。

**G1–G8 全部已过，main 干净（`8d862e4`）。subagent 能力 seam Phase 1 已落地（#380，main `73493b5`，2026-09-17）：harness 容器现在有 `harness.subagents` 服务 + model-facing `subagent` 工具 + fork backend（复用 `subagent::runner`），daemon 生产路径可解析；**Phase 4 的 ACP 出进程后端 + interrupt 亦已落地（#387，2026-09-19）。** G6 之前的 6 条开放缺口**全部关闭**（1/2/3/5 由 G6，4/6 由 G7）：

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

**G8 之后没有排队中的整合点。** 接下来走 dsh 全量对照 backlog：2026-09-18 已划分三批（B1 MCP 捆绑 web 契约 / B2 session resume + LLM 路由 / B3 jobs + subagent P4），附件与遥测类不排批次（见「后续 backlog」）。B1（#381）/ B2（#382）/ 审查收尾（#383）已合入 main（`b20ebc0`），见上文「当前位置」；**B3 全部完成并合入 main（PR1 #385 `bc24a7e` + PR2 #387 `a15171f`，2026-09-19），见下两节。**

**B3-PR1 已合并（jobs + terminal + web 词表，2026-09-19，PR #385，squash `bc24a7e`；分支 `feat/b3-jobs-terminal`，base `e8b271f`）：**
- **两个新 crate**：`crates/harness/jobs`（`JobRegistry` trait + `LocalJobRegistry`；全部方法取 `&self`，`Mutex` + `Condvar` 同步，作业跑在 `std::thread` 上）与 `crates/harness/terminal`（`TerminalRegistry`，portable-pty 0.9 后端，每会话一个 reader 线程把 pty master 排空进内存 buffer，所以 `read` 非阻塞且是 **drain 语义**）。14 + 15 个本地测试全绿。
- **6 个模型工具 + 4 个 terminal 工具**：`crates/harness/tools/src/jobs_terminal.rs`（`session_tools()` 一次建 10 把，`SESSION_TOOL_NAMES` 常量做单一真源）。实现的是 **agent-domain `tools::Tool`**，不是 harness `Tool` —— 引擎派发的就是前者、MCP 工具也是这个形状，直接实现省掉一层 adapter（C6 的两 trait 分离仍成立）。
- **daemon 接线**：沿用 MCP 的同款 seam —— `OrbitConfig.session_tools: Arc<Vec<Arc<dyn Tool>>>`（与 `mcp_tools` 并列），daemon `session_tools()` 建一次注册表、每个 engine respawn 克隆同一份 `Arc`。`combined_tools` 合并顺序 catalog → MCP → session。
- **web 词表**：`ui_state.rs::tool_call_from_rpc` 加 `jobs_*` → `bash`/`job`、`terminal_*` → `terminal` 分支，标题取 `id`/`data`/`command`（兜底分支只认 `path`，会退化成工具名）；`chat.rs::kind_chip` 给 `job`/`terminal` 复用 brand 配色；`chat.yaml` 的 `chip-kind-bash` 锚点同步改。
- **两个真实发现（非测试瑕疵）**：
  1. **pty 会回显输入，且"数出现次数"不是解法。** `write("echo hi\n")` 后读端有两份 `hi`（回显 + 输出）；想靠"等出现两次"绕过不可靠——pty 无分帧，回显本身可能跨 read 到达而被数两次，调用方在命令跑之前就返回了。正解是让 shell 拼出哨兵（`printf '\n__DONE_%s__\n' OK`），命令行文本里不含该字面量，于是任何一次出现都是真输出。已写进 terminal crate 模块文档 + 测试 helper。
  2. **`TerminalRegistry::list()` 曾自死锁**：持 `sessions` 锁再调 `status()` → `get()` 重入同一把 `std::sync::Mutex`。靠本地跑测试发现（挂起 >60s），抽 `summarize(&TerminalId, &Session)` 直取 `Arc` 修掉。
- **代码审查（CRG + ocr + 三层）修出的问题**，见下节「B3-PR1 审查修复」。
- **余量（不阻塞）**：dsh `jobs` 的其余 4 个工具（`jobs_output` 等）；terminal 的 pwsh 后端；`onJobDone` 生命周期回调。


### B3-PR1 审查修复（三个 commit，2026-09-19）

CI 首轮红两次，两次都是**既有测试本身的缺陷**，不是新代码引入的：

1. **`daemon_subagent_seam_e2e` 的 mock 截断了请求体**（`1197399`）。`serve_smoke` 从 `buf[buf.len() - body_len.min(buf.len())..]` 取 body 起点，但 `buf` 此时只含 header（读循环在 `\r\n\r\n` 停），偏移量塌成 0，于是 `rest` 被塞进 header 字节、补读循环正好短一个 header 长度（实测 9401 字节的请求只记录到 8538）。JSON 解析失败 → 模型从未拿到完整工具表 → 没有 `tool_calls` → 循环直接 `agent_end`。按 `\r\n\r\n` 切分、从空开始补 body 修掉；顺带把脚本槽位从**连接序**改成**请求序**、计数从 accept 时自增改成 body 入列后自增（两者都会掩盖同一故障）。
2. **`mcp_config_validate` 的 cwd 竞态**（`6e722f0`）。`Config::load` 按**进程** cwd 解析 `./.oi/config.toml` 且无目录参数，`set_current_dir` 同样进程级，三个用例在 cargo 并行线程下互相抢。加 `cwd_lock()` 把（切 cwd、load、还原）整段串行化；锁对 poison 容忍（一个用例失败不该让另两个报"poisoned lock"）。临时禁用锁可稳定复现三个全 fail。

3. **terminal 会话的 kill 不彻底（flaky 的真因，`64a6bad`）**。`kill_leaves_the_record_so_output_can_still_be_drained` 三 crate 合跑时约 1/6 失败，报 `killed shell never reported exit`。根因不在测试：

   - `kill` 委托 `portable_pty` 的 `Child::kill`，它在 unix 只对**单个 pid** 发 `SIGHUP` 然后 reap。pty 只要**还有任何进程持有 slave** 就不关闭，而 `bash` 死时不杀自己的子进程 → 被孤立的 `sleep 30` 攥着 slave，reader 线程一直停在 `read(master)`，会话看起来"还活着"直到孤儿到期（最多 30s，而测试只等 5s）。
   - **杀 shell 的进程组也不够**：job control 给每个前台/后台作业单独一个进程组，实测 `bash pgrp=700823` / `sleep pgrp=700825`（`sid` 都是 700823），所以 `kill(-shell_pgrp)` 只打到 shell。
   - 真正共享的是**会话 id**（`portable_pty` 在 `exec` 前调了 `setsid`），信号改成打 `-sid` 对应的进程组。

   实测三种信号：`SIGTERM` **根本不杀** pty 上的 bash（会话存活）；`SIGHUP` 杀掉 shell 但 pty 永不 EOF（孤儿仍持 slave）；`SIGKILL` 才既 reap 又让 pty 在 ~0.4ms 内 EOF。故 `kill` 改用 `SIGKILL`。

   同批还修掉审查意见里的若干项：`push_output` / reader 退出路径改为**持锁 notify**（原先 drop guard 后再 notify，`read` 在进入 `wait_timeout` 时已释放互斥锁，窗口内的通知无人接收 → 丢失唤醒）；全部 `.lock().unwrap()` 换成 `lock_recover` / `state_guard`（对齐 `crates/agent/mcp`、`crates/infra/daemon` 的 poison 容忍惯例）；`run_command` 的 reader join 不再吞 panic（空串与"命令没输出"无法区分）；`kill_tree` 返回并检查结果（仅非 ESRCH 时报错）。

   验证：33 测试全绿（14 jobs + 15 terminal + 4 tools），三 crate 合跑 8 轮无失败；复现用的 kill 探针从近乎全败变为 60/60 通过。

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

**B3-PR2 已合并（subagent Phase 4：ACP 出进程后端 + interrupt，2026-09-19，PR #387，squash `a15171f`；分支 `feat/b3-subagent-p4`，base `bc24a7e`，issue #386）：**

- **ACP 协议层**（`crates/harness/subagent/src/acp.rs`）：JSON-RPC 2.0 over NDJSON 的 client 最小子集（initialize / session-new / session-prompt / session-cancel / session-update 通知 / session-requestPermission 应答），`AcpClient` 持 stdin/stdout + 读线程分派，`AtomicU64` 请求 id + `mpsc::sync_channel(1)` pending 表，`AcpHandlers` trait 让 provider 注入文本累积与权限应答。9 个集成测试（内存 mpsc 管道，不起进程）覆盖初始化失败、静默 new-session、通道关闭、未知 pending id。
- **出进程后端**（`acp_provider.rs`）：`AcpProvider` 实现 `SubagentProvider`——spawn 外部 agent，跑 handshake + 一轮 prompt，流式 assistant 文本折叠成最终输出；`AcpDisposer` 两阶梯销毁（cancel → 关 pending 表 → drop stdin EOF → `eof_grace` 轮询 → SIGKILL → `kill_grace` → wait reap），幂等（`AtomicBool::swap`），exit watcher 线程保证 child 中途崩溃不把 `prompt` 楔死。`AcpPermission` Allow 取第一个 option、Reject 一律 Deny。
- **interrupt 链路**：`RunDisposer` trait 外化销毁（`SubagentRun::new` 三参数、`dispose()` 无参，fork 用 `ForkDisposer` 翻 signal）；`SubagentRuntimeService` 加 run 注册表（`start_run` 发 `sub-N` 序号 id、`interrupt` / `finish_run` / `active_runs`）；`subagent` 工具改走 `start_run` 并在输出 payload 回 `run_id`；`subagent_control` 加 `interrupt` action（未知/已结束的 run 返回 `interrupted:false` 负载而非工具错误，和 list 一致）。
- **配置 + daemon 装配**：`[[subagent.providers]]`（name/command/args/env/cwd/permission/dispose_grace_ms/dispose_eof_grace_ms，校验拒空 name、保留名 `fork`、空 command，merge trim）+ `DaemonConfig.subagent_providers` 透传；daemon `orbit_setup` 把每条配置翻译成 `AcpProviderSpec` 注册（command+args 空白拼接，含空白参数的升级路径记在注释），并把 `subagent` / `subagent_control` 工具注册进 harness `ToolCatalog`（**不是** `session_tools`：这两个工具实现的是 harness `Tool` 而非 agent-domain `tools::Tool`，而 catalog 路径的 `HarnessTool` 桥接已就绪且把引擎 abort flag 注入 `AbortSignal`——interrupt 就骑在这条线上）。
- **一处 clean cutover**：`OrbitSetup.providers`（#380 给 Phase 4 预留的 (name, allow-list) 占位字段）**删除**——落地方式确认是 daemon 装配层直接注册 provider 与工具，worker 不再需要这个意图字段；两处 rpc 测试的构造点同步改掉。issue #386 的 Done-when「providers seam 意图传到 worker」因此调整为「daemon 装配层消费」。
- **web 词表**：`ui_state.rs::tool_call_from_rpc` 加 `subagent` / `subagent_control` → `subagent` kind，标题取 prompt / run_id。
- **测试规模**：subagent crate 9（协议）+ 9（provider，自带 `mock_acp_server` 脚本化子进程，覆盖 MOCK_HANG 在 eof_grace 内被收 / MOCK_IGNORE_CANCEL 升级 SIGKILL / MOCK_CRASH / MOCK_PERMISSION 双策略 / 无 session id 回滚）+ 4（interrupt，BlockingBackend + gate 做确定性）+ config 8 + daemon e2e 3（装配链路：ACP 文本回传、control list 同时列出配置 provider 与内建 fork、未知 provider 失败不拖垮 daemon）。`cargo check --tests` 全 crate 零警告，`cargo fmt --check` 通过；测试本身推 CI。
- **明确省略（余量）**：SIGTERM 中间层（std 无可移植信号 API，`libc`/`portable-pty` 可补）；permission option 的 kind 过滤；`send_message`/`report`、continuable/background run、session-seeding；args 含空白的命令行（spec 只有单一 command 字符串）。
**`cargo check` 只做类型检查，本地不跑 `cargo test`（CI 跑）；B3-PR2 的 daemon e2e 用 `current_exe()` 的相对路径找 `mock_acp_server`（`CARGO_BIN_EXE_` 只在同 crate 的测试里可见）。**

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

### 批次 B3：jobs + terminal（Rust 侧）+ subagent Phase 4 ✅ 已完成（PR1 #385 / PR2 #387，2026-09-19）

- **jobs + terminal 持久 PTY**（dsh `packages/jobs/jobs*` + `terminal/terminal*` 可对照）：`JobRegistry` trait（`start`/`list`/`kill`/`wait`/`onJobDone`）+ `LocalJobRegistry` 内存实现；PTY 后端 bash 起步（pwsh 可选）；6 个模型工具中先做 `jobs_wait`/`jobs_kill` 两把最刚需的，其余（`jobs_list` 等）可余量。解决长命令 30s 超时被杀、模型无跨 tool call 交互 shell。
- **subagent Phase 4** ✅ 已完成（#387）：`interrupt_subagent` 实现（runtime run 表 + `subagent_control` interrupt action + `RunDisposer` 外化销毁）+ ACP 出进程后端起步（`AcpProvider`，照 dsh `packages/subagent/subagent-acp`，两阶梯 dispose）。**消费方式确认后 `OrbitSetup.providers` seam 已删除**（装配上移 daemon，见 B3-PR2 节），不是预留占位。余量（仍缺）：`send_message`/`report`、continuable/background run、session-seeding、SIGTERM 中间层、permission option kind 过滤、其余三个进程外后端（Codex / Claude Code / SDK）。
- **Web 面小改动**（顺带带上，不独立 PR）：`chat.rs` 工具折叠卡加 `jobs_wait`/`jobs_kill` 工具名进词表（否则 LIFO 当未知折叠掉）；`subagent_report` 新帧进词表；侧栏 `group_sessions` 已有树不用动。

PR 拆分（已按此执行）：PR1 jobs + terminal + web 工具词表（#385）；PR2 subagent P4（ACP + interrupt，出进程后端是跨系统协议验证，单独一个 PR 便于隔离，#387）。

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
| **subagent 能力 seam** | `SubagentRuntime` 服务（provider registry + one-shot/continuable + 生命周期事件）+ 11 子包（spawn/fork 进程内 + ACP/Codex/Claude Code/SDK 四个进程外后端 + control/report 工具） | **Phase 1 已落地（#380）**：新增 crate `omenic-harness-subagent`（`SubagentProvider` trait / `SubagentRuntimeService` / `ForkProvider` in-process backend 复用 `subagent::runner` / model-facing `subagent` + `subagent_control`（list only）工具）；`ToolCatalog` 内建可变；composition 注册三个插件；daemon `orbit_setup` 注册 fork provider（只读工具子集）；e2e smoke 通过 | **已完成（B3-PR2，#387，2026-09-19，squash `a15171f`）**：interrupt + ACP 出进程后端 + `RunDisposer` 外化销毁 + `[[subagent.providers]]` 配置；余量：`send_message`/`report`、continuable/background run、session-seeding、SIGTERM 层、permission kind、Codex/Claude Code/SDK 三个后端 |
| **MCP 多传输 + 重连** | `mcp-client` Cordis 插件：stdio + Streamable HTTP 双传输，`RECONNECT_DEFAULTS` 重连监督，`failOnStartupError` 启动失败即熔断，per-tool call timeout，`cwd` 每服务 | **已完成（#381，2026-09-18；#383 补审 2026-09-19）**：stdio + Streamable HTTP 双传输、重连监督（指数退避 + 重试上限熔断）、per-server timeout/cwd、`fail_on_startup_error`、daemon `orbit_setup` 注入；`write_full_config` 丢 `[[mcp.servers]]` 已在 #383 修（`6661a70`） | 余量：dsh `mcp-client` 其余可选项（tool-level filter、server-level env 注入）未对齐 |
| **session resume 生产路径** | `SessionPersistence` 抽象（`prepare`/`load`/`inspect`/`readFrom`）+ JSONL/SQLite 双后端 + `session-checkpoint-policy` 在 llm/tools/pre-step 前自动 flush durable log | **已完成（#382 B2a，2026-09-18）**：daemon 重启后 orbit worker 按 `session_id` 从 session DB 加载最近 50 条 user/assistant 历史回放进 `ctx.messages`（dedupe + 切换清 ctx） | 余量：checkpoint flush 策略（dsh `session-checkpoint-policy`）；回放跳过 `System`/`Tool` 行 |
| **附件全链路** | `AttachmentStore` 抽象（`validateImage`/`saveImage`/`readImage`/`readImageRequest`）+ `LocalAttachmentStore` 内容寻址 + `ui-attachment` 前端 + adapter `resolveAttachments` 注入 | omenic 全仓 grep `attachment`/`image` = 0 命中（仅 `ETXTBSY` 误匹配） | **不排批次**（超三批上限；三段全缺，见「不排批次」节） |
| **LLM provider 注册表/路由 + retry** | `LlmRuntime` 服务（`registerAdapter`/`registerConfigurableProviders`/`stream` waterfall）+ DeepSeek/PiAi 双 adapter + `llm-retry` 插件（provider 路由指数退避）；`TokenMeter` | **waterfall 已完成（#382 B2b，2026-09-18；#383 补审 2026-09-19）**：`orbit::WaterfallLlm`（`[[llm.fallbacks]]` 按序切换 + per-provider `RetryPolicy` 退避 + 中间 Error 判据改用终态/`leaked_content`）+ web settings Fallback 表单 | token-meter 不做（C8 裁定维持）；DeepSeek/PiAi 官方 adapter 未接（现走 OpenAI 兼容端点） |
| **jobs 后台作业 + terminal 持久 PTY** | `JobRegistry` 抽象（`start`/`list`/`kill`/`wait`/`onJobDone`）+ `LocalJobRegistry` 内存实现 + `terminal` PTY 后端（bash/pwsh）+ 6 个模型工具 | **已合并（B3-PR1，#385，2026-09-19，squash `bc24a7e`）**：新增 `crates/harness/jobs` + `crates/harness/terminal`（portable-pty）；10 把模型工具（`session_tools()`）+ daemon `OrbitConfig.session_tools` 接线 + web 工具词表；33 个测试绿（CI）；三轮审查意见已落实（含 kill 打整会话、持锁 notify 等真实缺陷修复） | **余量**：`onJobDone` 回调、pwsh 后端、dsh jobs 其余工具 |
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
