# Agent 行为规范

## Issue/PR 创建

创建 issue/PR 前必须先读 `.githooks/gh-gate`，然后通过它创建，禁止直接 `gh issue create` / `gh pr create`。

```bash
# issue（正文按 .github/ISSUE_TEMPLATE/ 下模板）
python .githooks/gh-gate issue create --title "..." --body "..." --label <epic|sub|...>

# PR（正文按 .github/PULL_REQUEST_TEMPLATE.md）
python .githooks/gh-gate pr create --title "..." --body "..." --head <branch>
```

gate 自动做创建前校验（规则在 `.githooks/spec/`）+ 创建后调 issues.py / pull_requests.py 现实校验，FAIL 拒绝创建。