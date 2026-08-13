#!/usr/bin/env bash
# validate_pr.sh <owner/repo> <pr_number>
# 校验 PR 是否符合 pr-dev-workflow / github-issue-pr skill 的可脚本化要求（P-01 ~ P-30）。
# 输出每项 PASS/FAIL/WARN，任一 FAIL → 退出码 1。
#
# 依赖：gh、jq、python3。lib 位于脚本同级的 ../lib。
#
# 豁免机制（避免对历史内容误报）：
#   --cutoff=N    仅校验 PR 号 >= N（默认 61，即 #61 起强制新模板）
#   --strict      强制审计模式，忽略 cutoff（用于全量审计历史 PR）
#   环境变量 OMENIC_PR_CUTOFF 也可设置默认 cutoff。

set -euo pipefail

# ---- 解析 flag（插在位置参数之前）----
CUTOFF="${OMENIC_PR_CUTOFF:-61}"
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

REPO="${1:?usage: validate_pr.sh <owner/repo> <pr_number> [--cutoff=N] [--strict]}"
PR="${2:?}"

# ---- 应用 cutoff（豁免历史内容）----
if [[ $STRICT -eq 0 ]] && [[ "$PR" -lt "$CUTOFF" ]]; then
  echo "== PR #$PR: SKIP (below cutoff $CUTOFF; legacy exempt. Use --strict to force.) =="
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/gh_api.sh
source "$SCRIPT_DIR/../lib/gh_api.sh"
# shellcheck source=../lib/regex.sh
source "$SCRIPT_DIR/../lib/regex.sh"
# shellcheck source=../lib/diff.sh
source "$SCRIPT_DIR/../lib/diff.sh"

FAIL=0
IS_GIT_REPO=0
if git rev-parse --is-inside-work-tree &>/dev/null; then IS_GIT_REPO=1; fi

report() { # <PASS|FAIL|WARN> <code> <msg...>
  local level="$1" code="$2"; shift 2
  printf '%-4s %-6s %s\n' "$level" "$code" "$*"
  [[ "$level" == "FAIL" ]] && FAIL=1
  return 0
}

# ---- 拉取 PR 数据 ----
pr_title=$(gh_pr_view "$REPO" "$PR" '.title')
pr_body=$(gh_pr_view "$REPO" "$PR" '.body // ""')
pr_draft=$(gh_pr_view "$REPO" "$PR" '.isDraft')
pr_state=$(gh_pr_view "$REPO" "$PR" '.state')
pr_head_ref=$(gh_pr_view "$REPO" "$PR" '.head.ref')
pr_labels=$(gh_pr_view "$REPO" "$PR" '[.labels[]?.name] | join(",")')

echo "== PR #$PR: $pr_title =="

# ============ Title ============
echo "--- title ---"
if echo "$pr_title" | has_cjk; then
  report FAIL P-01 "PR title contains CJK (title should be English): $pr_title"
else
  report PASS P-01 "title is English"
fi
if echo "$pr_title" | is_conventional_title; then
  report PASS P-02 "conventional commit title"
else
  report WARN P-02 "title not conventional commit (repo template allows natural English): $pr_title"
fi

# ============ PR body structure ============
echo "--- body structure ---"
for h in "What" "Why" "Issue" "Construction plan" "Delivery record" "How to test" "Checklist"; do
  if printf '%s' "$pr_body" | grep -qF "## $h"; then
    report PASS "P-0[structure]" "heading present: $h"
  else
    report FAIL P-xx "missing heading: ## $h"
  fi
done

# P-10 heading 英文 + 正文中文
heading_bad=$(printf '%s' "$pr_body" | heading_lines | has_cjk && echo hit || echo clean)
if [[ "$heading_bad" == "hit" ]]; then
  report FAIL P-10 "headings contain CJK (headings must be English)"
else
  report PASS P-10 "headings are English"
fi
what_section=$(printf '%s' "$pr_body" | awk '/^## What/{f=1;next}/^## /{f=0}f')
if echo "$what_section" | has_cjk; then
  report PASS P-10 "What section has Chinese prose"
else
  report WARN P-10 "What section has no Chinese prose (template requires Chinese)"
fi

# ============ Issue linkage (Fixes / Related) ============
echo "--- issue link ---"
fixes_count=$( { printf '%s' "$pr_body" | grep -oE 'Fixes #[0-9]+' || true; } | sort -u | wc -l )
if [[ "$pr_state" == "OPEN" ]] && [[ $fixes_count -gt 0 ]]; then
  report WARN P-11 "open PR already uses Fixes # (may close issue prematurely)"
else
  report PASS P-11 "no premature Fixes while open (or PR not open)"
fi
if [[ $fixes_count -eq 1 ]]; then
  report PASS P-12 "exactly one Fixes #"
elif [[ $fixes_count -eq 0 ]]; then
  if [[ "$pr_draft" == "true" ]]; then
    report PASS P-12 "draft PR, Fixes may appear at merge authorization"
  else
    report WARN P-12 "no Fixes # yet (needs one primary issue before merge)"
  fi
else
  report WARN P-12 "multiple Fixes # ($fixes_count): one PR should close one issue"
fi
if [[ $fixes_count -le 1 ]]; then
  report PASS P-13 "one primary issue"
else
  report FAIL P-13 "one PR should close one primary issue"
fi

# ============ Labels (driven by .github/label-policy.yml) ============
echo "--- labels ---"
# shellcheck source=../lib/label_policy.sh
source "$SCRIPT_DIR/../lib/label_policy.sh"
valid_labels=$(gh_label_list "$REPO")
type_labels_cfg=$(lp_type_labels)
type_hit=0; missing=0
mapfile -t _pr_lbls < <(printf '%s\n' "$pr_labels" | tr ',' '\n')
for l in "${_pr_lbls[@]}"; do
  l="${l# }"; l="${l% }"; [[ -z "$l" ]] && continue
  if grep -qxF "$l" <<<"$type_labels_cfg"; then type_hit=1; fi
  if ! grep -qxF "$l" <<<"$valid_labels"; then
    printf '  unknown label not in repo: %s\n' "$l" >&2; missing=1
  fi
done
if [[ $type_hit -eq 1 ]]; then
  report PASS P-14 "type label present"
else
  report FAIL P-14 "no type label (expected one of: $(echo "$type_labels_cfg" | paste -sd', ' -))"
fi
if [[ $missing -eq 0 ]]; then report PASS P-15 "labels all exist in repo"; else report FAIL P-15 "some labels do not exist in repo"; fi

# P-14b: 关键字建议（WARN 级辅助）— PR 标题/body 命中关键字但缺对应 label 时提示
current_pr_labels=$(echo "$pr_labels" | tr ',' '\n')
suggested_pr=$(lp_suggest_for "$pr_title
$pr_body")
if [[ -n "$suggested_pr" ]]; then
  missing_pr_sug=""
  while IFS= read -r sug; do
    [[ -z "$sug" ]] && continue
    if ! grep -qxF "$sug" <<<"$current_pr_labels"; then
      missing_pr_sug="$missing_pr_sug $sug"
    fi
  done <<<"$suggested_pr"
  if [[ -n "$missing_pr_sug" ]]; then
    report WARN P-14b "based on content keywords, consider also labeling:$missing_pr_sug"
  else
    report PASS P-14b "content keywords align with assigned labels"
  fi
else
  report PASS P-14b "no keyword suggestions (or policy not configured)"
fi

# ============ Diff hygiene (requires local git repo) ============
echo "--- diff hygiene ---"
if [[ $IS_GIT_REPO -eq 0 ]]; then
  report PASS P-16 "not a git repo here; diff checks skipped"
  report PASS P-17 "not a git repo here; secret scan skipped"
  report PASS P-18 "not a git repo here; gen-artifact scan skipped"
  report PASS P-19 "not a git repo here; whitespace ratio skipped"
else
  if diff_check_no_ws_errors; then report PASS P-16 "no whitespace errors"; else report FAIL P-16 "git diff --check found errors"; fi
  if diff_has_secrets; then report PASS P-17 "no secrets in diff"; else report FAIL P-17 "secrets/hardcoded tokens in diff"; fi
  if diff_has_gen_artifacts; then report PASS P-18 "no generated artifacts in diff"; else report FAIL P-18 "generated/blacklisted files in diff"; fi
  ws_ratio=$(diff_whitespace_ratio)
  if awk -v r="$ws_ratio" 'BEGIN{exit !(r>0.5)}'; then
    report WARN P-19 "large whitespace-only diff ratio ($ws_ratio)"
  else
    report PASS P-19 "whitespace ratio acceptable ($ws_ratio)"
  fi
fi

# ============ Review artifacts ============
echo "--- review artifacts ---"
# P-36: 顶层会话评论必须有至少一条 Agent 🤖 - CRG Review: 前缀的 CRG 审查产物
conv_json=$(gh_pr_issue_comments "$REPO" "$PR" '.[] | .body')
crg_found=0
while IFS= read -r body; do
  [[ -z "$body" ]] && continue
  if printf '%s' "$body" | is_agent_finding_crg; then crg_found=1; break; fi
done <<<"$conv_json"
if [[ $crg_found -eq 1 ]]; then
  report PASS P-36 "PR conversation has CRG review summary"
else
  report FAIL P-36 "no Agent 🤖 - CRG Review: comment in PR conversation"
fi
# P-20: 兼容旧规则（顶层会话有任意 agent prefix 评论）
summary_found=0
while IFS= read -r body; do
  [[ -z "$body" ]] && continue
  if printf '%s' "$body" | is_agent_prefixed; then summary_found=1; break; fi
done <<<"$conv_json"
if [[ $summary_found -ge 1 ]]; then report PASS P-20 "PR conversation has agent comment"; else report WARN P-20 "no agent comment in conversation"; fi

inline_json=$(gh_pr_review_comments "$REPO" "$PR" '[.[] | {id, body, path, line, in_reply_to_id, user: .user.login}]' 2>/dev/null || echo '[]')
# 过滤掉已撤回/测试/演示评论：body 以 [superseded/[withdrawn/[deprecated 开头，或以 demo 开头的视为非 finding
active_inline_json=$(echo "$inline_json" | jq '[.[] | select(.body | startswith("[superseded") or startswith("[withdrawn") or startswith("[deprecated") or startswith("demo ") | not)]')
n_inline=$(echo "$active_inline_json" | jq 'length')
if [[ $n_inline -eq 0 ]]; then
  report PASS P-21 "no inline comments (clean PR)"
  report PASS P-22 "no inline comments to check for noise"
  report PASS P-23 "no inline comments to check prefix"
  report PASS P-35 "no inline comments to check finding format"
else
  n_unanchored=$(echo "$active_inline_json" | jq '[.[] | select(.line == null)] | length')
  if [[ $n_unanchored -eq 0 ]]; then report PASS P-21 "all inline comments anchored (path+line)";
  else report FAIL P-21 "$n_unanchored inline comments lack line anchor"; fi
  # P-22 inline 无确认噪声（删 OK 关键字避免英文误报）
  n_noise=$(echo "$active_inline_json" | jq '[.[] | select(.in_reply_to_id == null and (.body | test("确认无问题|没问题|无问题"))) ] | length')
  if [[ $n_noise -eq 0 ]]; then report PASS P-22 "no clean-confirmation noise inline";
  else report WARN P-22 "inline comments look like clean confirmations (belong in summary)"; fi
  # P-23 任意 agent 前缀（finding 或 reply）
  bad_prefix=$(echo "$active_inline_json" | jq '[.[] | select(.in_reply_to_id == null) | .body | select(startswith("Agent 🤖 - ") | not)] | length')
  if [[ $bad_prefix -eq 0 ]]; then report PASS P-23 "all root inline comments use Agent 🤖 - prefix";
  else report WARN P-23 "$bad_prefix root inline comments lack Agent 🤖 - prefix"; fi
  # P-35（新约定）inline 根评论必须 Agent 🤖 - Inline Review P0/P1/P2/P3: 格式（首行匹配）
  bad_finding_fmt=$(echo "$active_inline_json" | jq '[.[] | select(.in_reply_to_id == null) | .body | split("\n")[0] | select(startswith("Agent 🤖 - Inline Review") | not)] | length')
  if [[ "${bad_finding_fmt:-0}" -eq 0 ]]; then report PASS P-35 "all inline findings use Agent 🤖 - Inline Review Px: format";
  else report FAIL P-35 "$bad_finding_fmt inline findings lack Agent 🤖 - Inline Review Px: format"; fi
fi

# P-38 (WARN): maintainer review (pulls/N/reviews state=COMMENTED/APPROVED/CHANGES_REQUESTED) 存在性
has_maintainer_review=$(gh_api_get "repos/$REPO/pulls/$PR/reviews" '[.[] | select(.state == "COMMENTED" or .state == "APPROVED" or .state == "CHANGES_REQUESTED")] | length' 2>/dev/null || echo 0)
if [[ "${has_maintainer_review:-0}" -ge 1 ]]; then
  report PASS P-38 "maintainer review present"
else
  report WARN P-38 "no maintainer review (COMMENTED/APPROVED/CHANGES_REQUESTED) — human required"
fi

# ============ Inline thread closure ============
echo "--- inline thread closure ---"
# 计算根 finding 数量与各自 reply（仅统计未撤回的 active 评论）
root_count=$(echo "$active_inline_json" | jq '[.[] | select(.in_reply_to_id == null)] | length')
replied_root=0
fix_replied=0
for id in $(echo "$active_inline_json" | jq -r '.[] | select(.in_reply_to_id == null) | .id'); do
  reply_body=$(echo "$active_inline_json" | jq -r --arg id "$id" '[.[] | select(.in_reply_to_id == ($id|tonumber)) | .body] | join("\n")')
  [[ -z "$reply_body" ]] && continue
  if printf '%s' "$reply_body" | is_agent_reply; then
    replied_root=$((replied_root+1))
    # 含 commit SHA 引用的算作 "已修复" 类
    if printf '%s' "$reply_body" | has_commit_sha_ref; then
      fix_replied=$((fix_replied+1))
    fi
  fi
done
if [[ $root_count -eq 0 ]] || [[ $root_count -eq $replied_root ]]; then
  report PASS P-24 "all finding threads resolved ($replied_root/$root_count)"
else
  report FAIL P-24 "unresolved inline threads ($((root_count-replied_root))/$root_count)"
fi
# P-37（新约定）：每条 inline finding 必须有 fix-reply；发现问题不修复是严重 bug
if [[ $root_count -eq 0 ]]; then
  report PASS P-37 "no findings to resolve"
elif [[ $replied_root -eq $root_count ]]; then
  report PASS P-37 "all inline findings have reply ($replied_root/$root_count)"
else
  report FAIL P-37 "findings without reply: $((root_count-replied_root))/$root_count (every finding must get a Fix/Block/Note/... reply)"
fi
# P-25: fix-reply 引用 commit SHA 的强度（仅统计有 reply 的）
if [[ $replied_root -eq 0 ]]; then
  report PASS P-25 "no replies to check"
elif [[ $fix_replied -ge $replied_root ]]; then
  report PASS P-25 "all replies cite commit SHA"
else
  report WARN P-25 "$((replied_root-fix_replied))/$replied_root replies lack commit SHA reference"
fi

# ============ CI checks (WARN-level, not blocking) ============
echo "--- CI (weak) ---"
checks_failed=$(gh_pr_checks "$REPO" "$PR" '[.[] | select(.conclusion == "failure") | .name] | join(",")' 2>/dev/null || echo "")
if [[ -z "$checks_failed" ]]; then
  report PASS P-26 "no failing checks"
else
  report WARN P-26 "failing checks (weak per user): $checks_failed"
  conv_all=$(gh_pr_issue_comments "$REPO" "$PR" '[.[].body] | join("\n")')
  if echo "$conv_all" | grep -qE 'caused_by_pr|unrelated|insufficient_evidence'; then
    report PASS P-27 "CI failures are classified in conversation"
  else
    report WARN P-27 "CI failures lack classification marker in conversation"
  fi
fi

# ============ Checkbox evidence sync ============
echo "--- checkbox sync ---"
cp_section=$(printf '%s' "$pr_body" | awk '/^## Construction plan/{f=1;next}/^## /{f=0}f')
if echo "$cp_section" | has_unchecked_checkbox; then
  report FAIL P-28 "Construction plan has unchecked boxes"
else
  report PASS P-28 "Construction plan fully checkable"
fi
cl_section=$(printf '%s' "$pr_body" | awk '/^## Checklist/{f=1;next}/^## /{f=0}f')
if echo "$cl_section" | has_unchecked_checkbox; then
  report FAIL P-29 "Checklist has unchecked boxes"
else
  report PASS P-29 "Checklist fully checked"
fi

# P-30：issue Done when 对齐（被 Fixes 的 issue）
issue_num=$( { echo "$pr_body" | grep -oE 'Fixes #([0-9]+)' | grep -oE '[0-9]+' || true; } | head -1)
if [[ -n "$issue_num" ]]; then
  issue_body=$(gh_issue_view "$REPO" "$issue_num" '.body // ""')
  done_section=$(printf '%s' "$issue_body" | awk '/^## Done when/{f=1;next}/^## /{f=0}f')
  done_total=$(echo "$done_section" | grep -cE '^ *- *\[' || true)
  done_checked=$(echo "$done_section" | grep -cE '^ *- *\[[xX]\]' || true)
  if [[ $done_total -eq 0 ]]; then
    report WARN P-30 "issue #$issue_num has no Done when boxes to compare"
  elif [[ $done_total -eq $done_checked ]]; then
    report PASS P-30 "issue #$issue_num Done when all checked ($done_checked/$done_total)"
  else
    report FAIL P-30 "issue #$issue_num Done when not fully checked ($done_checked/$done_total) — sub-issue must finish all boxes before its PR merges"
  fi
else
  report PASS P-30 "no Fixes issue found; n/a"
fi

# ============ Branch name ============
echo "--- branch ---"
# 使用 PR API 的 head ref (而非本地 HEAD), 避免在不同 worktree 运行时显示错误的分支名
head_br="$pr_head_ref"
if echo "$head_br" | is_valid_branch; then
  report PASS P-31 "branch name valid: $head_br"
else
  report WARN P-31 "branch name off-standard: $head_br (expect feat|fix|chore/issue-N-...|epic/issue-N-...|main|master|release/...)"
fi

echo "======"
if [[ $FAIL -eq 0 ]]; then
  echo "RESULT: ALL PASS"
  exit 0
else
  echo "RESULT: FAIL"
  exit 1
fi