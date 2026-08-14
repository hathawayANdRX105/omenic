#!/usr/bin/env python3
"""gh gate — 全局 gh wrapper + 创建拦截器。

安装到 ~/.local/bin/gh 后，所有 gh issue create / gh pr create 自动走校验。
规则从项目 .githooks/spec/ + issues.py/pull_requests.py 读取（向上查找 cwd）。

用法:
    python .githooks/install_gh_gate.py --install     # 安装到 ~/.local/bin/gh
    python .githooks/install_gh_gate.py --uninstall   # 卸载
    gh issue create ...                                # 安装后自动拦截
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

INSTALL_DIR = Path.home() / ".local" / "bin"
GATE_NAME = "gh"


# ---------------------------------------------------------------------------
# 安装 / 卸载
# ---------------------------------------------------------------------------

def _install() -> int:
    INSTALL_DIR.mkdir(parents=True, exist_ok=True)
    target = INSTALL_DIR / GATE_NAME
    shutil.copy2(__file__, target)
    target.chmod(0o755)
    print(f"✓ 已安装: {target}")
    print(f"  PATH 前置: export PATH=\"{INSTALL_DIR}:$PATH\"")
    print(f"  验证: hash -r && which gh")
    # 检查 PATH 是否包含 INSTALL_DIR
    if str(INSTALL_DIR) not in os.environ.get("PATH", ""):
        shell = Path.home() / ".zshrc"
        if shell.exists():
            ans = input(f"是否写入 {shell} 的 PATH？[y/N] ")
            if ans.lower().startswith("y"):
                with open(shell, "a") as fh:
                    fh.write(f'\nexport PATH="{INSTALL_DIR}:$PATH"\n')
                print(f"✓ 已写入 {shell}，新终端生效")
    return 0


def _uninstall() -> int:
    target = INSTALL_DIR / GATE_NAME
    if target.exists():
        target.unlink()
        print(f"✓ 已删除: {target}")
    else:
        print(f"未安装: {target}")
    return 0


# ---------------------------------------------------------------------------
# 查找项目 .githooks 和真实 gh
# ---------------------------------------------------------------------------

def _find_project_githooks() -> Path | None:
    """从 cwd 向上查找 .githooks 目录。"""
    d = Path.cwd()
    while True:
        candidate = d / ".githooks"
        if candidate.is_dir():
            return candidate
        parent = d.parent
        if parent == d:
            return None
        d = parent


def _find_real_gh() -> str:
    """找真实 gh 二进制，跳过自身。"""
    self_path = Path(__file__).resolve()
    for p in os.environ.get("PATH", "").split(":"):
        candidate = Path(p) / "gh"
        if candidate.is_file() and candidate.resolve() != self_path:
            try:
                proc = subprocess.run([str(candidate), "--version"],
                                      capture_output=True, text=True, timeout=5)
                if proc.returncode == 0:
                    return str(candidate)
            except Exception:
                continue
    for fallback in ["/usr/bin/gh", "/usr/local/bin/gh", "/bin/gh"]:
        if Path(fallback).is_file():
            return fallback
    return "gh"


# ---------------------------------------------------------------------------
# 拦截逻辑
# ---------------------------------------------------------------------------

def _run_gh(args: list[str]) -> tuple[int, str, str]:
    proc = subprocess.run([_find_real_gh()] + args, capture_output=True, text=True, timeout=60)
    return proc.returncode, proc.stdout, proc.stderr


def _extract(args: list[str]) -> tuple[str, str, list[str], str, str]:
    """Extract title, body, labels, head, parent."""
    title = body = head = parent = ""
    labels: list[str] = []
    for i, a in enumerate(args):
        if a in ("--title", "-t") and i + 1 < len(args):
            title = args[i + 1]
        elif a.startswith("--title="):
            title = a.split("=", 1)[1]
        elif a in ("--body", "-b") and i + 1 < len(args):
            body = args[i + 1]
        elif a.startswith("--body="):
            body = a.split("=", 1)[1]
        elif a in ("--label", "-l") and i + 1 < len(args):
            labels.extend(args[i + 1].split(","))
        elif a.startswith("--label="):
            labels.extend(a.split("=", 1)[1].split(","))
        elif a in ("--head", "-H") and i + 1 < len(args):
            head = args[i + 1]
        elif a.startswith("--head="):
            head = a.split("=", 1)[1]
        elif a in ("--parent", "-P") and i + 1 < len(args):
            parent = args[i + 1].lstrip("#")
        elif a.startswith("--parent="):
            parent = a.split("=", 1)[1].lstrip("#")
    return title, body, labels, head, parent


def _gh_args(args: list[str]) -> list[str]:
    """Strip gh-gate-only flags (--parent) from args."""
    out: list[str] = []
    skip = False
    for a in args:
        if skip:
            skip = False
            continue
        if a in ("--parent", "-P") or a.startswith("--parent="):
            skip = a in ("--parent", "-P")
            continue
        out.append(a)
    return out


def _intercept_issue_create(args: list[str]) -> int:
    """拦截 issue create：调项目 issues.py.check_content → FAIL 拒，否则创建 + 后校验 + 挂载。"""
    title, body, labels, _, parent = _extract(args)
    repo = _derive_repo()

    # 找项目 .githooks
    githooks = _find_project_githooks()
    if githooks is None:
        # 无项目规范，透传
        return _passthrough(["issue", "create"] + args)

    sys.path.insert(0, str(githooks))
    sys.path.insert(0, str(githooks / "github"))
    import issues as issues_mod

    mode = "parent" if "epic" in [x.lower() for x in labels] else "sub"
    findings = issues_mod.check_content(title, body, labels, mode=mode)
    fails = [f for f in findings if f.severity.name == "FAIL"]
    for f in findings:
        print(f"{f.severity.name}\t{f.message}")
    if fails:
        print("闸门: 校验 FAIL，拒绝创建。修正后重试。")
        return 1

    print("闸门: 检查通过，执行 gh ...")
    clean_args = _gh_args(args)
    rc, out, err = _run_gh(["issue", "create"] + clean_args)
    if out: print(out)
    if err: print(err, file=sys.stderr)
    if rc != 0: return rc

    # 创建后校验
    url = out.strip()
    if url.startswith("https://github.com/"):
        num = url.split("/issues/")[1].split("/")[0] if "/issues/" in url else ""
        if num:
            proc = subprocess.run(
                [sys.executable, str(githooks / "github" / "issues.py"), repo, num],
                capture_output=True, text=True, timeout=30,
            )
            if "FAIL" in proc.stdout:
                print(f"FAIL\t#{num} 创建后校验 FAIL，修正后重跑")
            else:
                print(f"INFO\t#{num} 创建后校验 ALL PASS")

        # sub-issue 自动挂载
        if "epic" not in [x.lower() for x in labels] and "/issues/" in url:
            _auto_link_sub(url, repo, githooks, parent)

    return 0


def _intercept_pr_create(args: list[str]) -> int:
    """拦截 PR create：调项目 pull_requests.py.check_content → FAIL 拒。"""
    title, body, labels, head, _ = _extract(args)
    repo = _derive_repo()

    githooks = _find_project_githooks()
    if githooks is None:
        return _passthrough(["pr", "create"] + args)

    sys.path.insert(0, str(githooks))
    sys.path.insert(0, str(githooks / "github"))
    import pull_requests as pr_mod

    findings = pr_mod.check_content(title, body, labels, head_ref=head, state="open", draft=False)
    fails = [f for f in findings if f.severity.name == "FAIL"]
    for f in findings:
        print(f"{f.severity.name}\t{f.message}")
    if fails:
        print("闸门: 校验 FAIL，拒绝创建。修正后重试。")
        return 1

    print("闸门: 检查通过，执行 gh ...")
    rc, out, err = _run_gh(["pr", "create"] + args)
    if out: print(out)
    if err: print(err, file=sys.stderr)
    if rc != 0: return rc

    url = out.strip()
    if url.startswith("https://github.com/") and "/pull/" in url:
        num = url.split("/pull/")[1].split("/")[0]
        if num and repo:
            proc = subprocess.run(
                [sys.executable, str(githooks / "github" / "pull_requests.py"), repo, num],
                capture_output=True, text=True, timeout=30,
            )
            if "FAIL" in proc.stdout:
                print(f"FAIL\tPR #{num} 创建后校验 FAIL，修正后重跑")
            else:
                print(f"INFO\tPR #{num} 创建后校验 ALL PASS")
    return 0


def _auto_link_sub(url: str, repo: str, githooks: Path, parent_arg: str) -> None:
    """创建后自动挂载 sub-issue 到 parent。"""
    from _shared import load_yaml
    cfg = load_yaml(githooks / "spec" / "github_issues.yaml")
    if not cfg.get("sub_issue_must_link_parent", False):
        return
    parent = parent_arg or str(cfg.get("default_parent_issue", 0))
    if not parent or parent == "0":
        return
    sub_num = url.split("/issues/")[1].split("/")[0]
    try:
        _, sub_id_raw, _ = _run_gh(["api", f"repos/{repo}/issues/{sub_num}", "--jq", ".id"])
        sub_id = json.loads(sub_id_raw)
        rc2, out2, _ = _run_gh(["api", f"repos/{repo}/issues/{parent}/sub_issues",
                                 "-X", "POST", "-F", f"sub_issue_id={sub_id}"])
        if rc2 == 0:
            print(f"INFO\t#{sub_num} 已挂载到 parent #{parent}")
        else:
            print(f"WARN\t挂载 #{sub_num} → parent #{parent}: {out2.strip()}")
    except Exception as e:
        print(f"WARN\t挂载失败: {e}")


def _derive_repo() -> str:
    try:
        out = subprocess.run(
            ["git", "remote", "get-url", "origin"],
            capture_output=True, text=True, timeout=5,
        ).stdout.strip()
        for prefix in ("https://github.com/", "git@github.com:", "ssh://git@github.com/"):
            if out.startswith(prefix):
                return out[len(prefix):].removesuffix(".git")
    except Exception:
        pass
    return ""


def _passthrough(args: list[str]) -> int:
    """透传所有参数到真实 gh。"""
    rc, out, err = _run_gh(args)
    if out: sys.stdout.write(out + "\n")
    if err: sys.stderr.write(err + "\n")
    return rc


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def _intercept_issue_close(args: list[str]) -> int:
    """拦截 issue close：必须带 --comment 说明关闭原因。"""
    has_comment = any(a.startswith("--comment") or a == "-c" for a in args)
    if not has_comment:
        print("闸门: gh issue close 必须带 --comment 说明关闭原因，例如：")
        print('  gh issue close <N> --comment "Agent 🤖 - Note: 原因说明"')
        return 1
    rc, out, err = _run_gh(["issue", "close"] + args)
    if out: print(out)
    if err: print(err, file=sys.stderr)
    return rc


def _intercept_pr_merge(args: list[str]) -> int:
    """拦截 pr merge：必须带 --body 说明合并原因。"""
    has_body = any(a.startswith("--body") or a == "-b" for a in args)
    if not has_body:
        print("闸门: gh pr merge 必须带 --body 说明合并原因，例如：")
        print('  gh pr merge <N> --squash --body "Agent 🤖 - Merge: 原因说明"')
        return 1
    rc, out, err = _run_gh(["pr", "merge"] + args)
    if out: print(out)
    if err: print(err, file=sys.stderr)
    return rc


def main() -> int:
    if len(sys.argv) >= 2 and sys.argv[1] == "--install":
        return _install()
    if len(sys.argv) >= 2 and sys.argv[1] == "--uninstall":
        return _uninstall()

    if len(sys.argv) >= 3:
        cmd = sys.argv[1:3]
        rest = sys.argv[3:]
        if cmd == ["issue", "create"]:
            return _intercept_issue_create(rest)
        if cmd == ["issue", "close"]:
            return _intercept_issue_close(rest)
        if cmd == ["pr", "create"]:
            return _intercept_pr_create(rest)
        if cmd == ["pr", "merge"]:
            return _intercept_pr_merge(rest)

    return _passthrough(sys.argv[1:])

if __name__ == "__main__":
    sys.exit(main())
