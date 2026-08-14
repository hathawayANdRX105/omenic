# GitHub Issue/PR 指南

本指南指导 agent 创建、更新、关联 GitHub issue/PR。检查由 `.githooks/` 强制（gh-gate 创建前拦截 + issues.py/pull_requests.py 创建后校验），本文件只讲怎么做。

## 创建前必读

- `.githooks/gh-gate` — 创建入口，先读其头部注释
- `.github/ISSUE_TEMPLATE/` — 选模板：`task.yml`（内部工作）、`feature.yml`（新功能）、`bug.yml`（缺陷）
- `.github/PULL_REQUEST_TEMPLATE.md` — PR 正文结构
- `gh label list` — 确认 label 真实存在于仓库，模板写过但仓库没创建的 label 不能用

## Issue 规则（by spec/github_issues.yaml）

| 检查项 | 规则 | 强制 |
|--------|------|------|
| 必填段 | Goal / Background / Done when / Suspected areas / Out of scope / How to observe success | I-01/I-02 |
| 标题 | 中文（I-05），禁全角括号 `（）「」【】『』《》〈〉`（I-xx） | I-05, I-xx |
| heading | 英文（I-06） | I-06 |
| 正文 | 中文（I-07） | I-07 |
| Done when | 必须 `- [x]` checkbox（I-04），禁 table | I-04 |
| Suspected areas | 非空（I-02b） | I-02b |
| Labels 段 | 正文禁写，用 gh 操作 | I-00 |
| 关键词 | 禁 TODO/TBD/FIXME/XXX（I-xx） | I-xx |
| 关键字建议 | 正文命中关键词时建议对应 label（I-21b，WARN） | I-21b |

### parent（epic）
- 禁 `## Done when`（I-16 → FAIL）
- 不用 required_headings（I-01/I-02 跳过）
- 可写 `## Implementation Order`（I-17，可选）
- 必须有 native sub-issues（I-18 → FAIL 如果没有）

### sub
- 必须 6 个必填段
- 禁 cross-reference：`Depends on:` / `Blocks: / Related # / Parent PR:`（I-11/13/14 → FAIL）
- 禁 PR 占位符：`待补 PR` / `TODO.*PR` / `需 PR` / `PR 关联`（I-12 → FAIL）
- 关闭时 Done when 必须全勾（I-22b）

## PR 规则（by spec/github_pull_requests.yaml）

| 检查项 | 规则 | 强制 |
|--------|------|------|
| 标题 | 英文禁 CJK（P-01 → FAIL），Conventional Commit（P-02 → WARN） | P-01, P-02 |
| 必填段 | Issue / What / Why / Construction plan / Delivery record / How to test / Checklist（P-xx） | P-xx |
| heading | 英文（P-10 → FAIL） | P-10 |
| What 段 | 中文（P-10 → WARN） | P-10 |
| Fixes | 必含 `Fixes #N` 或 `Closes #N` 或 `Resolves #N`（P-12 → WARN） | P-12 |
| 分支 | 前缀 `feat/` `fix/` `chore/` `epic/` `main` `master` `release/`（P-31 → FAIL） | P-31 |
| label | 对照仓库实际 label（P-14/P-20） | P-14 |
| 关键字建议 | 同 issue（P-14b/P-21b，WARN） | P-14b |

## Review 规则（by spec/github_reviews.yaml）

| 类型 | 格式 | 强制 |
|------|------|------|
| CRG Review | `## Agent 🤖 - CRG Review: <title>`（H2）→ 子分类 `###`（H3） | P-35, P-36 |
| Inline Review | `Agent 🤖 - Inline Review P0|P1|P2|P3: <content>` | P-35 |
| Reply | `Agent 🤖 - Fix: <原因>` / `Block:` / `Note:` / `Resolve:` / `Withdraw:` / `Supersede:` | P-24, P-25 |
| 禁 checkbox | review 评论中严禁 `- [x]` / `- [ ]` | P-22 |

## 创建流程（走 gh-gate）

```bash
# issue
python .githooks/gh-gate issue create --title "<中文标题>" --body "<模板正文>" --label <epic|sub|bug|enhancement|chore>

# PR
python .githooks/gh-gate pr create --title "feat(scope): desc" --body "<模板正文>" --head <分支>
```

gate 自动：创建前按 spec 校验（FAIL 拒绝）→ 调 gh 创建 → 创建后跑 issues.py/pull_requests.py 现实校验（FAIL 提示修正）。**禁止绕过**。

## 创建后校验

```bash
python .githooks/github/issues.py <owner/repo> <#N>
python .githooks/github/pull_requests.py <owner/repo> <#N>
python .githooks/merge <owner/repo> <#N> --dry-run
```

必须 `RESULT: ALL PASS` 才算完成。FAIL → 修正后重跑。

## 自检

- [ ] 模板已读，label 已确认存在
- [ ] issue 标题中文，heading 英文，正文中文
- [ ] sub 自包含（无 parent/PR/依赖引用），parent 无 Done when
- [ ] 正文无 Labels 段
- [ ] PR 含 Fixes #N，标题 Conventional Commit
- [ ] 走 gh-gate 创建，创建后校验 ALL PASS