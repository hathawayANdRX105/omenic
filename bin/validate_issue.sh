#!/usr/bin/env bash
# validate_issue.sh <owner/repo> <issue_number> [parent|sub]
# 校验 Issue 是否符合 github-issue-pr skill 的可脚本化要求（I-01 ~ I-22）。
# 支持三种模式：
#   validate_issue.sh <repo> <issue> parent   — 校验 parent issue 格式（I-16~I-19）
#   validate_issue.sh <repo> <issue> sub     — 校验 sub-issue 自包含（I-11~I-15）
#   validate_issue.sh <repo> <issue>          — 默认：通用校验（I-01~I-10, I-20~I-22）
#
# 输出 RESULT: ALL PASS 才通过；任一 FAIL → 退出码 1。
#
# 豁免机制（避免对历史内容误报）：
#   --cutoff=N    仅校验 issue 号 >= N（默认 55，即 #55 起强制新模板）
#   --strict      强制审计模式，忽略 cutoff（用于全量审计历史 issue）
#   环境变量 OMENIC_ISSUE_CUTOFF 也可设置默认 cutoff。

set -euo pipefail

# ---- 解析 flag（插在位置参数之前）----
CUTOFF="${OMENIC_ISSUE_CUTOFF:-55}"
STRICT=0
NEW_ARGS=()
for a in "$@"; do
  case "$a" in
    --cutoff=*) CUTOFF="${a#*=}";;
    --strict) STRICT=1;;
    *) NEW_ARGS+=("$a");;
  esac
done
set -- "${NEW_ARGS[@]+"${NEW_ARGS[@]}"}"

REPO="${1:?usage: validate_issue.sh <owner/repo> <issue_number> [parent|sub] [--cutoff=N] [--strict]}"
NUM="${2:?}"
MODE="${3:-}"

# ---- 应用 cutoff（豁免历史内容）----
if [[ $STRICT -eq 0 ]] && [[ "$NUM" -lt "$CUTOFF" ]]; then
  echo "== Issue #$NUM: SKIP (below cutoff $CUTOFF; legacy exempt. Use --strict to force.) =="
  exit 0
fi

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

# I-02b: Scope 应描述改动范围（模块/文件/子系统），不应替代 GitHub Development 字段关联 PR
echo "--- scope writing ---"
if [[ "$MODE" != "parent" ]]; then
  scope_section=$(printf '%s' "$issue_body" | awk '/^## Scope/{f=1;next}/^## /{f=0}f')
  if [[ -z "$scope_section" ]]; then
    : # heading 缺失已被 I-01/I-02 覆盖
  elif echo "$scope_section" | grep -qE '(单个 PR|一个 PR|single PR|multiple PRs|一个或多个 PR|此 issue 由.*PR 关闭|一个 PR 关闭)'; then
    report WARN I-02b "Scope describes PR linkage rather than change extent; use GitHub Development field for PR-issue link, describe affected modules/files here"
  else
    report PASS I-02b "Scope describes change extent"
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
  # I-16: parent 必须无 Done when（parent 不直接产出验收，由 sub 承担）
  if printf '%s' "$issue_body" | grep -q '## Done when'; then report FAIL I-16 "parent must NOT have Done when section";
  else report PASS I-16 "parent has no Done when"; fi

  # I-18: parent 必须建立 native sub-issues（GitHub 进度条/页面渲染依赖此关系；
  # 文字版 Implementation Order 不再替代它）
  actual=$(gh_sub_issue_numbers "$REPO" "$NUM" 2>/dev/null | sort -u || echo "")
  if [[ -n "$actual" ]]; then
    report PASS I-18 "parent has native sub-issues ($(echo "$actual" | tr '\n' ',' | sed 's/,$//'))"
  else
    report FAIL I-18 "parent has no native sub-issues (use GitHub sub-issue feature or: gh api -X POST repos/OWNER/REPO/issues/PARENT/sub_issues -F sub_issue_id=DB_ID)"
  fi

  # I-17/I-19: Implementation Order 为可选（仅在有依赖顺序或特殊说明时才写）
  # 若存在则必须与 native sub-issues 一致（避免内容/UI 漂移）
  if printf '%s' "$issue_body" | grep -q '## Implementation Order'; then
    report PASS I-17 "Implementation Order present (optional; use only for dep order or special notes)"
    io_numbers=$(printf '%s' "$issue_body" | awk '/^## Implementation Order/{f=1;next}/^## /{f=0}f' | grep -oE '\(#[0-9]+\)' | tr -d '#()' | sort -un)
    if [[ -n "$io_numbers" ]] && [[ -n "$actual" ]]; then
      if [[ "$io_numbers" == "$actual" ]]; then
        report PASS I-19 "Implementation Order matches native sub-issues"
      else
        report FAIL I-19 "Implementation Order ($(echo "$io_numbers" | tr '\n' ',' | sed 's/,$//')) != native sub-issues ($(echo "$actual" | tr '\n' ',' | sed 's/,$//'))"
      fi
    fi
  else
    report PASS I-17 "no Implementation Order section (optional; native sub-issues list is authoritative)"
  fi
fi

# ---- I-20/I-21 label（从 .github/label-policy.yml 读取） ----
echo "--- labels ---"
# shellcheck source=../lib/label_policy.sh
source "$SCRIPT_DIR/../lib/label_policy.sh"
valid_labels=$(gh_label_list "$REPO")
type_labels_cfg=$(lp_type_labels)

missing=0
for l in $(echo "$issue_labels" | tr ',' '\n'); do
  [[ -z "$l" ]] && continue
  if ! grep -qxF "$l" <<<"$valid_labels"; then printf '  unknown label not in repo: %s\n' "$l" >&2; missing=1; fi
done
if [[ $missing -eq 0 ]]; then report PASS I-20 "labels all exist in repo"; else report FAIL I-20 "some labels do not exist in repo"; fi

# I-21: 至少一个 type label（从配置读取，配置缺失时 fallback）
type_hit=0
for l in $(echo "$issue_labels" | tr ',' '\n'); do
  [[ -z "$l" ]] && continue
  if grep -qxF "$l" <<<"$type_labels_cfg"; then type_hit=1; break; fi
done
if [[ $type_hit -eq 1 ]]; then
  report PASS I-21 "type label present"
else
  report FAIL I-21 "no type label (expected one of: $(echo "$type_labels_cfg" | paste -sd', ' -))"
fi

# I-21b: 关键字建议（WARN 级辅助）— 标题/body 命中关键字但缺对应 label 时提示
current_labels=$(echo "$issue_labels" | tr ',' '\n')
suggested=$(lp_suggest_for "$issue_title
$issue_body")
if [[ -n "$suggested" ]]; then
  missing_suggestions=""
  while IFS= read -r sug; do
    [[ -z "$sug" ]] && continue
    if ! grep -qxF "$sug" <<<"$current_labels"; then
      missing_suggestions="$missing_suggestions $sug"
    fi
  done <<<"$suggested"
  if [[ -n "$missing_suggestions" ]]; then
    report WARN I-21b "based on content keywords, consider also labeling:$missing_suggestions"
  else
    report PASS I-21b "content keywords align with assigned labels"
  fi
else
  report PASS I-21b "no keyword suggestions (or policy not configured)"
fi

# ---- I-22 关闭时机 ----
echo "--- closure ---"
if [[ "$issue_state" == "closed" || "$issue_state" == "CLOSED" ]]; then
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