# .githooks 规范总览

```
.githooks/
├── hooks/                     # git hooks 入口（core.hooksPath = .githooks/hooks）
│   ├── pre-commit            # bash 包装 → exec gate pre-commit（CM-01/02/03 + workspace + code）
│   ├── pre-push              # bash 包装 → exec gate pre-push（workspace + code）
│   └── merge                 # bash 包装 → exec gate merge（PR + reviews + cleanup + CRG + ocr）
├── spec/                      # 规则配置（改规则只改这里，不改脚本）
│   ├── SPEC_OVERVIEW.md      # 本文件（规范总览）
│   ├── dispatch.yaml         # 钩子→主题映射（哪个钩子跑哪些检查）
│   ├── github_issues.yaml    # Issue 规则（IS-* 检查项）
│   ├── github_pull_requests.yaml  # PR 规则（PR-* 检查项）
│   ├── github_reviews.yaml   # Review 评论格式（RV-* 检查项）
│   ├── code_{rust,go,javascript,typescript,python,bash}.yaml  # Rust code lint 参数
│   ├── workspace_{tree_hygiene,file_placement}.yaml           # Rust workspace 检查参数
│   ├── cleanup_branch_cleanup.yaml                            # CL-01 分支清理参数
│   ├── cleanup_tests_{rust,go,javascript,bash}.yaml          # CL-02 测试代码检查参数
│   └── cleanup_docs_hygiene.yaml                             # CL-03 文档卫生检查参数
├── GITHUB_ISSUE_PR.md         # Issue/PR 创建指南（含关联机制）
├── PR_DEV_WORKFLOW.md         # PR 开发工作流指南（含 CRG + ocr 审查流程）
└── WORKFLOW.md                # 工作隔离规范（.wt/ worktree 分支目录）

```

### 外部工具依赖

- `code-review-graph`（CRG）：结构分析/变更影响检测（`detect-changes --brief --base main`）
- `ocr`（OpenCodeReview CLI）：AI 代码审查（`review --format json --audience agent`）
- `gh`（GitHub CLI）：所有 GitHub API 操作入口
- `upx`：二进制压缩（gate 部署产物，构建期使用）

## 本文档用途

本文档是 gate 全部检查规则的**唯一总览**，供人/agent 对照审查：

- 规则**只**在 `.githooks/spec/*.yaml`（参数）和 Rust 校验器（逻辑）两处，改规则只改 spec
- **新增/修改规则后必须更新本文档**
- 规则编号采用**主题前缀 + 连续编号**（IS/PR/RV/GT/WS/CD/CL/CM）
- commit 标题规则 CM-01/02/03 实现在 `gate pre-commit`，语义与 PR-01/PR-02 对齐

## 主题一：GitHub 规则（IS/PR/RV）

参数在 `spec/github_*.yaml`，逻辑在 gate 二进制。

### Issue 规则（IS-01 ~ IS-16）

- IS-01 必填段完整性（Goal/Background/Done when/Suspected areas/Out of scope/How to observe success）— FAIL
- IS-02 Suspected areas 非空 — WARN
- IS-03 body 聚焦（多 H1 提示）— WARN
- IS-04 Done when 必须 checkbox、禁 table — FAIL
- IS-05 标题中文 — FAIL
- IS-06 heading 英文 — FAIL
- IS-07 正文中文 — FAIL
- IS-08 反引号中的仓库路径存在性 — WARN
- IS-09 sub 禁 cross-reference（Depends on/Blocks/Related #/Parent PR）— FAIL
- IS-10 sub 禁 PR 占位符 — FAIL
- IS-11 parent 禁 Done when — FAIL
- IS-14 type label + keyword label 建议 — WARN
- IS-15 关闭时 Done when 全勾（关闭前检查）— FAIL
- IS-16 标题全角括号、禁词、乱码 — FAIL

### PR 规则（PR-01 ~ PR-12）

- PR-01 标题英文（禁 CJK）— FAIL
- PR-02 Conventional Commit 格式 — WARN
- PR-03 必填 body 段完整性 — FAIL
- PR-04 heading 英文、What 段中文 — FAIL/WARN
- PR-05 一个 PR 一个主 issue（Fixes 数量）— WARN
- PR-06 label 存在性 + type label — FAIL
- PR-07 Construction plan/Checklist 至少 2 个 checkbox — FAIL
- PR-08 分支前缀合法 — FAIL
- PR-10 关联机制：Part of/Related 纯文本不产生关联（INFO）；Fixes #N 是 parent issue（epic）时提示用 sub-issue 层级链（WARN）— INFO/WARN
- PR-11 合并前 PR 内 checkbox 全勾（merge 时检查）— FAIL
- PR-12 合并留言理由（merge 时必须 --body）— FAIL

### Review 规则（RV-01 ~ RV-06）

- RV-01 禁 checkbox — FAIL
- RV-02 reply 用词合法（Fix/Block/Resolve/Note/Withdraw/Supersede）— WARN
- RV-03 reply 详细程度 — WARN
- RV-04 CRG/Inline Review 前缀格式 — FAIL
- RV-05 CRG Review 存在 — FAIL
- RV-06 inline findings 有回复 — WARN

## 主题二：拦截门（GT-01 ~ GT-07）

部署为 `~/.local/bin/gh`（argv[0]==gh 时拦截）。

- GT-01 issue create 前校验（调 IS-*）；支持 `--disable-check` 逃生门；sub mode 被拒且看起来是 epic 时提示加 `--label epic` — FAIL 拒 — 触发：gh issue create
- GT-02 pr create 前校验（调 PR-*）— FAIL 拒 — 触发：gh pr create
- GT-03 sub 自动挂载 parent（addSubIssue POST + verify_mount 重试）— 挂载失败 WARN（issue 已创建不回滚，rc=2）— 触发：gh issue create
- GT-04 issue close 前只查 Done when 段 checkbox 全勾 + 必须 --comment 理由（Implementation Order 进度格不拦）— FAIL 拒 — 触发：gh issue close
- GT-04b issue close 前 PR 关联检查：epic 豁免（完成信号是 GT-06 sub 全关）；非 epic 无关联仅提示不阻塞 — WARN — 触发：gh issue close
- GT-05 pr merge 前 checkbox 全勾 + 关联 Fixes issue Done when 全勾（epic 目标豁免，由 GT-06 保障）+ --body 理由 + squash 标题 CM-01/CM-02 — FAIL 拒 — 触发：gh pr merge
- GT-06 epic close/merge 前所有 sub-issues 已关闭（fail-closed：sub 查询失败也拒绝）— FAIL 拒 — 触发：gh issue close / gh pr merge
- GT-07 merge 后自动在 PR 留言 + 删除本地 head 分支（安全模式）— 行为（无拦截）— 触发：gh pr merge
- RV-07 有文件改动的 PR merge 前必须 CRG + ocr 审查 — FAIL 阻塞 — 触发：gate merge

参数剥离：`gh_args()` 剥 `--parent`/`--repo`/`-R`；`arg_repo()` 提取 `--repo` 值（issue close 从 --repo 或 cwd 取仓库）。

## 主题三：钩子调度（gate pre-commit / pre-push / merge）

- `gate init` 部署：复制二进制到 `~/.local/bin/gate`（+ 同二进制为 `~/.local/bin/gh`）、设置 `core.hooksPath=.githooks/hooks`、写 hook 模板
- pre-commit：CM-01/CM-02/CM-03（commit 标题格式/CJK/与 PR type 一致）+ workspace（WS-*）+ code（CD-*）
- pre-push：workspace + code（cargo 不传 target、ruff 排除 .githooks、file_placement 忽略 .githooks/）
- merge（手动 `gate merge <owner/repo> <pr_number> [--dry-run]`）：PR + reviews + cleanup + RV-07（CRG + ocr）

### Commit 标题规则（CM-01 ~ CM-03）

- CM-01 conventional commit 格式（feat/fix/docs/style/refactor/perf/test/build/ci/chore/revert）— FAIL
- CM-02 禁 CJK（应为英文，同 PR-01）— FAIL
- CM-03 分支有关联 open PR 时 type 与 PR 标题一致（gh api，失败/无 PR 跳过）— FAIL


## 主题四：Code（CD-01 ~ CD-06，Rust code lint dispatcher）

- CD-01 rust：cargo fmt --check（spec/code_rust.yaml）— FAIL（工具缺失 → WARN 跳过）
- CD-02 go：gofmt -l（spec/code_go.yaml）— FAIL（工具缺失 → WARN 跳过）
- CD-03 javascript：eslint（spec/code_javascript.yaml）— FAIL（工具缺失 → WARN 跳过）
- CD-04 typescript：npx tsc --noEmit（spec/code_typescript.yaml）— FAIL（工具缺失 → WARN 跳过）
- CD-05 python：ruff check --no-cache（spec/code_python.yaml）— FAIL（工具缺失 → WARN 跳过）
- CD-06 bash：shellcheck（spec/code_bash.yaml）— FAIL（工具缺失 → WARN 跳过）

多语言 code YAML 由 `tools/code.rs` 的 `LANGUAGES` 触发；工具缺失 → WARN 跳过（优雅降级：仅 rc==127 或完整 "command not found" 短语视为缺失，防 lint 输出干扰）。公共字段：enabled/command/args/fail_severity/paths_include/paths_exclude。

## 主题五：Workspace（WS-01 ~ WS-02，Rust workspace validators）

- WS-01 tree_hygiene：空目录、单文件目录、深度 > max_depth、孤儿目录（spec/workspace_tree_hygiene.yaml）— WARN
- WS-02 file_placement：forbidden_patterns、expected_locations（spec/workspace_file_placement.yaml）— WARN；`os.walk` 目录剪枝（ignore 含 node_modules/.git/ 等子树时跳过遍历，防大仓库卡死）

## 主题六：Cleanup（CL-01 ~ CL-03，Rust cleanup validators）

- CL-01 branch_cleanup：merged/orphan/temp 分支清理，dry-run 默认（gate merge 调用）；配置 `spec/cleanup_branch_cleanup.yaml` — WARN
- CL-02 tests_check：四语言测试命名/断言数/必需 helper（配置 `spec/cleanup_tests_{rust,go,javascript,bash}.yaml`，gate merge 调用）— WARN
- CL-03 docs_hygiene：全角括号/死链/遗留标记（TODO/FIXME/XXX）/空文件/CRLF/尾随空白（配置 `spec/cleanup_docs_hygiene.yaml`，gate merge 调用）— WARN/INFO

## 主题七：本地审查（RV-07，gate review）

`gate review [--post|--post-inline] [--pr N]`：

1. CRG 结构分析：`code-review-graph detect-changes --brief --base main` → 影响文件/风险分
2. ocr AI 审查：`ocr review --format json --audience agent` → findings（path/start_line/severity/category/content）
3. 输出：终端（默认）/ PR conversation（--post）/ Files changed inline（--post-inline）
4. 审查闭环：findings 留言（有行号）→ 修复 → `Agent 🤖 - Fix:` 逐条回复 → RV-06 校验
5. `[ocr]` 前缀的错误/超时字符串不当 findings（ocr_has_findings 排除），空输出视为审查不可信（fail-closed）

## 主题八：每日合规检查

- `.github/workflows/daily_audit.yml`：每天 UTC 0:30 自动跑 `gate audit --recent=1`
- 有 FAIL → 自动建 issue 记录；全部 PASS → 无动作；支持 workflow_dispatch


## 触发式（lazy）规则映射

- gh issue create → IS-01~16、GT-01（含 --disable-check 逃生门）、GT-03
- gh issue close → GT-04、GT-04b、GT-06（epic；--repo/-R 被 gate 剥离并透传）
- gh pr create → PR-01~10、GT-02
- gh pr merge → PR-11、PR-12、GT-05（含 squash 标题 CM-01/CM-02）、GT-06、GT-07、RV-01~06
- gh pr comment → RV-01~06
- git commit → CM-01、CM-02、CM-03、WS-01、WS-02、CD-01~06
- git push → WS-01、WS-02、CD-01~06
- gate merge 手动 → 全量（PR + reviews + cleanup：CL-01~03 + RV-07）
- CI 每日（daily_audit.yml）→ 最近 1 天创建的 issue/PR 全规则

## 更新与校验

- 新增/修改规则：只改 `.githooks/spec/*.yaml` 参数 + 对应校验器逻辑，更新本文档
- gate 改动后：`cargo build --release -p gate` → `upx --best --lzma target/release/gate` → `gate init` 重部署 + `install` 复制为 `~/.local/bin/gh`
- 触发式按上表 lazy 执行，不全局扫描