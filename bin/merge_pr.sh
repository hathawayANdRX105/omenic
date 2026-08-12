#!/usr/bin/env bash
# merge_pr.sh <owner/repo> <pr_number> [--dry-run]
# 合并唯一入口：串联 validate_issue + validate_pr + P-33（draft）+ P-26/27(弱) + P-34（合并后复查）。
# 全 PASS 才执行 gh pr merge。防止直接 gh pr merge 绕过合规检查。
#
# 用法：
#   bin/merge_pr.sh <owner/repo> <pr_number>          # 实际合并（squash + 删分支）
#   bin/merge_pr.sh <owner/repo> <pr_number> --dry-run  # 只检查不合并

set -euo pipefail

REPO="${1:?usage: merge_pr.sh <owner/repo> <pr_number> [--dry-run]}"
PR="${2:?}"
DRY_RUN="${3:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/gh_api.sh
source "$SCRIPT_DIR/../lib/gh_api.sh"

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
info() { printf '== %s\n' "$*"; }

info "merge_pr.sh $REPO PR #$PR (dry_run=${DRY_RUN:-no})"

# ---- 0. 重读 PR 最新状态 ----
pr_state=$(gh_pr_view "$REPO" "$PR" '.state')
pr_draft=$(gh_pr_view "$REPO" "$PR" '.draft // false')
[[ "$pr_state" == "open" || "$pr_state" == "OPEN" ]] || fail "PR #$PR is not OPEN (state=$pr_state)"
info "PR #$PR state=$pr_state draft=$pr_draft"

# ---- 1. P-33 草稿置 ready（dry-run 跳过） ----
if [[ "$pr_draft" == "true" ]]; then
  if [[ "$DRY_RUN" == "--dry-run" ]]; then
    info "draft PR; would run gh pr ready (skipped in dry-run)"
  else
    info "draft PR; marking ready (gh pr ready)"
    gh pr ready "$PR" --repo "$REPO" >/dev/null || fail "could not mark PR ready"
  fi
fi

# ---- 2. 关联 issue 侧校验 ----
# 从 PR body 的 Fixes 提取主 issue（在每个 issue 校验对象是单个 PR 关闭它的那个）
primary_issue=$(gh_pr_fixes_issue "$REPO" "$PR" | head -1)
if [[ -n "$primary_issue" ]]; then
  info "primary issue: #$primary_issue"
  "$SCRIPT_DIR/validate_issue.sh" "$REPO" "$primary_issue" || fail "issue #$primary_issue validation failed"
else
  info "no primary issue via Fixes; skipping issue validation"
fi

# ---- 3. PR 侧校验 ----
"$SCRIPT_DIR/validate_pr.sh" "$REPO" "$PR" || fail "PR validation failed"

# ---- 4. P-26/P-27 CI 弱检查 ----
checks_failed=$(gh_pr_checks "$REPO" "$PR" '[.[] | select(.conclusion == "failure") | .name] | join(",")' 2>/dev/null || echo "")
if [[ -n "$checks_failed" ]]; then
  info "WARN: failing checks (weak policy): $checks_failed ; verify classification before merging"
fi

# ---- 5. 合并前 final gate：mergeable / review decision（信息性） ----
mergeable=$(gh_pr_view "$REPO" "$PR" '.mergeable')
info "mergeable=$mergeable"

if [[ "$DRY_RUN" == "--dry-run" ]]; then
  info "DRY-RUN: all checks passed; would run: gh pr merge $PR --repo $REPO --squash --delete-branch"
  exit 0
fi

# ---- 6. 执行合并 ----
info "merging PR #$PR (squash, delete branch)"
gh pr merge "$PR" --repo "$REPO" --squash --delete-branch || fail "merge command failed"

# ---- 7. P-34 合并后复查 ----
sleep 3
post_state=$(gh_pr_view "$REPO" "$PR" '.state' 2>/dev/null || echo "")
merge_commit=$(gh_pr_view "$REPO" "$PR" '.merge_commit_sha // empty' 2>/dev/null || echo "")
if [[ "$post_state" != "MERGED" && "$post_state" != "merged" ]]; then
  fail "P-34 post-merge: PR state=$post_state (expected MERGED)"
fi
info "P-34 PR merged: state=$post_state commit=$merge_commit"

if [[ -n "$primary_issue" ]]; then
  issue_state=$(gh_issue_view "$REPO" "$primary_issue" '.state' 2>/dev/null || echo "")
  if [[ "$issue_state" == "CLOSED" ]]; then
    info "P-34 issue #$primary_issue closed by merge"
  else
    info "WARN P-34: issue #$primary_issue not auto-closed (state=$issue_state)"
  fi
fi

echo "RESULT: MERGE OK"