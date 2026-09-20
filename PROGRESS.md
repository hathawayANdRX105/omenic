# omenic PROGRESS（当前进度与规划）

> 已落地的能力域、整合门与 dsh 对照批次（B1–B3）的交付/审查记录，全部在 [ROADMAP.md](ROADMAP.md)（已完成档）。本文只讲**还没做的**和**接下来做什么**。
>
> 编号体系沿用 ROADMAP：`C1–C8` / `R1–R7` / `G1–G8`。

## 当前位置（2026-09-20）

main `756c8fe`。**G1–G8 全部已过**；dsh 对照三批 B1（#381）/ B2（#382）/ B3（#385 + #387）已全部合入 main，交付内容、审查拦下的真实缺陷与验收证据见 ROADMAP 的「已交付的 dsh 对照批次 B1–B3」节。

- **C7（tag omenic-harness-v0.1.0 + ferrite 接线）**：前置条件全满足，**等用户拍板时机**，不占批次。
- **C8（interaction + token-meter）**：已裁定不做。

**没有排队中的整合点。** G1–G8 + B1–B3 全部交付，daemon/web/CLI 三入口均可构建运行（CI 证实）。剩下的缺口按「**使用时哪里不爽**」排——不是「dsh 有什么」，是「自己用起来卡在哪」。

## 缺口优先级（2026-09-20 二次重排，纯使用视角）

> 一次重排仍带着 dsh 对照的惯性（把附件排成 P1 是错的——文字会话本身还没到「用着顺」的程度，排图像输入是本末倒置）。本次判据只有一条：**打开 web 用的时候，哪一项缺失会直接让你觉得难用**。结论：体验增强集中在元工具与治理那片（原划范围外），存储/可观测/附件类全部是底层锦上添花。

### P0 — 使用体验的直接卡点

| 项 | 为什么是 P0 | 成本 | omenic 现状 |
|---|---|---|---|
| **元工具：todo + goal** | 真正用起来时，**任务追踪是核心体验**——开几个会话并行干活、记住每条线推进到哪、阻塞了什么。omp 的 todo 模型你已经在用；omenic 现在一个 todo 都没有，全靠脑子记。**扩 omenic 自己的 `crates/agent/task`（RPC 任务模型），不复刻 dsh 的模型运行时自写编排**（方向相反） | ~3–5 天 | `crates/agent/task` 有 runner/graph/store/template，无 todo/goal 概念；web 无任务面板数据源 |
| **session-title**（会话自动标题） | 侧栏现在全是手填/时间戳，会话一多完全没法找、没法切。dsh 确定性 fallback 就是首条用户消息截词，**不需要 LLM 也能用** | ~2 天 | **PR #391 已完成确定性 fallback**：`title_from_first_message` 首条消息截词（40 字符上限，中文/emoji 按字符计），新建仍用时间戳占位、首条消息发送后替换。LLM 生成标题仍排 title-llm（P1） |

### P1 — 体验明显变好（不阻塞，但做了能感受到）

| 项 | 为什么 | 成本 | omenic 现状 |
|---|---|---|---|
| **元工具：plan-mode / guard / skill** | plan-mode = 动手前先出计划让你确认（长任务敢放手）；guard = 重复提醒/超时止损（防 token 白烧）；skill = 复用成套指令。三件都是「用得越多越值」 | ~5–8 天（三件分开排，可单件落地） | 全无；dsh 有 guard（loop 卫生）+ skill-filesystem，均为可选增强 |
| **boot/bundle 声明式装配** | 换模型/换渠道/换插件集现在要改 TOML + 重启；声明式 profile 是「多场景切换」的地基，也是 ferrite 复用 harness 的前置 | ~3–4 天 | `composition::assemble()` 硬编码两个核心插件；无 profile schema |
| **LLM 路由余量**：DeepSeek/PiAi 官方 adapter | 现全走 OpenAI 兼容端点，reasoner 等模型特性吃不到 | ~2 天/adapter | `orbit::WaterfallLlm` 已有 fallback + retry，只缺 provider 适配 |

### P2 — 明确延后（文字体验没稳住之前不排）

| 项 | 为什么延后 | 成本 |
|---|---|---|
| **附件全链路** | **图像输入是文字会话体验稳了之后的事**。三段全缺（composer 上传 + 内容寻址存储 + 上下文投影），但纯文本编程场景零影响——属于「有了更好，没有不耽误用」 | ~4–6 天 |
| **credentials / authorization / identity** | omenic 现有 TOML `[openai]` + `[[llm.fallbacks]]` 已覆盖单机多渠道；等真的要多用户/多 key 轮换再排 | ~5–7 天 |
| **session telemetry / OTel** | dsh 自己都可禁用，纯可选投影，无生产需求 | ~2–3 天 |
| **token-meter** | C8 已裁定不做；5.7 token 卡「无真数据则隐藏」已兜底 | ~1 天 |

### 已完成批次的余量（不阻塞，可随时捡）

- **MCP**（#381 / #383）：tool-level filter、server-level env 注入未对齐。
- **session resume**（#382 B2a）：checkpoint flush 策略；回放跳过 `System`/`Tool` 行。
- **LLM 路由**（#382 B2b）：全败 Error 不聚合各 provider 原始错误文本；statusline fallback 切换 `model` 显示口径保持 `config.model` 不变。
- **jobs / terminal**（#385）：`onJobDone` 生命周期回调、pwsh 后端、dsh jobs 其余 4 个工具（`jobs_output` 等）。
- **subagent**（#387）：`send_message`/`report`、continuable/background run、session-seeding、SIGTERM 中间层、permission option kind 过滤、Codex / Claude Code / SDK 三个进程外后端。

## 范围外（维持现状）

| 项 | 原因 |
|---|---|
| 元工具重子集：lsp / schedule / hooks / workflow | 体验增益小或与 omenic 模型方向相反（dsh workflow 是模型运行时自写编排，omenic 的 task 是 RPC 任务模型）。轻子集 todo/goal/plan-mode/guard/skill 已按使用价值上提至 P0/P1 |
| core scope + agent-tool-presentation | dsh 作用域隔离与工具结果呈现策略，不与主线耦合 |
| acp 协议 | 协议层已随 #387 落地；只剩 Codex/Claude Code/SDK 三个进程外后端（见余量） |
| C7（tag omenic-harness-v0.1.0 + ferrite 接线） | 前置全满足，等用户拍板时机 |
| C8 interaction + token-meter | 已裁定不做；token-meter 降级 P2（projection-only 可选） |

## 工作约定

- **工作目录**：`.wt/<branch>`（git worktree add 必须在仓库根执行，防嵌套）
- **PR-only**：2026-09-15 裁定只开 PR 不开 issue；缺 `Fixes #` 只是 WARN 不阻塞
- **测试一律不在本地跑**：`cargo test` / `cargo clippy` / 全量 `cargo build` / `npm install` 全部交给 PR 的 CI（`.github/workflows/ci.yml`）。本地只允许 `cargo fmt --check`、单 crate `cargo check -p <crate>`（**不得**加 `--workspace` / `--all-targets`）、grep/ls/git/读写
- **唯一例外**：web UI 需要肉眼确认时的 `cpulimit -l 65 -i -- cargo build --bin oi-web`
- **提交前必跑** `cargo fmt --all`（commit checklist hook 拦不合格的 rust）
- **测试放同层 `tests/`**，不在 src/ 写 `#[cfg(test)]`；重型测试推 CI
- **跨 crate 测试二进制定位**：cargo test 跑在 `target/<profile>/deps`，同 target 的兄弟 bin 用 `std::env::current_exe().parent().parent()` 找（`CARGO_BIN_EXE_*` 只在同 crate 测试可见）
- **改组件源码要同步** `.githooks/spec/` 下对应 UI 契约 yaml 的 `find` 锚点（新 spec 必须登记 `bin/web/tests/ui_contract.rs` 的 `SPEC_FILES`）
- **加依赖必须同步改 `Cargo.lock`**（CI `--locked` 拒绝重算）
- **`.githooks/` 只读**：gate 规则、hook 脚本、spec yaml 下的 gate 规则都不许改（UI 契约 yaml 由 `ui_contract.rs` 消费，不算 gate 规则，可改）。gate FAIL 了改自己的提交/正文去迎合，不要改规则
- **合并**：gate 拦截的 `gh pr merge --squash` + 删分支 + 清工作树；绝不本地 merge 到 main；合并留言以 `Agent 🤖 - Merge:` 开头，PR 正文与 CRG Review comment 的 heading 必须**纯英文**（gate PR-04 / RV-04 拦 CJK）
- **审查双层（B3 起强制）**：合并前 CRG `detect-changes`（0 affected flow 为结构判据）+ ocr 分批（大 diff 按目录分 3 批避免上游 400）；ocr 的稳定误报类（死 fallback `{payload:?}`、已知 ponytail、跨 crate test-gap 名字噪音）直接裁定驳回
- **serde wire 契约**：enum 级 `rename_all` 只重命名变体名，struct-variant 字段需**变体级** `rename_all`；运行时 serde 错误本地 `cargo check` 抓不到，新协议层靠 CI 验证
- **领域依赖方向**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO
- **无 `Co-authored-by` trailer**
- **schema 迁移模式**：照 G6 的 `apply_turn_log_column`——`PRAGMA table_info` 检查 + 缺列才 `ALTER TABLE`，幂等，每次 open 都跑
