#!/usr/bin/env bash
# gh_api.sh — 统一的 GitHub API 调用封装。
# 提供分页、--jq 投影、失败重试，供 validate_*.sh / merge_pr.sh 复用。
# Source 用法：source "$(dirname "${BASH_SOURCE[0]}")/gh_api.sh"

set -euo pipefail

# gh_api_get <endpoint> [jq_expr] — 分页 GET + 重试（网络类错误重试 3 次）
gh_api_get() {
  local endpoint="$1" jq_expr="${2:-}" out rc try
  for try in 1 2 3; do
    if [[ -n "$jq_expr" ]]; then
      out=$(gh api --paginate "$endpoint" --jq "$jq_expr" 2>&1) && rc=0 || rc=$?
    else
      out=$(gh api --paginate "$endpoint" 2>&1) && rc=0 || rc=$?
    fi
    if [[ $rc -ne 0 ]] && [[ "$out" == *"EOF"* || "$out" == *"unexpected EOF"* ]]; then
      sleep $((try * 2)); continue
    fi
    break
  done
  if [[ $rc -ne 0 ]]; then
    printf 'gh_api_get FAIL %s: %s\n' "$endpoint" "$out" >&2
    return $rc
  fi
  printf '%s\n' "$out"
}

# --- issue / PR 基础字段 ---
gh_issue_view()  { # <owner/repo> <issue_number> <jq_expr>
  gh_api_get "repos/$1/issues/$2" "$3"
}
gh_pr_view()  { # <owner/repo> <pr_number> <jq_expr>
  gh_api_get "repos/$1/pulls/$2" "$3"
}

# --- label ---
gh_label_list() { # <owner/repo> → 每行一个 label 名
  gh_api_get "repos/$1/labels?per_page=100" '.[].name'
}

# --- native sub-issue ---
gh_sub_issue_numbers() { # <owner/repo> <parent_number> → 每行一个 child number
  gh_api_get "repos/$1/issues/$2/sub_issues" '.[].number'
}
gh_sub_issue_list() { # <owner/repo> <parent_number> <jq_expr>
  gh_api_get "repos/$1/issues/$2/sub_issues" "$3"
}

# --- PR 评论（会话 / inline）---
gh_pr_issue_comments() { # <owner/repo> <pr_number> <jq_expr> — PR 会话评论
  gh_api_get "repos/$1/issues/$2/comments" "$3"
}
gh_pr_review_comments() { # <owner/repo> <pr_number> <jq_expr> — inline review comments
  gh_api_get "repos/$1/pulls/$2/comments?per_page=100" "$3"
}
gh_pr_checks() { # <owner/repo> <pr_number> <jq_expr> — check runs
  gh_api_get "repos/$1/pulls/$2/checks" "$3"
}

# gh_pr_fixes_issue <owner/repo> <pr_number> → 输出被本 PR Fixes 的 issue number（可能多个，每行一个）
gh_pr_fixes_issue() {
  gh_pr_view "$1" "$2" '.body // ""' | grep -oE 'Fixes #[0-9]+' | grep -oE '[0-9]+'
}