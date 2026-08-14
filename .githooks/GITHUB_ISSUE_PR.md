# GitHub Issue/PR 指南

本指南指导 agent 创建、更新、关联 GitHub issue/PR。
检查由 `.githooks/` 强制（install_gh_gate.py 安装后 ~/.local/bin/gh 自动拦截 + issues.py/pull_requests.py 校验）。
本文件只讲怎么做，规则见 `.githooks/SPEC_OVERVIEW.md`。

## 创建前必读

- `.githooks/install_gh_gate.py --install` — 安装 gh 拦截门（自动创建 `~/.local/bin/gh`）
- 安装后 `gh issue create` / `gh pr create` 自动走校验（禁止绕过）
- `.github/ISSUE_TEMPLATE/` — 选模板：`task.yml` / `feature.yml` / `bug.yml`
- `.github/PULL_REQUEST_TEMPLATE.md` — PR 正文结构
- `gh label list` — 确认 label 真实存在于仓库

## 创建流程

```bash
# 安装拦截门（如有更新）
python .githooks/install_gh_gate.py --install

# 创建 issue（自动走校验，FAIL 拒绝）
gh issue create --title "<中文标题>" --body "<模板正文>" --label <epic|sub|bug|enhancement|chore>

# 创建 PR（自动走校验，FAIL 拒绝）
gh pr create --title "feat(scope): desc" --body "<模板正文>" --head <分支>
```

## 创建后校验

```bash
python .githooks/github/issues.py <owner/repo> <#N>
python .githooks/github/pull_requests.py <owner/repo> <#N>
python .githooks/merge <owner/repo> <#N> --dry-run
```

## 本地审查

```bash
python .githooks/review.py                     # CRG 结构分析 + ocr AI 审查
python .githooks/review.py --post-inline       # 审查结果→PR inline review
```

## 参考

- 规则总览：`.githooks/SPEC_OVERVIEW.md`
- 工作流指南：`.githooks/PR_DEV_WORKFLOW.md`
- 钩子配置：`.githooks/spec/dispatch.yaml`