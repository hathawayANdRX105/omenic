# omenic PROGRESS（当前进度与规划）

> 已落地的能力域、整合门与 dsh 对照批次（B1–B3）的交付/审查记录，全部在 [ROADMAP.md](ROADMAP.md)（已完成档）。本文只讲**还没做的**和**接下来做什么**。
>
> 编号体系沿用 ROADMAP：`C1–C8` / `R1–R7` / `G1–G8`。

## 当前位置（2026-09-20）

main `3978817`。**G1–G8 全部已过**；dsh 对照三批 B1（#381）/ B2（#382）/ B3（#385 + #387）已全部合入 main，交付内容、审查拦下的真实缺陷与验收证据见 ROADMAP 的「已交付的 dsh 对照批次 B1–B3」节。

- **C7（tag omenic-harness-v0.1.0 + ferrite 接线）**：前置条件全满足，**等用户拍板时机**，不占批次。
- **C8（interaction + token-meter）**：已裁定不做。

**没有排队中的整合点。** 剩下的全是下文未排期缺口——其中只有「附件全链路」是三段全缺的中等缺口，其余为低优先或范围外。

## 未排期缺口

> 2026-09-18 划分的三批已全部完成（记录在 ROADMAP）。以下三项当时裁定不排批次，是 dsh 对照里仅剩的开放域。

| 域 | dsh 现状 | omenic 现状 | 缺口 |
|---|---|---|---|
| **附件全链路** | `AttachmentStore` 抽象（`validateImage`/`saveImage`/`readImage`/`readImageRequest`）+ `LocalAttachmentStore` 内容寻址 + `ui-attachment` 前端 + adapter `resolveAttachments` 注入（`attachment/attachment*/src/index.ts`） | 全仓 grep `attachment`/`image` = 0 命中（仅 `ETXTBSY` 误匹配） | 前端 composer 上传入口 + 内容寻址存储 + 上下文投影三段全缺；web 前端与后端耦合重，单做后端无意义，需整体排期 |
| **session telemetry / OTel** | `SessionTelemetryBackend` 抽象 + OTel SDK 导出（`session/session-telemetry*/src/index.ts`） | grep `session_telemetry`/`opentelemetry` = 0 | 无生产需求，低优先 |
| **session-title / title-llm** | `SessionTitleService`（确定性 fallback + LLM 生成）（`session/session-title*/src/index.ts`） | grep `session-title`/`title-llm` = 0 | 同上 |
| **credentials / authorization / identity** | `CredentialProvider` 抽象（分层 env 解析 + YAML 持久化 + 跨进程锁）+ `AuthorizationService` + `AnonymousUserId`（`credentials/*/src/index.ts`、`identity/*/src/index.ts`） | grep `credentials`/`identity`/`anonymous-user-id` = 0 | 低优先；现有配置走 TOML 文件够用 |

### 已完成批次的余量（不阻塞，可随时捡）

- **MCP**（#381 / #383）：dsh `mcp-client` 其余可选项（tool-level filter、server-level env 注入）未对齐。
- **session resume**（#382 B2a）：checkpoint flush 策略（dsh `session-checkpoint-policy`）；回放跳过 `System`/`Tool` 行。
- **LLM 路由**（#382 B2b）：全败 Error 不聚合各 provider 原始错误文本；statusline fallback 切换 `model` 显示口径保持 `config.model` 不变；DeepSeek/PiAi 官方 adapter 未接（现走 OpenAI 兼容端点）。
- **jobs / terminal**（#385）：`onJobDone` 生命周期回调、pwsh 后端、dsh jobs 其余 4 个工具（`jobs_output` 等）。
- **subagent**（#387）：`send_message`/`report`、continuable/background run、session-seeding、SIGTERM 中间层、permission option kind 过滤、Codex / Claude Code / SDK 三个进程外后端。

## 范围外（维持现状）

| 项 | 原因 |
|---|---|
| 元工具与治理整片（goal/todo/plan-mode/schedule/skill/lsp/hooks/guard/feedback/workflow，约 60 子包） | 复刻差距最大的功能域，不与 C1–C8 主线耦合，独立规划 |
| core scope + agent-tool-presentation | dsh 作用域隔离与工具结果呈现策略，不与主线耦合 |
| acp 协议 + boot/bundle 声明式装配 | omenic `composition` 已对齐 cordis 运行时；声明式层（profile/bundle）留给 C7 或独立路线 |
| C7（tag omenic-harness-v0.1.0 + ferrite 接线） | 前置全满足，等用户拍板时机 |
| C8 interaction + token-meter | 已裁定不做 |

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
