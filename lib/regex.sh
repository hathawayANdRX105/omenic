#!/usr/bin/env bash
# regex.sh — 语言边界 / 格式 / checkbox / 泄密 等通用判定函数。
# Source 用法：source "$(dirname "${BASH_SOURCE[0]}")/regex.sh"

set -euo pipefail

# --- CJK / 语言边界 ---
has_cjk() { # stdin 含 CJK → exit 0
  grep -P -q '[\x{4e00}-\x{9fff}]'
}

# --- 全角括号检测：任何全角中文括号出现即 FAIL ---
has_fullwidth_brackets() { # stdin 含全角括号 → exit 0
  grep -P -q '[\x{ff08}\x{ff09}\x{300c}\x{300d}\x{300e}\x{300f}\x{3010}\x{3011}\x{300a}\x{300b}\x{3008}\x{3009}\x{3014}\x{3015}\x{ff5b}\x{ff5d}]'
}

# 全角字符通用检查（括号专指更严；此函数作为 WARN 用）
has_fullwidth_any() { # stdin 含任意全角字符 → exit 0
  grep -P -q '[\x{ff00}-\x{ffef}]'
}

# --- checkbox ---
# 仅 ASCII [ ] / [x]，不认全角括号。
has_unchecked_checkbox() { # stdin 含 - [ ] → exit 0
  grep -qE '^ *- *\[ \] ' || grep -qE '^ *- *\[ \]$'
}
has_checked_checkbox() { # stdin 含 - [x]/- [X] → exit 0
  grep -qE '^ *- *\[[xX]\] '
}

# --- heading ---
heading_lines() { # 抽出所有 markdown heading 行
  grep -E '^#{1,6} '
}

# --- Conventional Commit（PR 标题；仓库模板实际不强制前缀，此为正则供 P-01 判定）---
CONVENTIONAL_COMMIT_RE='^(feat|fix|chore|docs|refactor|test|perf|build|ci)(\([a-zA-Z0-9_-]+\))?: .+'
is_conventional_title() { # stdin 标题匹配 → exit 0
  grep -qE "$CONVENTIONAL_COMMIT_RE"
}

# --- 分支名规范 ---
BRANCH_RE='^(feat|fix|chore)/issue-[0-9]+-.+|^(main|master|release/)'
is_valid_branch() { # stdin 分支名匹配 → exit 0
  grep -qE "$BRANCH_RE"
}

# --- secrets / 泄密检测 ---
SECRET_PATTERN='(api[_-]?key|access[_-]?token|client[_-]?secret|BEGIN [A-Z ]*PRIVATE KEY|password[[:space:]]*=|ghp_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16})'
has_secret_hit() { # stdin 命中泄密模式 → exit 0
  grep -qE "$SECRET_PATTERN"
}

# --- 生成文件 / 垃圾路径黑名单 ---
GEN_BLACKLIST='(^|/)(target|node_modules|dist|build|\.cache)/|\.(log|jsonl|pyc|o|class)$|~$|/omenic-data/|\.min\.(js|css)$'
is_genpath_hit() { # stdin 每行一个路径，命中黑名单 → exit 0
  grep -qE "$GEN_BLACKLIST"
}