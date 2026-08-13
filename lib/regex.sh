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

# --- Agent review 前缀约定（新约定，2026-08）---
# Finding 类（约定固定词）：
#   Inline Review  — 行级审查，绑 path+line；可带 severity P0/P1/P2/P3
#     格式: Agent 🤖 - Inline Review P0: <内容>
#   CRG Review     — 图级别审查，放 PR 顶层会话评论
#     格式: Agent 🤖 - CRG Review: <内容>
# Reply 类（任意英文意图词 + 理由）：
#   Fix / Block / Resolve / Note / Wontfix / <任何英文词>: <理由>
#   关键是表明意图并说明理由；不限定词表。

# stdin（一段评论 body）以 Inline Review 前缀开头 → exit 0
is_agent_finding_inline() {
  grep -qE '^Agent 🤖 - Inline Review( P[0-3])?:'
}

# stdin 以 CRG Review 前缀开头 → exit 0
is_agent_finding_crg() {
  grep -qE '^Agent 🤖 - CRG Review:'
}

# stdin 以任意 agent reply 前缀开头（含 Fix/Block/Resolve/Note 等，但排除 Inline/CRG Review）→ exit 0
is_agent_reply() {
  # 用两条 grep 实现 "agent 前缀 且 不是 finding" 等价于 lookahead
  grep -qE '^Agent 🤖 - [A-Z][A-Za-z]*( P[0-3])?:' \
    && ! grep -qE '^Agent 🤖 - (Inline Review( P[0-3])?|CRG Review):'
}

# stdin 以任意 agent 前缀开头（finding 或 reply）→ exit 0
is_agent_prefixed() {
  grep -qE '^Agent 🤖 - [A-Z][A-Za-z]*( P[0-3])?:'
}

# stdin 含 commit SHA 引用（独立 7-40 位 hex 或带 sha=/commit 上下文）→ exit 0
has_commit_sha_ref() {
  grep -qE '(^|[^0-9a-fA-F])[0-9a-f]{7,40}([^0-9a-fA-F]|$)'
}