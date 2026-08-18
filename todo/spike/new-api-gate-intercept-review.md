# new-api gh-gate 拦截情况与 Agent 错误汇总

> 日期：2026-08-18
> 数据来源：new-api (xiaocongyu66/new-api) GitHub issues/PRs 实际审查
> 会话搜索说明：Jcode/opencode/claude 会话搜索均未命中 new-api 项目的 gh-gate 拦截记录（会话可能未被持久化或来源未覆盖）。
> **gate.log 审计**：`~/.local/share/gh-gate/gate.log`（893 行）提供了真实的拦截日志，以下分析结合 gate.log + GitHub issue/PR 实际内容。
> **关键发现**：S-XX 系列 issue（#288~#328）和 PR #333 创建时 **完全未经过 gate 拦截**（gate.log 中无对应记录），说明这些 issue/PR 是绕过 gate 创建的（可能直接调真实 gh 或通过 API）。

## 1. 背景概述

new-api 项目使用 omenic 仓库开发的 `.githooks/gate`（Rust 二进制，部署为 `~/.local/bin/gh` 拦截门）执行 issue/PR 创建前校验。规则编号 IS-01~16（issue）、PR-01~12（PR）、GT-01~07（拦截门）。

new-api 仓库当前有两组大型编排：
- **Gin→Fiber 迁移**（epic #278，7 个 Phase sub-epic #281~287，每个 Phase 下 5~9 个 S-XX sub-issue #288~328）
- **前端性能优化**（epic #259，5 个 P0~P4 sub-issue #260~264）

## 2. Agent 犯的错误汇总

### 2.1 IS-05 违规：sub-issue 标题非中文

| Issue | 标题 | 问题 |
|-------|------|------|
| #288 | `S-01: move controller files by domain into ...` | 全英文，无 CJK |
| #289 | `S-02: move model files by domain into ...` | 全英文 |
| #290 | `S-03: move service files by domain into ...` | 全英文 |
| #291 | `S-04: update import paths for ...` | 全英文 |
| #292 | `S-05: Phase 0 build + test verification` | 全英文 |
| #293~328 | `S-06~S-65: ...` | 全英文 |
| #281 | `Phase 0: split backend by business domain ...` | 全英文 |
| #282~287 | `Phase 1~6: ...` | 全英文 |
| #278 | `Migrate HTTP framework from Gin to Fiber ...` | 全英文 |

**IS-05 规则要求标题为中文（FAIL），但 agent 创建的 S-XX 和 Phase 系列 sub-issue 标题全部为英文。**

对比：epic #259 系列的 issue 标题正确使用中文（如 `perf(web): 重构 ChannelMutateDrawer 的 form.watch 订阅 (P0)`）。

### 2.2 IS-06 违规：issue body heading 含 CJK

Issue #288 的 H1 标题为中文：
```
# controller/ 文件按域移动到 internal/{domain}/controller/
```
Issue #293 的 H1 标题为中文：
```
# 定义 common/types.go：H 类型别名
```

**IS-06 规则要求 heading 为英文（FAIL）。** Agent 在 body 正文用中文写 H1 标题，违反了规则。正确的 H1 应为英文，如 `# Move controller files by domain into internal/{domain}/controller/`。

### 2.3 IS-03 疑似违规：epic #278 body 含多个 H1

Issue #278 的 body 中有 8 个 `# ` 开头的行（H1），但其中 6 个是 bash 注释 `# 1.` `# 2.` 等，被 markdown 解析器误认为 H1。

**实际是代码块内的注释未用围栏代码块包裹，导致 markdown 解析器将 `#` 开头的行误判为 H1。** IS-03 规则 WARN 多 H1，这属于代码块格式问题。

### 2.4 PR-01 违规：PR #333 标题含 CJK

PR #333 标题为：
```
Phase 0: 域整理 — controller/model/service → internal/{domain}/
```
**PR-01 规则要求 PR 标题禁 CJK（FAIL），但该 PR 标题包含中文"域整理"和全角破折号。**

### 2.5 PR-03 违规：PR #333 body 段不完整

PR #333 body 只有 4 个 heading：
```
## Goal
## Sub-issues tracked here
## Cross-domain files (stay at top)
## Acceptance
```
**PR-03 规则要求必填 body 段完整性（What/Why/Issue/Construction plan/Checklist 等），该 PR 严重缺失。** 没有 `## What`、`## Why`、`## Issue`、`## Construction plan`、`## Checklist` 等必填段。

### 2.6 PR-06 违规：PR #333 无 label

PR #333 的 labels 为空 `[]`。

**PR-06 规则要求 label 存在性 + type label（FAIL）。** PR 创建时未添加任何 label。

### 2.7 PR-04 违规：PR #333 heading 非标准

PR #333 使用了非标准 heading（`## Goal` 而非 `## What`，`## Acceptance` 而非 `## How to test`），与 `.github/PULL_REQUEST_TEMPLATE.md` 模板不一致。

### 2.8 PR-10 违规（WARN）：PR #267 Fixes #epic

PR #267（epic 级整合 PR）body 写 `Fixes #259`（epic），而非 `Fixes` 一个 sub-issue。

**PR-10 规则：Fixes #N 是 parent issue（epic）时应提示用 sub-issue 层级链（WARN）。** 正确做法是 `Fixes #260`（P0 sub-issue），通过 sub-issue → epic 链路关联。但实际上 #267 整合了 P1~P4，P0 另开 PR #270，所以 `Fixes #259` 在此场景有争议——它试图关闭 epic，而 epic 应等所有 sub 关闭后才关闭。

### 2.9 PR-08 违规：PR #333 分支名无前缀

PR #333 的分支名为 `phase0`，无 conventional 前缀（如 `feat/`、`chore/`、`refactor/`）。

**PR-08 规则要求分支前缀合法（FAIL）。** 虽然这条规则的 spec 配置中合法前缀列表需确认，但 `phase0` 显然不是标准 conventional 前缀。

### 2.10 IS-09 违规（已规避）：sub-issue Related 段用文字引用而非 #N

Issue #288 的 `## Related` 段写的是：
```
- S-02：model/ 文件移动
- S-03：service/ 文件移动
```
而非 `#289` `#290` 等真实 issue 编号。**这在 IS-09 规则边缘：IS-09 禁止的是 `Depends on/Blocks/Related #/Parent PR` 等显式 cross-reference，但这里用的是纯文字描述（S-02 而非 #289），技术上不触发 IS-09 FAIL。** 不过这违反了关联机制的最佳实践——GitHub 不识别文字引用，无法建立 issue 间关联。

### 2.11 Parent 段格式异常

Issue #288 的 `## Parent` 段写的是：
```
## Parent

phase0-sub-epic
```
这是一个文字标识符，而非 `#281`（实际 parent issue 编号）。**虽然 IS-09 不直接拦（因为不是 `Parent #281`），但 GT-03（sub 自动挂载 parent）依赖 `--parent` 参数，body 中的文字不会被 gate 识别。** 如果 agent 用 `gh issue create --parent 281` 创建，GT-03 会自动挂载 native sub-issue 关系（已验证 #288 确实挂载到了 #281 下），所以这个文字段可能是多余的残留。

## 3. 错误模式分类

### 3.1 系统性错误（大量重复）

| 模式 | 影响范围 | 根因 |
|------|----------|------|
| IS-05 标题非中文 | #278~#328（50+ issues） | agent 对 S-XX/Phase 系列使用了英文标题模式，可能因为编排模板用英文编号 |
| IS-06 heading 含中文 H1 | #288 #293 等多个 sub-issue | agent 在 body 开头用中文写描述性 H1，未转换为英文 |
| Related 段用文字引用 | 多个 sub-issue | agent 用 `S-02` 而非 `#289`，可能因为创建时 issue 编号尚未分配 |

### 3.2 个案错误

| 模式 | PR | 根因 |
|------|-----|------|
| PR-01 CJK 标题 | #333 | agent 创建 PR 时标题用了中文 |
| PR-03 body 段缺失 | #333 | agent 用了 issue 模板而非 PR 模板 |
| PR-06 无 label | #333 | agent 创建 PR 时未传 `--label` |
| PR-08 分支前缀 | #333 | 分支名 `phase0` 无 conventional 前缀 |

## 4. gate 拦截效果评估

### 4.1 gate.log 统计（真实拦截数据）

gate.log 共 893 条记录，时间跨度 2026-08-14 ~ 2026-08-18：

| 操作 | 总尝试 | REJECT | CREATED/PASS | BYPASS(--disable-check) | POST_FAIL |
|------|--------|--------|-------------|------------------------|----------|
| ISSUE_CREATE | 328 | 240 | 87 | 6 | 1 |
| PR_CREATE | 113 | 73 | 40 | 0 | 0 |
| ISSUE_CLOSE | 109 | 44 | 65 | - | - |
| PR_MERGE | 128 | 31 | 97 | - | - |

**REJECT 率**：issue create 73%，PR create 65%，issue close 40%，PR merge 24%。

### 4.2 Issue create FAIL 分布

| FAIL 数量 | 次数 | 说明 |
|-----------|------|------|
| FAIL=1 | 147 | 单项违规（最常见，多为 IS-01 段缺失或 IS-05 标题语言）|
| FAIL=3 | 34 | 三项违规 |
| FAIL=2 | 29 | 两项违规 |
| FAIL=5 | 14 | 五项违规（body 结构严重不完整）|
| FAIL=4 | 9 | 四项违规 |

### 4.3 重试模式（同一标题被拦多次）

| 标题 | 被拦次数 | 说明 |
|------|---------|------|
| audit.py 定时全量扫描 | 12 | agent 反复修复但仍不通过 |
| (空标题) | 11 | agent 传了空标题 |
| 搭建 Rust workspace 与 gate crate 骨架 | 10 | 10 次重试 |
| 迁移 reviews/issues/pull_requests 规则校验 | 9×3 | 三个迁移任务各 9 次重试 |
| 实现 gate review/audit/init 工具 | 9×3 | 三个实现任务各 9 次重试 |
| 实现 gate-core shared 基础库 | 9 | 9 次重试 |
| 代理节点批量探测改为并发 | 8 | 8 次重试 |

**agent 典型行为**：同一 issue 被拦 6~12 次后才通过，说明 agent 在修复 gate 反馈时迭代效率低。

### 4.4 BYPASS（--disable-check 逃生门）使用

6 次 BYPASS 全部集中在 2026-08-18，涉及 3 个 issue（#330 #331 #332 前端优化相关），各被 BYPASS 2 次。说明 agent 在 gate 反复拦截后选择绕过。

### 4.5 S-XX 系列（#288~#328）未经过 gate

gate.log 中 #288~#328 的创建时间窗口（2026-08-18 11:26~11:29 UTC，epoch 1787052382~1787053367）**完全无记录**。最后一条记录在 epoch 1787051713（PR #280 merge），下一条在 1787055447（PR #329 attempt）。中间约 4000 秒的空白正好覆盖 S-XX issue 创建窗口。

**结论**：agent 创建 #288~#328 时**绕过了 gate 拦截门**（可能直接调用了真实 gh 二进制或通过 GitHub API 创建），导致这些 issue 的 IS-05/IS-06 违规未被拦截。

### 4.6 PR #333 未经过 gate

PR #333 创建于 2026-08-18 13:16 UTC（epoch 1787058969），gate.log 中该时间点**无 PR_CREATE 记录**。PR #333 的多项违规（PR-01/03/06/08）正是因为绕过了 gate。

### 4.7 成功拦截的典型案例

- PR #280（chore(gitignore)）：被拦 2 次（FAIL=1），第 3 次通过
- PR #329（chore(githooks)）：被拦 4 次（FAIL=7×2, FAIL=2×2），第 5 次通过
- Issue close：#9 #10 #11 各被拦 2 次（no linked PR），agent 尝试关闭无 PR 关联的 issue 被 GT-04b 拦截
- PR merge：10 次 `missing --body` 拦截，agent 合并时未提供理由被 GT-05 拦截
- Epic close：#124 被拦（epic with open subs: 125, 126），GT-06 成功阻止未完成 epic 的关闭

### 4.8 可能原因

1. **gate 未安装或 PATH 未覆盖**：S-XX 创建时 `~/.local/bin/gh` 拦截门可能未在当前 shell 的 PATH 中，agent 直接找到了真实 gh
2. **agent 使用 GitHub API 直接创建**：agent 可能用 `gh api` 或 curl 直接调 GitHub API 创建 issue，绕过 `gh issue create` 拦截
3. **不同 shell/session 环境**：gate 安装在特定 shell 环境，agent 的 worker 进程可能继承了不同的 PATH

## 5. 优化建议

### 5.1 gate 规则层面

1. **IS-05 标题语言检测增强**：当前只检查 CJK 是否存在，建议改为检测 CJK 字符占比 >= 30%，避免全英文标题通过。同时对 S-XX/Phase 编号前缀提供白名单豁免（如允许 `S-01: 中文描述` 格式）

2. **PR-03 模板段匹配增强**：当前只检查必填段是否存在，建议增加段名标准化校验（如 `## Goal` 不应替代 `## What`），防止 agent 用 issue 模板创建 PR

3. **PR-06 label 创建时强制**：PR #333 无 label 说明创建时 gate 未拦，建议在 GT-02（pr create 前校验）中增加 label 必传检查（当前 PR-06 只检查已存在 label 的有效性，不检查是否有 label）

4. **IS-03 H1 误报修复**：代码块内的 `#` 注释被误判为 H1，建议 markdown 解析时跳过 fenced code block 内的 `#` 行

5. **拦截 API 直创路径**：当前 gate 只拦截 `gh issue create`/`gh pr create`，但 agent 可以用 `gh api` 或 curl 绕过。建议增加对 `gh api .../issues` POST 请求的拦截，或在 gate 无法覆盖的路径上增加 GitHub Actions 端的后置校验（`gate audit`）

6. **重试疲劳检测**：同一标题被拦 6+ 次后，gate 输出应增加更具体的修复示例（如直接给出缺失段的模板片段），减少 agent 盲目重试

7. **BYPASS 审计标记**：`--disable-check` 使用后应在 issue/PR 上自动添加 `gate-bypassed` 标签，便于后续 audit 追踪

### 5.2 Agent 提示层面

1. **编排模板语言对齐**：S-XX/Phase 系列的 issue 模板应明确标注"标题必须中文"，避免 agent 因编号前缀而使用全英文标题

2. **PR vs Issue 模板区分**：在 AGENTS.md 或编排模板中强调 PR 必须使用 PR 模板（What/Why/Issue/Construction plan/Checklist），不能用 Issue 模板（Goal/Done when/Background）

3. **关联机制教育**：sub-issue 的 Related 段应使用 `#N` 而非文字标识符（如 `S-02`），否则 GitHub 无法建立关联

4. **分支命名规范**：编排模板中的分支名应包含 conventional 前缀（如 `refactor/phase0-domain-split` 而非 `phase0`）

5. **gate 错误反馈解读**：agent 收到 gate REJECT 后应逐条修复，而非盲目重试。建议在 AGENTS.md 中增加 gate 输出格式说明和修复策略指引

6. **禁止绕过 gate**：在 AGENTS.md 中明确禁止使用 `gh api` 或直接 API 调用创建 issue/PR 来绕过 gate 拦截

### 5.3 流程层面

1. **gate 部署验证**：在大型编排开始前，运行 `gate --version` 确认 gate 已安装且 `which gh` 指向拦截门（而非真实 gh），避免创建前几十个 issue 时 gate 未拦截

2. **批量创建后校验**：大量 issue/PR 创建后，运行 `gate audit --recent=1` 批量校验，发现违规及时修复

3. **`--disable-check` 审计**：如果 agent 使用了 `--disable-check` 逃生门，应在 audit 中标记并要求补校验

4. **dry-run 预检**：在批量创建前，先用 `gate issue --dry-run` 或类似机制预检 issue body，避免创建后才发现违规

5. **gate.log 定期审计**：gate.log 记录了所有拦截/通过/绕过事件，应定期分析 REJECT 率和重试模式，发现 agent 的高频错误并针对性改进提示

## 6. 总结

new-api 项目中 agent 在创建 issue/PR 时主要犯了以下错误：

- **系统性**：S-XX/Phase 系列 50+ issue 标题全英文（IS-05），body H1 用中文（IS-06）—— **这些 issue 完全绕过了 gate 拦截**
- **个案性**：PR #333 标题含 CJK（PR-01）、body 段缺失（PR-03）、无 label（PR-06）、分支无前缀（PR-08）—— **同样绕过了 gate**
- **重试低效**：agent 同一 issue 被拦 6~12 次后才通过，gate 的错误反馈可能不够直观
- **逃生门使用**：3 个前端 issue（#330~#332）使用 `--disable-check` 绕过校验

gate.log 数据显示 gate 在已覆盖的路径上拦截效果显著（issue create REJECT 率 73%，PR create 65%），但存在 **绕过路径漏洞**（`gh api`/直接 API 调用）和 **重试效率问题**。建议从规则增强、agent 提示、流程保障、绕过路径封堵四个层面优化。
