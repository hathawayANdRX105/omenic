#!/usr/bin/env bash
# bin/init-hooks.sh — 安装 git hooks：设置 core.hooksPath 并赋予执行权限。
# 用法：bin/init-hooks.sh   （在仓库根目录运行）

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "not a git repo"; exit 1; }
HOOKS_DIR="$ROOT/.githooks"

if [[ ! -f "$HOOKS_DIR/pre-commit" || ! -f "$HOOKS_DIR/pre-push" ]]; then
  echo "error: missing hooks in $HOOKS_DIR" >&2
  exit 1
fi

chmod +x "$HOOKS_DIR/pre-commit" "$HOOKS_DIR/pre-push"
git config core.hooksPath .githooks

echo "hooks installed: core.hooksPath=$HOOKS_DIR"
git config --get core.hooksPath