#!/usr/bin/env bash
# validate_issue.sh <owner/repo> <issue_number> [parent|sub]
# 校验 Issue 是否符合 github-issue-pr skill 的可脚本化要求（I-01 ~ I-22）。
# 支持三种模式：
#   validate_issue.sh <repo> <issue> parent   — 校验 parent issue 格式（I-16~I-19）
#   validate_issue.sh <repo> <issue> sub     — 校验 sub-issue 自包含（I-11~I-15）
#   validate_issue.sh <repo> <issue>          — 默认：通用校验（I-01~I-10, I-20~I-22）
#
# 输出 RESULT: ALL PASS 才通过；任一 FAIL → 退出码 1。

set -euo pipefail

REPO="${1:?usage: validate_issue.sh <owner/repo> <issue_number> [parent|sub]}"
NUM="${2:?}"
MODE="${3:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/gh_api.sh
source "$SCRIPT_DIR/../lib/gh_api.sh"
# shellcheck source=../lib/regex.sh
source "$SCRIPT_DIR/../lib/regex.sh"

FAIL=0

report() { # <PASS|FAIL|WARN> <code> <msg...>
  local level="$1" code="$2"; shift 2
  printf '%-4s %-6s %s\n' "$level" "$code" "$*"
  [[ "$level" == "FAIL" ]] && FAIL=1
  return 0
}

issue_title=$(gh_issue_view "$REPO" "$NUM" '.title')
issue_body=$(gh_issue_view "$REPO" "$NUM" '.body // ""')
issue_state=$(gh_issue_view "$REPO" "$NUM" '.state')
issue_labels=$(gh_issue_view "$REPO" "$NUM" '[.labels[]?.name] | join(",")')

echo "== Issue #$NUM ($MODE): $issue_title =="

# ---- I-01/I-02 模板与结构 ----
# parent 模式跳过（parent 有 Implementation Order 无 Done when）；其余从模板动态读取必填 heading
echo "--- structure ---"
if [[ "$MODE" == "parent" ]]; then
  report PASS I-01/I-02 "parent mode: template structure n/a (Implementation Order instead)"
else
  template_path=""
  for t in .github/ISSUE_TEMPLATE/task.yml .github/ISSUE_TEMPLATE/feature.yml .github/ISSUE_TEMPLATE/bug.yml; do
    [[ -f "$t" ]] && { template_path="$t"; break; }
  done
  if [[ -n "$template_path" ]]; then
    # 模板必填字段：label 后跟 required: true 的字段（用 -B12 覆盖 dropdown 定义跨度）
    expected_headings=$(grep -B12 'required: true' "$template_path" | grep -oE 'label: [A-Za-z ]+' | sed 's/label: //' | sort -u || true)
  else
    expected_headings="Goal
Done when
Scope"
  fi
  found_all=1
  while IFS= read -r h; do
    [[ -z "$h" ]] && continue
    if printf '%s' "$issue_body" | grep -qF "## $h" || printf '%s' "$issue_body" | grep -qF "### $h"; then
      :
    else
      printf '  missing template heading: %s\n' "$h" >&2
      found_all=0
    fi
  done <<< "$expected_headings"
  if [[ $found_all -eq 1 ]]; then report PASS I-01/I-02 "all template headings present";
  else report FAIL I-01/I-02 "missing required template headings"; fi
fi

# I-03 body 单一结果：不允许出现多个一级 `# ` 标题（弱检查 WARN，parent 除外）
echo "--- focus ---"
n_h1=$(printf '%s' "$issue_body" | grep -cE '^# [^#]' || true)
if [[ "$MODE" != "parent" ]] && [[ $n_h1 -gt 1 ]]; then
  report WARN I-03 "multiple H1 titles ($n_h1); body should focus one outcome"
else
  report PASS I-03 "body focused (or parent mode)"
fi

# I-04 checkbox 验收，不用表格（parent 模式跳过：parent 无 Done when）
echo "--- acceptance checkboxes ---"
if [[ "$MODE" == "parent" ]]; then
  report PASS I-04 "parent mode: Done when n/a (parent has no acceptance section)"
else
  done_section=$(printf '%s' "$issue_body" | awk '/^## Done when/{f=1;next}/^## /{f=0}f')
  if echo "$done_section" | has_unchecked_checkbox || echo "$done_section" | has_checked_checkbox; then
    report PASS I-04 "Done when uses checkboxes"
  else
    report FAIL I-04 "Done when section lacks checkbox items"
  fi
  if printf '%s' "$done_section" | grep -qE '^\|[- ]+\|[- ]+\|' ; then
    report FAIL I-04 "Done when uses a table (checkboxes required)"
  else
    report PASS I-04 "Done when has no table"
  fi
fi

# ---- I-05~I-08 语言边界 ----
echo "--- language ---"
if echo "$issue_title" | has_cjk; then report PASS I-05 "title is Chinese";
else report FAIL I-05 "title lacks Chinese (repo convention)"; fi

bad_heading=$(printf '%s' "$issue_body" | heading_lines | has_cjk && echo hit || echo clean)
if [[ "$bad_heading" == "clean" ]]; then report PASS I-06 "headings are English only";
else report FAIL I-06 "headings contain CJK (headings must be English)"; fi

if echo "$issue_body" | has_cjk; then report PASS I-07 "body prose is Chinese";
else report FAIL I-07 "body lacks Chinese prose"; fi

# 全角括号检查（用户明确要求）：任何全角中文括号 → FAIL
echo "--- fullwidth brackets ---"
if echo "$issue_body" | has_fullwidth_brackets; then
  report FAIL I-xx "body contains fullwidth (Chinese) brackets; must use ASCII half-width"
else
  report PASS I-xx "no fullwidth brackets in body"
fi
if echo "$issue_title" | has_fullwidth_brackets; then
  report FAIL I-xx "title contains fullwidth brackets"
else
  report PASS I-xx "no fullwidth brackets in title"
fi

# ---- I-09 路径真实性（WARN） ----
echo "--- path realism ---"
if git rev-parse --is-inside-work-tree &>/dev/null; then
  missing=0
  # shellcheck disable=SC2016 # 字面反引号（markdown code span 边界），禁用展开提示
  while IFS= read -r p; do
    p="${p//\`/}"
    [[ -z "$p" ]] && continue
    # 跳过命令/URL/非路径
    if [[ "$p" == http* || "$p" == *"("* || "$p" == *.rs:* ]]; then continue; fi
    if [[ ! -e "$p" ]]; then printf '  nonexistent path in Area/Suspected: %s\n' "$p" >&2; missing=1; fi
  done < <(printf '%s\n' "$issue_body" | grep -oE '\`[^\`]+\`' | sort -u)
  if [[ $missing -eq 0 ]]; then report PASS I-09 "referenced paths exist";
  else report WARN I-09 "some backtick references not found in repo"; fi
else
  report PASS I-09 "not in repo, path check skipped"
fi

# ---- 模式分派：sub / parent ----
if [[ "$MODE" == "sub" ]]; then
  echo "--- sub-issue self-contained ---"
  bad=$(printf '%s\n' "$issue_body" | grep -nE '^(Parent|Depends on|\*\*Parent\*\*|\*\*Depends|Blocks |依赖：|Related #)' || true)
  if [[ -n "$bad" ]]; then report FAIL I-11/13/14 "forbidden cross-references in sub-issue:"; printf '%s\n' "$bad" >&2;
  else report PASS I-11/13/14 "no parent/dep/sibling references"; fi
  pr_ref=$(printf '%s\n' "$issue_body" | grep -nE '(PR #[0-9]+|pull/[0-9]+|待补 PR|TODO.*PR)' || true)
  if [[ -n "$pr_ref" ]]; then report FAIL I-12 "sub-issue has PR references/placeholders"; printf '%s\n' "$pr_ref" >&2;
  else report PASS I-12 "no PR references"; fi

elif [[ "$MODE" == "parent" ]]; then
  echo "--- parent format ---"
  if printf '%s' "$issue_body" | grep -q 'Done when'; then report FAIL I-16 "parent must NOT have Done when section";
  else report PASS I-16 "parent has no Done when"; fi
  if printf '%s' "$issue_body" | grep -q '## Implementation Order'; then report PASS I-17 "parent has Implementation Order";
  else report FAIL I-17 "parent lacks Implementation Order section"; fi
  io_numbers=$(printf '%s' "$issue_body" | awk '/^## Implementation Order/{f=1;next}/^## /{f=0}f' | grep -oE '\(#[0-9]+\)' | tr -d '#()' | sort -un)
  actual=$(gh_sub_issue_numbers "$REPO" "$NUM" | sort -u)
  if [[ -n "$actual" ]]; then
    if [[ "$io_numbers" == "$actual" ]]; then report PASS I-18/I-19 "Implementation Order matches native sub-issues ($(echo "$io_numbers" | tr '\n' ',' | sed 's/,$//'))";
    else report FAIL I-18/I-19 "Implementation Order ($(echo "$io_numbers" | tr '\n' ',' | sed 's/,$//')) vs native sub-issues ($(echo "$actual" | tr '\n' ',' | sed 's/,$//'))"; fi
  else
    report WARN I-19 "no native sub-issues found for parent (relationship not established)"
  fi
  # 检查 column：Implementation Order 应含 child 号+标题
  if [[ -n "$io_numbers" ]]; then report PASS I-18 "Implementation Order lists sub-issue numbers";
  else report WARN I-18 "Implementation Order has no sub-issue numbers"; fi
fi

# ---- I-20/I-21 label ----
echo "--- labels ---"
valid_labels=$(gh_label_list "$REPO")
missing=0
for l in $(echo "$issue_labels" | tr ',' '\n'); do
  [[ -z "$l" ]] && continue
  if ! grep -qxF "$l" <<<"$valid_labels"; then printf '  missing label: %s\n' "$l" >&2; missing=1; fi
done
if [[ $missing -eq 0 ]]; then report PASS I-20 "labels all exist"; else report FAIL I-20 "some labels do not exist in repo"; fi
if echo "$issue_labels" | grep -qE '(bug|enhancement|chore)'; then report PASS I-21 "type label present (bug/enhancement/chore)";
else report FAIL I-21 "no type label"; fi

# ---- I-22 关闭时机 ----
echo "--- closure ---"
if [[ "$issue_state" == "CLOSED" ]]; then
  # 收集 timeline 证据：closed 事件（用户/系统）与 cross-referenced（PR 引用）
  timeline=$(gh_api_get "repos/$REPO/issues/$NUM/timeline" '[.[] | {event, actor: .actor.login, commit_id, pr: .source.issue.number, ref: .source.issue.pull_request.merged_at}]' 2>/dev/null || echo "[]")
  closed_events=$(echo "$timeline" | jq '[.[] | select(.event == "closed")] | length')
  # 简化判定：有 closed 事件即视为有关闭动作；无则告警。语义归因无 ground truth → WARN 不强执 FAIL。
  if [[ $closed_events -gt 0 ]]; then
    report PASS I-22 "issue closed with explicit closed event ($closed_events)"
  else
    report WARN I-22 "issue closed without visible closed event on timeline"
  fi
else
  report PASS I-22 "issue open; closure rule n/a"
fi

echo "======"
if [[ $FAIL -eq 0 ]]; then
  echo "RESULT: ALL PASS"
  exit 0
else
  echo "RESULT: FAIL"
  exit 1
fi