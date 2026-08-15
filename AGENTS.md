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

## Demo 验证沙盒

验证 issue/PR 流程、gh-gate 拦截、规则改动时，**不要在本仓库（omenic）直接创建 demo issue/PR**，使用专用沙盒：

- 仓库：https://github.com/hathawayANdRX105/demo-githooks（本地 `~/projects/demo-githooks`）
- 用途：验证 epic/sub/PR 链路、checkbox 强制、双向关联（GT-04b）、审查强制等，避免污染 omenic
- .githooks 与 omenic 同步；规则改动后先在此仓库验证，再同步到其他项目（deskctl / new-api）

规则文件同步流程：改动 omenic `.githooks/` → 复制到 demo-githooks / deskctl / new-api 并提交。