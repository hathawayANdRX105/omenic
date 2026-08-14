# .githooks 规范总览

```
.githooks/
├── gh-gate                    # 创建拦截门（创建前调 check_content + 创建后调 issues.py/pull_requests.py）
├── pre-commit                 # git commit 钩子（轻量：workspace + code）
├── pre-push                   # git push 钩子（全量：workspace + code + PR + reviews + cleanup）
├── merge                      # 合并入口（手动跑：pull_requests + reviews + cleanup）
├── _shared.py                 # 共享模块（gh_api / Finding / load_yaml / run_external）
├── spec/                      # 规则配置（改规则只改这里，不改 .py）
│   ├── dispatch.yaml          # 钩子→主题映射（哪个钩子跑哪些检查）
│   ├── github_issues.yaml     # Issue 规则（I-* 检查项）
│   ├── github_pull_requests.yaml  # PR 规则（P-* 检查项）
│   ├── github_reviews.yaml    # Review 评论格式（P-22/P-24/P-25/P-35/P-36/P-37）
│   ├── code_{rust,go,javascript,typescript,python,bash}.yaml  # 六语言 lint
│   ├── workspace_{tree_hygiene,file_placement}.yaml           # 工作区检查
│   ├── cleanup_branch_cleanup.yaml  # 分支清理
│   ├── cleanup_tests_{rust,go,javascript,bash}.yaml           # 测试代码检查
│   └── cleanup_docs_hygiene.yaml    # 文档清洁
├── github/                    # GitHub 校验器（规则实现，含 check_content 供 gh-gate 复用）
│   ├── issues.py            # Issue 校验（I-01 ~ I-30）
│   ├── pull_requests.py     # PR 校验（P-01 ~ P-39，非 review 部分）
│   └── reviews.py           # Review 校验（P-22/P-24/P-25/P-35/P-36/P-37）
├── code/lint.py               # 六语言 lint 分发器（读 code_*.yaml）
├── workspace/                 # 工作区检查
│   ├── tree_hygiene.py      # 目录整洁（空目录/深度/单文件/孤儿）
│   └── file_placement.py    # 文件位置合理性
├── cleanup/                   # 清理检查
│   ├── branch_cleanup.py    # 分支清理（merged/orphan/temp）
│   ├── tests_check.py       # 测试代码检查（命名/断言/helper）
│   └── docs_hygiene.py      # 文档清洁（全角括号/死链/空文件/CRLF）
├── tests/                     # 单元测试（102 个）
├── SKILL_GITHUB_ISSUE_PR.md   # Issue/PR 创建指南
├── SKILL_PR_DEV_WORKFLOW.md   # PR 开发工作流指南
└── OVERVIEW.md                # 本文件（规范总览）

项目根：
├── AGENTS.md                  # Agent 行为规范（创建前读 gh-gate）
├── pytest.ini                 # pytest 配置（禁用缓存）
└── ruff.toml                  # ruff 配置（缓存重定向到 /tmp）
```

## 主题一：GitHub（github/）

### Issue 规则（spec/github_issues.yaml → github/issues.py）

- ✅ I-01/I-02 必填段完整性（check_content 实现，run 触发）— FAIL
- ✅ I-02b Suspected areas 非空（check_content）— WARN
- ✅ I-03 body 聚焦（check_content）— INFO/WARN
- ✅ I-04 Done when 必须 checkbox、禁 table（check_content）— FAIL
- ✅ I-05 标题中文（check_content）— FAIL
- ✅ I-06 heading 英文（check_content）— FAIL
- ✅ I-07 正文中文（check_content）— FAIL
- ✅ I-09 反引号路径存在性（check_content）— WARN
- ✅ I-11/13/14 sub 禁 cross-reference（check_content）— FAIL
- ✅ I-12 sub 禁 PR 占位符（check_content）— FAIL
- ✅ I-16 parent 禁 Done when（check_content）— FAIL
- ✅ I-17 Implementation Order 可选（check_content）— INFO
- ✅ I-18 parent 必须有 native sub-issues（run API，调 sub_issues）— FAIL
- ✅ I-19 Implementation Order 与 native sub-issues 一致（run API）— FAIL
- ✅ I-20 label 必须存在于仓库（run API，调 /labels）— FAIL
- ✅ I-21 type label 存在（check_content）— FAIL
- ✅ I-21b 关键字→label 建议（check_content）— WARN
- ✅ I-22 关闭事件（check_content）— INFO
- ✅ I-22b 关闭时 Done when 全勾（check_content）— FAIL
- ✅ I-xx 全角括号、禁词（check_content）— FAIL
- ✅ I-xx 标题全角括号（check_content，spec.forbidden_brackets_in_title）— FAIL
- ✅ I-00 标题禁用前缀（check_content，spec.title_forbidden_prefixes）— FAIL
- ✅ I-00 正文禁 Labels 段（check_content，spec.labels_section_forbidden）— FAIL
- ✅ I-30 正文含字面 `\n`/`\r` 或 U+FFFD（check_content，spec.garbled_content_check）— FAIL
- ✅ sub-issue 自动挂载 parent（gh-gate 创建后执行，spec.sub_issue_must_link_parent）

### PR 规则（spec/github_pull_requests.yaml → github/pull_requests.py）

- ✅ P-01 标题英文（check_content）— FAIL
- ✅ P-02 Conventional Commit 格式（check_content）— WARN
- ✅ P-10 heading 英文、What 段中文（check_content）— FAIL/WARN
- ✅ P-11 open PR 提前用 Fixes（check_content）— WARN
- ✅ P-12 Fixes #N 存在性（check_content）— WARN
- ✅ P-13 一个 PR 一个主 issue（check_content）— FAIL
- ✅ P-14/P-20 label 存在性（run API，调 /labels）— FAIL
- ✅ P-14b/P-21b 关键字→label 建议（check_content）— WARN
- ✅ P-31 分支前缀合法（check_content）— FAIL
- ✅ P-38 维护者审查（check_content）— WARN
- ⚠️ P-39 closing reference（run API，简化版只查 Fixes，#126 范围）— INFO
- ✅ P-xx 必填 body 段完整性（check_content）— FAIL

### Review 规则（spec/github_reviews.yaml → github/reviews.py）

- ✅ P-22 禁 checkbox（reviews.py run）— FAIL
- ✅ P-24 reply 用词合法（reviews.py run）— WARN
- ✅ P-25 reply 详细程度（reviews.py run）— WARN
- ✅ P-35 CRG/Inline Review 前缀格式（reviews.py run）— FAIL
- ✅ P-36 CRG Review 存在（reviews.py run）— FAIL
- ✅ P-37 inline findings 有回复（reviews.py run）— WARN

## 主题二：Code（code/lint.py + code_*.yaml）

分发器读 `spec/code_<lang>.yaml`，逐语言跑外部工具；工具缺失 → WARN 跳过（优雅降级）。

| 语言 | 配置位置 | 工具命令 | fail_severity |
|------|---------|---------|--------------|
| rust | spec/code_rust.yaml → code/lint.py run_lang | cargo fmt --check | FAIL |
| go | spec/code_go.yaml → code/lint.py run_lang | gofmt -l | FAIL |
| javascript | spec/code_javascript.yaml → code/lint.py run_lang | eslint | FAIL |
| typescript | spec/code_typescript.yaml → code/lint.py run_lang | npx tsc --noEmit | FAIL |
| python | spec/code_python.yaml → code/lint.py run_lang | ruff check --no-cache | FAIL |
| bash | spec/code_bash.yaml → code/lint.py run_lang | shellcheck | FAIL |

公共字段（每个 code_*.yaml）：
- enabled（true/false）— 是否启用该语言
- command / args — 外部工具调用
- fail_severity（FAIL/WARN）— 失败等级
- paths_include / paths_exclude — 扫描范围

## 主题三：Workspace（workspace/ + workspace_*.yaml）

### tree_hygiene（spec/workspace_tree_hygiene.yaml）

- ✅ 空目录（workspace/tree_hygiene.py run）— WARN
- ✅ 单文件目录（tree_hygiene.py run）— WARN
- ✅ 深度 > max_depth（tree_hygiene.py run）— WARN
- ✅ 孤儿/残留目录（tree_hygiene.py run）— WARN
- ✅ ignore_paths（.wt/、node_modules/、target/、__pycache__/、.git/）

### file_placement（spec/workspace_file_placement.yaml）

- ✅ forbidden_patterns（workspace/file_placement.py run）— WARN
- ✅ expected_locations（file_placement.py run）
- ✅ ignore_paths

## 主题四：Cleanup（cleanup/ + cleanup_*.yaml）

### branch_cleanup（spec/cleanup_branch_cleanup.yaml）

- ✅ 本地已 merged 到 main 的分支（cleanup/branch_cleanup.py run）— WARN
- ✅ 远端已 merged 且与 main 一致的分支（branch_cleanup.py run）— WARN
- ✅ 无远端追踪的孤儿本地分支（branch_cleanup.py run）— WARN
- ✅ 临时分支（branch_cleanup.py run）— WARN
- ✅ protected_branches（main/master/dev/release/*）不删
- ✅ 当前分支不删
- ✅ 默认 dry-run，--apply 才真删

### tests_check（spec/cleanup_tests_{rust,go,javascript,bash}.yaml）

- ✅ rust：命名/断言（cleanup/tests_check.py check_file）— WARN
- ✅ go：命名/断言（tests_check.py check_file）— WARN
- ✅ javascript：命名/断言/helper（tests_check.py check_file）— WARN
- ✅ bash：命名/断言（tests_check.py check_file）— WARN
- 公共字段：enabled、naming_pattern、min_assertions_per_test、assert_patterns、required_helpers、paths_include/exclude

### docs_hygiene（spec/cleanup_docs_hygiene.yaml）

- ✅ 全角括号（cleanup/docs_hygiene.py check_file）— WARN
- ✅ 死链（docs_hygiene.py check_file）— WARN
- ✅ 遗留标记（docs_hygiene.py check_file）— WARN
- ✅ 空文件/占位文件（docs_hygiene.py check_file）— WARN
- ✅ CRLF 行尾（docs_hygiene.py check_file）— WARN
- ✅ 行尾空白 / 缺尾换行（docs_hygiene.py check_file）— INFO
- 可关：broken_link_check、check_fullwidth、cn_en_space_required

## 主题五：钩子调度（spec/dispatch.yaml）

- ✅ pre-commit `git commit` — workspace + code（轻量，不调 API）
- ✅ pre-push `git push` — workspace + code + PR 校验 + reviews + cleanup（全量）
- ✅ merge 手动 `python .githooks/merge` — PR 校验 + reviews + cleanup + squash 计划

## 架构说明

- **规则只在两处**：`.githooks/spec/*.yaml`（参数）+ `.githooks/{github,code,workspace,cleanup}/*.py`（逻辑）
- **gh-gate 不是主题，是执行器**：不含任何规则。创建前调 issues.check_content / pull_requests.check_content（规则从 spec 读），创建后调 issues.py / pull_requests.py（现实校验），FAIL 即拒。主题只有 github / code / workspace / cleanup 四个
- **check_content 是纯函数**：不调 API，供 gh-gate 创建前用；run() 调 check_content + API 专属检查
- **改规则**：只改对应 spec yaml，校验器从 spec 读，gh-gate 不用改
