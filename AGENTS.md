# Agent 行为规范

## Issue/PR 创建

创建 issue/PR 前必须读 `.github/ISSUE_TEMPLATE/` 或 `.github/PULL_REQUEST_TEMPLATE.md`，然后通过已安装 gate 拦截的 `gh` 创建，禁止绕过 gate。

```bash
# 安装/更新拦截门
gate init

# issue(正文按 .github/ISSUE_TEMPLATE/ 下模板)
gh issue create --title "..." --body "..." --label <epic|sub|...>

# PR(正文按 .github/PULL_REQUEST_TEMPLATE.md)
gh pr create --title "..." --body "..." --head <branch>
```

gate 自动做创建前校验(规则在 `.githooks/spec/`)+ 创建后现实校验，FAIL 拒绝创建。

## Demo 验证沙盒

验证 issue/PR 流程、gh-gate 拦截、规则改动时，**不要在本仓库(omenic)直接创建 demo issue/PR**，使用专用沙盒：

- 仓库：https://github.com/hathawayANdRX105/demo-githooks(本地 `~/projects/demo-githooks`)
- 用途：验证 epic/sub/PR 链路、checkbox 强制、双向关联(GT-04b)、审查强制等，避免污染 omenic
- .githooks 与 omenic 同步；规则改动后先在此仓库验证，再同步到其他项目(deskctl / new-api)

规则文件同步流程：改动 omenic `.githooks/` → 复制到 demo-githooks / deskctl / new-api 并提交。

## TUI 验证约定（Ratatui）

agent 验证终端 UI 时**不依赖截图**，用 OMP `ui-validate` skill 的结构化断言。详见项目级 skill `.agent/skills/ui-validation/SKILL.md`：

- 用 `ratatui::backend::TestBackend` 渲染到 buffer，断言 cell 内容
- 每屏一个 `specs/tui/<screen>.yaml` 契约，列出坐标/文字/状态
- 测试放 `crates/tui/tests/*.rs`，3 秒内跑完
- 截图（`cat` 输出/`script` 录制）仅作辅助，失败时附带

禁区：只用截图肉眼判断、不写 tui-spec.yaml 直接 PR。
