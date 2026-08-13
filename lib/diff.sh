#!/usr/bin/env bash
# diff.sh — 本地 diff 卫生检查（P-16/P-17/P-18/P-19）。
# Source 用法：source "$(dirname "${BASH_SOURCE[0]}")/diff.sh"

set -euo pipefail

# diff_check_no_ws_errors — git diff --check（含 staged）无行尾空白/冲突标记
diff_check_no_ws_errors() {
  local out
  out=$(git diff --check 2>&1 || true)
  out+=$(git diff --cached --check 2>&1 || true)
  if [[ -n "$out" ]]; then
    printf '%s\n' "$out" >&2
    return 1
  fi
  return 0
}

# diff_changed_files [--cached] → 每行一个改动文件名
diff_changed_files() {
  local scope="${1:-}"  # --cached 或空
  if [[ "$scope" == "--cached" ]]; then
    git diff --cached --name-only --diff-filter=ACMR
  else
    git diff --name-only --diff-filter=ACMR
  fi
}

# diff_has_secrets [--cached] → stdin 外，检查改动文件内容命中泄密模式。命中 → exit 1
diff_has_secrets() {
  local scope="${1:-}" f content bad=0
  while IFS= read -r f; do
    [[ -f "$f" ]] || continue
    if [[ "$scope" == "--cached" ]]; then
      content=$(git show ":${f}" 2>/dev/null || true)
    else
      content=$(git diff -- "$f" 2>/dev/null || true)
    fi
    if echo "$content" | has_secret_hit; then
      printf 'SECRET HIT: %s\n' "$f" >&2
      bad=1
    fi
  done < <(diff_changed_files "$scope")
  [[ $bad -eq 0 ]]
}

# diff_has_gen_artifacts [--cached] → 改动文件命中生成文件黑名单。命中 → exit 1
diff_has_gen_artifacts() {
  local scope="${1:-}" bad=0
  while IFS= read -r f; do
    if echo "$f" | is_genpath_hit; then
      printf 'GEN ARTIFACT: %s\n' "$f" >&2
      bad=1
    fi
  done < <(diff_changed_files "$scope")
  [[ $bad -eq 0 ]]
}

# diff_whitespace_ratio → 纯空白改动占比（0~1），stdout
# 用 --numstat 与 --numstat -w 对比：纯空白改动在 -w 下消失
diff_whitespace_ratio() {
  local total nt ws
  total=$(git diff --numstat | awk '{s+=$1+$2} END{print s+0}')
  nt=$(git diff -w --numstat | awk '{s+=$1+$2} END{print s+0}')
  # git diff -w 的行数可能反而更大（因忽略空白合并块），用 min 保护
  ws=$(( total - nt ))
  if [[ $total -le 0 ]]; then
    echo "0"
  else
    awk -v a="$ws" -v b="$total" 'BEGIN{ r=a/b; if(r<0) r=0; if(r>1) r=1; printf "%.2f\n", r}'
  fi
}