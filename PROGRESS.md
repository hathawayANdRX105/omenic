# omenic PROGRESS（功能账与规划）

> 本文只讲两件事：**已实现的功能**、**没实现的功能（队列与计划）**。
>
> - 交付史（哪个 PR 做了什么、审查拦下过什么）：git 历史 + `gh pr list --state merged`（PR 标题即索引，正文是细节）
> - 代码结构（某能力在哪个文件/符号）：`cg search` / `cg summary` 现查，文档不复制、不漂移
> - 技术教训（会重复犯的错与规则）：[LESSONS.md](LESSONS.md)
> - 编号体系：`C1–C8` 能力域 / `R1–R7` 并发路线 / `G1–G8` 整合门 / `B1–B3` dsh 对照批次 / `P0–P2` 体验批次 / `F1–F4` 审查遗留——均只见于历史 PR 标题，本文不再展开

## 当前位置（2026-09-21）

main `43534a6`。daemon / web / CLI 三入口均可构建运行（CI 证实）；web 本地停用中，复起：`hub start omenic-daemon`（`./target/debug/daemon`，**cwd=仓库根**读 `.oi/config.toml`）+ `hub start oi-web`（`./target/debug/oi-web`，端口 8026；daemon 重启后先发一条预热消息避开 known-issue 1）。

## 已实现（能力级）

| 能力 | 说明 |
|---|---|
| agent 循环 + 工具系统 | orbit 循环（5 不变式）+ 10 内置工具（Guarded 策略包装） |
| 会话持久化 + crash-repair | sqlite 会话 CRUD；daemon 启动把半开 run 标 ABORTED（幂等） |
| 事件流推 web | `AgentEvent` DTO + daemon 广播订阅 + web 断线退避重连 |
| compaction + AGENTS.md 注入 | 插件化压缩与指令注入，**生产路径已生效**（非仅测试口径） |
| web 界面全真数据 | 聊天流式 + tool 折叠卡 / 侧栏谱系树 / 任务看板 / 统计 / 设置页（TOML 往返） |
| 插件面 | 服务容器 + 事件总线 + 生命周期 + 注册表（重名拒绝）；`assemble()` 装配 Compaction/Instruction 两核心插件，daemon bind 前消费 |
| MCP | stdio + Streamable HTTP 双传输、重连监督（指数退避 + 熔断）、per-server timeout/cwd |
| session resume | daemon 重启后按 session 回放最近 50 条 user/assistant（dedupe + 切换清 ctx） |
| LLM 路由 | `WaterfallLlm` fallback 按序切换 + 共享退避重试 + 已泄内容不切 provider；web Fallback 表单 |
| jobs + terminal | 后台作业注册表 + 持久 PTY（drain 语义 read）；10 把模型工具接入 daemon |
| subagent | fork 进程内后端 + ACP 出进程后端（两阶梯 dispose）+ interrupt + `[[subagent.providers]]` |
| todo + goal 全链路 | 5 把模型工具（todo_add/todo_update/todo_list/goal_add/goal_link）+ jsonl 存储 + `todo.list`/`goal.list` 读 RPC + web 看板投影（CLI 任务/模型 todo-goal/run 记录三类同板） |
| session 自动标题 | 首条用户消息确定性截词（40 字符预算 + markdown 剥刺 + emoji 安全）+ `session.update_title` 增量 RPC |

## 没实现（队列，按「打开 web 用时哪里卡」排）

### P1 — 下一批候选（不阻塞，做了能感受到）

| 项 | 为什么 | 成本 |
|---|---|---|
| plan-mode / guard / skill 元工具 | plan-mode=动手前出计划让你确认；guard=重复提醒/超时止损；skill=复用成套指令。三件可分开单件落地 | ~5–8 天 |
| boot/bundle 声明式装配 | 换模型/渠道/插件集要改 TOML+重启；声明式 profile 是多场景切换地基，也是 ferrite 复用 harness 的前置 | ~3–4 天 |
| DeepSeek/PiAi 官方 adapter | 现全走 OpenAI 兼容端点，reasoner 等模型特性吃不到 | ~2 天/adapter |

### P2 — 明确延后（文字体验没稳住之前不排）

附件全链路（~4–6 天）/ credentials+identity（~5–7 天）/ session telemetry+OTel（~2–3 天）/ token-meter（~1 天，C8 裁定不做，token 卡「无数据则隐藏」已兜底）。

### 批次余量（不阻塞，可随时捡）

- **MCP**：tool-level filter、server-level env 注入
- **session resume**：checkpoint flush 策略；回放跳过 `System`/`Tool` 行
- **LLM 路由**：全败 Error 不聚合各 provider 原始错误文本；statusline fallback 切换 `model` 显示口径
- **jobs/terminal**：`onJobDone` 回调、pwsh 后端、dsh jobs 其余工具（`jobs_output` 等）
- **subagent**：`send_message`/`report`、continuable/background run、session-seeding、SIGTERM 中间层、permission option kind、Codex/Claude Code/SDK 三进程外后端

## 不做（裁定，勿翻案）

| 项 | 原因 |
|---|---|
| 元工具重子集：lsp / schedule / hooks / workflow | 体验增益小或方向相反（dsh workflow 是模型运行时自写编排，omenic 的 task 是 RPC 任务模型） |
| core scope + agent-tool-presentation | dsh 作用域隔离与工具结果呈现策略，不与主线耦合 |
| C7（tag omenic-harness-v0.1.0 + ferrite 接线） | 前置全满足，**等用户拍板时机**，不占批次 |
| C8 interaction + token-meter | 已裁定不做 |

**独有功能保护区**（omenic 独有、dsh 无对应物——只读/单向依赖，不当缺口塞）：`crates/infra/memory`、`crates/agent/task`、`crates/evidence/spec` + `bin/gate`。

## 未修 known-issues（实测确认，未排期）

| # | 现象 | 根因位置 | 规避 / 前置 |
|---|---|---|---|
| 1 | 冷启动 daemon **首个 prompt 静默丢 run**：32ms 空 turn（`↑0 ↓0`）、UI 无错误提示，第二条起正常 | daemon worker 懒拉起与首 prompt 竞态 | daemon 重启后先发一条预热消息 |
| 2 | 首条消息标题更新失败无重试：`is_first_message` 门控一次性，占位 `会话 <ts>` 无稳定字面量 | page-workspace `on_send` | 需先给占位加可识别前缀再加重试 |

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
