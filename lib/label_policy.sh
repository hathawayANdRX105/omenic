#!/usr/bin/env bash
# label_policy.sh — 解析 .github/label-policy.yml，供 validate_*.sh 复用。
# 依赖：python3 + PyYAML（若不可用或配置缺失，fallback 硬编码默认值，不阻塞）。
# Source 用法：source "$(dirname "${BASH_SOURCE[0]}")/label_policy.sh"

# _lp_file — 定位 .github/label-policy.yml
# 查找顺序：(1) 当前 git 仓库根 (2) 脚本自身的 sibling .github/ (3) 沿 cwd 父目录
_lp_file() {
  local dir script_dir
  # (1) git 仓库根（脚本在仓库内运行时）
  dir="$(git rev-parse --show-toplevel 2>/dev/null || true)"
  [[ -n "$dir" && -f "$dir/.github/label-policy.yml" ]] && { echo "$dir/.github/label-policy.yml"; return 0; }
  # (2) 从脚本自身位置推导：label_policy.sh 在 lib/，配置在 sibling ../.github/
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd)"
  [[ -n "$script_dir" && -f "$script_dir/../.github/label-policy.yml" ]] && { echo "$script_dir/../.github/label-policy.yml"; return 0; }
  # (3) 从 cwd 沿父目录找
  dir="$(pwd)"
  while [[ "$dir" != "/" ]]; do
    if [[ -f "$dir/.github/label-policy.yml" ]]; then
      echo "$dir/.github/label-policy.yml"; return 0
    fi
    dir="$(dirname "$dir")"
  done
  return 1
}

# lp_type_labels — 输出合法 type label（每行一个）
lp_type_labels() {
  local cfg
  cfg=$(_lp_file) || { printf '%s\n' bug enhancement chore docs refactor test; return 0; }
  python3 - "$cfg" <<'PY'
import sys, yaml
try:
    with open(sys.argv[1]) as f:
        d = yaml.safe_load(f) or {}
    for l in d.get('type_labels', []) or []:
        print(l)
except Exception:
    # fallback
    for l in ('bug','enhancement','chore','docs','refactor','test'):
        print(l)
PY
}

# lp_keyword_suggest — 输出 '<label>\t<kw1,kw2,...>'（每行一组）
lp_keyword_suggest() {
  local cfg
  cfg=$(_lp_file) || return 0
  python3 - "$cfg" <<'PY'
import sys, yaml
try:
    with open(sys.argv[1]) as f:
        d = yaml.safe_load(f) or {}
    ks = d.get('keyword_suggest', {}) or {}
    for label, kws in ks.items():
        kws = [str(k) for k in (kws or [])]
        print(f"{label}\t{','.join(kws)}")
except Exception:
    pass
PY
}

# lp_suggest_for <text> — 根据 text 命中的关键字输出建议 label（每行一个，去重排序）
lp_suggest_for() {
  local text="$1"
  [[ -z "$text" ]] && return 0
  while IFS=$'\t' read -r label kws; do
    [[ -z "$label" || -z "$kws" ]] && continue
    local IFS=','
    for kw in $kws; do
      if grep -qiF "$kw" <<<"$text"; then
        printf '%s\n' "$label"
        break
      fi
    done
  done < <(lp_keyword_suggest) | sort -u
}
