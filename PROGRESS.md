# omenic PROGRESS（功能账与规划）

> 本文只讲两件事：**已实现的功能**、**没实现的功能（队列与计划）**。
>
> - 交付史（哪个 PR 做了什么、审查拦下过什么）：git 历史 + `gh pr list --state merged`（PR 标题即索引，正文是细节）
> - 代码结构（某能力在哪个文件/符号）：`cg search` / `cg summary` 现查，文档不复制、不漂移
> - 技术教训（会重复犯的错与规则）：[LESSONS.md](LESSONS.md)
> - 编号体系：`C1–C8` 能力域 / `R1–R7` 并发路线 / `G1–G8` 整合门 / `B1–B3` dsh 对照批次 / `P0–P2` 体验批次 / `F1–F4` 审查遗留——均只见于历史 PR 标题，本文不再展开

## 当前位置（2026-09-26）

main `e7fc982c`（Q1 附件 / Q7 aside / Q8 subagent / Q2 凭据档案 + 三件补账已合）。daemon / web / CLI / TUI（`oi tui`）**四入口**均可构建运行（CI 证实；TUI 真人 TTY 验证 2026-09-25 通过）；web 本地停用中，复起：`hub start omenic-daemon`（`./target/debug/daemon`，**cwd=仓库根**读 `.oi/config.toml`）+ `hub start oi-web`（`./target/debug/oi-web`，端口 8026；daemon 重启后先发一条预热消息避开 known-issue 1）。结构收口已落地：p3 #452（JSONL 原语提取 + God 文件拆分）与 p4 #457（fully-flat 26-crate 树，`crates/agent/orbit` → `crates/agent-loop/src/orbit`、`agent-loop/compaction|instruction` 拍平进父 crate）。

## 已实现（能力级）

| 能力 | 说明 |
|---|---|
| agent 循环 + 工具系统 | orbit 循环（5 不变式）+ 10 内置工具（Guarded 策略包装） |
| 会话持久化 + crash-repair | sqlite 会话 CRUD；daemon 启动把半开 run 标 ABORTED（幂等） |
| 事件流推 web | `AgentEvent` DTO + daemon 广播订阅 + web 断线退避重连 |
| compaction + AGENTS.md 注入 | 插件化压缩与指令注入，**生产路径已生效**（非仅测试口径） |
| web 界面全真数据 | 聊天流式 + tool 折叠卡 / 侧栏谱系树 / 任务看板 / 统计 / 设置页（TOML 往返） |
| 附件（图片）全链路 | composer 选图 → base64 随消息入队 → sqlite 落库 → resume 回放 → OpenAI 多模态 content 数组；daemon 侧 media type 白名单 + 严格 base64 + 体积上限 |
| TUI（`oi tui`，第三 daemon 客户端） | `--tui auto\|enhanced\|linear` 探针门三档：linear 线性回显（管道 / dumb / 无色 / 复用器内自动回落）+ enhanced 全屏 alternate screen（transcript + composer dock + 历史/中断键）+ `--reduced-motion` 降动效；契约测试 42 条（`crates/tui/tests/`，14 文件）。T1 #425、T2 #430/#438、T3 #447/#449、T4 #448/#451、T5 #453 文档收口、T6 #462 transcript 滚动 + tail follow、T7 #468 鼠标滚轮（归一化常数，出处 grok-build `mouse.rs` / jcode `navigation.rs`） |
| 插件面 | 服务容器 + 事件总线 + 生命周期 + 注册表（重名拒绝）；`assemble()` 装配 Compaction/Instruction 两核心插件，daemon bind 前消费 |
| MCP | stdio + Streamable HTTP 双传输、重连监督（指数退避 + 熔断）、per-server timeout/cwd |
| session resume | daemon 重启后按 session 回放最近 50 条 user/assistant（dedupe + 切换清 ctx） |
| LLM 路由 | `WaterfallLlm` fallback 按序切换 + 共享退避重试 + 已泄内容不切 provider；web Fallback 表单 |
| 凭据档案 | `[[llm.profiles]]` + `[llm] active_profile` 切 provider；`api_key_env` 让共享配置不带明文 key；档案名不存在即加载失败（不静默换 provider） |
| jobs + terminal | 后台作业注册表 + 持久 PTY（drain 语义 read）；10 把模型工具接入 daemon；作业完成经 `on_job_done` 以 **aside** 进模型上下文（不延长 run） |
| subagent | fork 进程内后端 + ACP 出进程后端（两阶梯 dispose）+ interrupt + `[[subagent.providers]]`；运行中发消息（`subagent_control message`，step 边界投递，不谎报给不读 inbox 的 provider）；worker 拆除 SIGTERM 宽限 |
| todo + goal 全链路 | 5 把模型工具（todo_add/todo_update/todo_list/goal_add/goal_link）+ jsonl 存储 + `todo.list`/`goal.list` 读 RPC + web 看板投影（CLI 任务/模型 todo-goal/run 记录三类同板） |
| session 自动标题 | 首条用户消息确定性截词（40 字符预算 + markdown 剥刺 + emoji 安全）+ `session.update_title` 增量 RPC |
| plan-mode | `/plan` 家族在 turn 间切状态（daemon 拦截 worker.prompt）；`exit_plan_mode` 工具挂 review port，计划评审问题经 `user.question` 事件推送 + `user.answer`/`user.question.pending` RPC 回答；web 问题卡（composer 上方）；plan:policy 段按态注入 system prompt（orbit 引擎每轮重算） |
| guard | repeat-tool-reminder + timeout-policy 两插件，`config["guard"]` 切片调参，缺省静默降级 |
| skill | 有界发现 + catalog 服务 + skill 元工具（cwd 发现，注册进 harness.tools） |
| DeepSeek adapter | agent/adaptor 内方言适配（OpenAI 兼容端点之外的模型特性入口） |
| boot/bundle profiles | `profiles/boot.toml` + `profiles/bundle.toml` 内嵌于 CLI，`oi profile list\|apply` 写入 `.oi/config.toml`（不覆盖）；声明式起点，非热切换 |

## 没实现（队列，按「打开 web 用时哪里卡」排）

### P1 — 下一批候选（不阻塞，做了能感受到）

（2026-09-21 P1 批次已清空：plan-mode/guard/skill 三件、boot/bundle profiles、DeepSeek adapter 全部落地，见上表。下一批候选待定。）

### P2 — 明确延后（文字体验没稳住之前不排）

session telemetry+OTel（~2–3 天）/ token-meter（~1 天，C8 裁定不做，token 卡「无数据则隐藏」已兜底）。

附件管线（#478 `a49d755f`：选图 → base64 入队 → sqlite 落库 → resume 回放 → OpenAI 多模态 content 数组；daemon 侧白名单 + 严格 base64 + 体积上限）已落地，composer 附件卡三态补齐（#491 `e2181266`：处理中 + 错误两态，错误列文件名做 HTML 转义）；仍缺 freebuff「附件处理中挡发送」。credentials（#485 `5a596807`：`[[llm.profiles]]` + `active_profile` + `api_key_env`，档案名拼错即加载失败）已落地「单文件 + 切换」，校验状态补齐（#490 `97ac9ac1`：`ProfileStatus` + daemon 启动日志报不可用档案）；指纹经裁定不做（单机定位，无跨设备对手方），设置页档案卡与 identity 维度仍欠账。

### 本批落地（2026-09-26，refs 四项目对照后的四个缺口）

| 项 | PR / commit | 落地内容 |
|---|---|---|
| Q1 附件 | #478 `a49d755f` | 选图 → 编码 → 入库 → resume → 模型；daemon 侧校验白名单 |
| Q7 aside | #481 `1cac9b78` | `LoopConfig::get_aside` 通道 + jobs `on_job_done` 回调：作业完成以 aside 进上下文，**不延长 run**；走直接回调 + 共享队列（未走事件总线）|
| Q8 subagent | #483 `eb2d7b3b` | 每 run 信箱 + `subagent_control message`（step 边界投递，不打断在飞工具）；worker 拆除走 SIGTERM → 500ms → SIGKILL |
| Q8 后台 run | #494 `e7fc982c` | `subagent` 加 `background` 开关：后台 run 完成后经 #481 的 aside 通道通知模型，`subagent_control result` 取回输出；同时让 `send_message` 真正可达 |
| Q2 credentials 校验 | #490 `97ac9ac1` | `ProfileStatus`（Ready/MissingKey/Incomplete）+ `Config::profile_statuses()`；daemon 启动对不可用档案打一行日志（Ready 的保持安静） |
| Q1 附件卡三态 | #491 `e2181266` | 处理中 + 错误两态（Dioxus 静态容器由 JS 填，文件名 HTML 转义）；待发态维持 #478 原样 |
对账补记（2026-09-26，cg + jev 复审）：Q1 UI 卡三态缺两态、Q2 指纹/校验状态缺两件、Q8 的 subagent 完成通知未做、Q7 未走事件总线——四项均已写进 `todo/q-roadmap/` 各 Q 文档的「落地结果 / 状态」节。
补记落地（#490 / #491 / #494）：Q2 校验状态、Q1 卡片三态两项已补齐；Q8 的「subagent 完成通知进 aside」与「send_message 投递窗口」同根的那项（缺 background run）随 #494 落地——`subagent background` 让 run 活过工具调用，完成事件经 `set_on_settled` 进 aside 队列，`send_message` 也由此可达；Q2 指纹经裁定不做（单机定位）。

审查期修掉的真问题：Q8 的控制工具对「不读 inbox 的 provider」（ACP 等）谎报投递成功 → 加 per-provider 能力位；Q1 合并 main 后新增的调用点（tui inline/local、workspace echo、kymic CLI 测试）漏改 → 补齐；Q7 的 `session_tools` 文档注释被新 helper 顶掉 → 复原。

### 批次余量（不阻塞，可随时捡）

- **MCP**：tool-level filter、server-level env 注入
- **session resume**：checkpoint flush 策略（回放本就只取 user/assistant，无 System/Tool 行问题；`pending_titles` 重试随 #452 落地）
- **LLM 路由**：全败 Error 不聚合各 provider 原始错误文本；statusline fallback 切换 `model` 显示口径
- **jobs/terminal**：pwsh 后端、dsh jobs 其余工具（`jobs_output` 等）（`onJobDone` 回调随 #481 落地，aside 通道已通）
- **subagent**：`send_message`/`report`、continuable/background run、session-seeding、permission option kind、Codex/Claude Code/SDK 三进程外后端（`send_message` 与 SIGTERM 中间层随 #483 落地；ACP 消费 inbox 留后续）

## 不做（裁定，勿翻案）

| 项 | 原因 |
|---|---|
| 元工具重子集：lsp / schedule / hooks / workflow | 体验增益小或方向相反（dsh workflow 是模型运行时自写编排，omenic 的 task 是 RPC 任务模型） |
| core scope + agent-tool-presentation | dsh 作用域隔离与工具结果呈现策略，不与主线耦合 |
| C7（tag omenic-harness-v0.1.0 + ferrite 接线） | 前置全满足，**等用户拍板时机**，不占批次 |
| C8 interaction + token-meter | 已裁定不做 |

**独有功能保护区**（omenic 独有、dsh 无对应物，只读或单向依赖，不当缺口塞）：`crates/infra/memory`、`crates/agent/task`、`crates/evidence/spec` 的模板层。Gate 正本已迁至 Canon；omenic 只保留 `.githooks/spec/` 项目规则与 hook 接线，不再保留 Gate 源码或仓库内二进制。

## 接下来可并发推进

以下路线修改面独立，可分别在 `.wt/<branch>` 推进：

1. **Web 稳定性**：修复 daemon 重启后首条消息空 turn，以及首条消息标题**跨页面刷新**的重试缺口（会话内重试已有，见 known-issue 2）。两项集中在 daemon/web 边界，优先消除当前使用阻塞。
2. **Leptos 资源基线**：保持现有 Dioxus 路径不动，在独立应用中完成空载、单会话和流式消息场景的 CPU/内存对照。数据不足前不启动迁移。
3. **代码结构治理**：扩展 `.githooks/spec/quality/` 的结构规则，优先覆盖超大文件、重复实现、模块聚合和无效包装。Gate 只消费规则，不重新承载项目策略。
4. **Agent 能力缺口**：本批 Q1/Q2/Q7/Q8 已合（见上表）。下一批从队列里独立实现 MCP tool-level filter、session resume checkpoint flush、Q3 telemetry，或 jobs/terminal 的 pwsh 后端。每条路线限制在一个能力域。
5. **TUI 批次收口**（epic #423）：七阶段交付完毕——T1 #425、T2 #430/#438、T3 #447（`7729268`）/ #449（`5ec96c4`）、T4 #448（`79bc37f`）/ #451（`4cb52e7`）、T5 #453 文档收口、T6 #462（transcript 滚动 + tail follow）、T7 #468（鼠标滚轮）、T8 #469（inline dock 原生回滚）、T9 #476（slash 命令面板）、T10 #479（飞行中提示队列 + recall）。遗留：TUI PTY smoke job 待授权后补 CI 级全屏断言（T2–T4 TTY 级证据欠账）。

整合顺序：先合 Web 稳定性；Leptos 只产出基准结论；结构治理和 Agent 能力可并行，避免同时修改共享 composition/daemon 入口。

## 未修 known-issues（实测确认，未排期）

| # | 现象 | 根因位置 | 规避 / 前置 |
|---|---|---|---|
| 1 | 冷启动 daemon **首个 prompt 静默丢 run**：32ms 空 turn（`↑0 ↓0`）、UI 无错误提示，第二条起正常 | daemon worker 懒拉起与首 prompt 竞态 | daemon 重启后先发一条预热消息 |
| 2 | 首条消息标题更新失败：会话内重试已落地（#452 `pending_titles`：落库失败后后续发送继续重试同一标题；本地标题已改则不覆盖），剩余缺口是**跨页面刷新无重试**（`pending_titles` 仅内存态，刷新后占位 `会话 <ts>` 重新派生） | page-workspace `on_send` + `pending_titles` | 占位加可识别前缀 + 刷新后从 daemon 侧补重试 |

## 工作约定

- **工作目录**：`.wt/<branch>`（git worktree add 必须在仓库根执行，防嵌套）
- **PR-only**：2026-09-15 裁定只开 PR 不开 issue；缺 `Fixes #` 只是 WARN 不阻塞
- **测试一律不在本地跑**：`cargo test` / `clippy` / 全量 `cargo build` / `npm install` 全部交给 PR 的 CI。本地只允许 `cargo fmt --check`、单 crate `cargo check -p <crate>`（**不得**加 `--workspace` / `--all-targets`）、grep/read/git/读写
- **唯一例外**：web UI 需要肉眼确认时的 `cpulimit -l 65 -i -- cargo build --bin oi-web`
- **提交前必跑** `cargo fmt --all`（commit checklist hook 拦不合格的 rust）
- **测试放同层 `tests/`**，不在 src/ 写 `#[cfg(test)]`（gate 规则 `rust_tests_in_tests_dir`）
- **`git commit` 退出码要显式查**：`git commit ... | tail -1` 会吞 pre-commit hook 失败，随后 `--amend` 会修错 HEAD（2026-09-21 踩过）
- **CI 的 `pull_request` 触发偶发不 firing**（commit status pending 但零 run）：`gh workflow run ci --ref <branch>` 手动补
- **调查优先**：改文档或共享代码前先 `cg status` → stale 就 `cg refresh` → 对主张 `cg search`/`callers` + grep 确认落点再落笔；文档只断言少而 forward 的事实，代码事实交给 cg/git 现答
- **`.githooks/` 只读**：gate 规则、hook 脚本不许改；gate FAIL 了改自己的提交/正文去迎合，不要改规则
- **合并**：`gh pr merge --squash` + 删分支 + 清工作树；绝不本地 merge 到 main；合并留言以 `Agent 🤖 - Merge:` 开头；PR 正文与 CRG Review comment 的 heading 必须**纯英文**（gate 拦 CJK）
- **审查双层（B3 起强制）**：合并前 CRG `detect-changes`（0 affected flow 为结构判据）+ ocr 分批（大 diff 按目录分批）；ocr 稳定误报类直接裁定驳回
- **领域依赖方向**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO
- **改组件源码要同步** `.githooks/spec/` 下对应 UI 契约 yaml 的 `find` 锚点（新 spec 必须登记 `bin/web/tests/ui_contract.rs` 的 `SPEC_FILES`）
- **加依赖必须同步改 `Cargo.lock`**（CI `--locked` 拒绝重算）
- **无 `Co-authored-by` trailer**
