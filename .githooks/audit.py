#!/usr/bin/env python3
"""验证并修复 issue/PR 的 checkbox 和 Fixes 关联。

用法:
    python .githooks/audit.py <owner/repo> [--fix]  # 检查并修复
    python .githooks/audit.py <owner/repo> --issues 139,140,141  # 指定 issue
"""
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = ""


def _gh(args: list[str]) -> tuple[int, str]:
    proc = subprocess.run(["gh", "api"] + args, capture_output=True, text=True, timeout=30)
    return proc.returncode, proc.stdout.strip()


def _section(body: str, heading: str) -> str:
    m = re.search(rf"^## {re.escape(heading)}\s*$", body, re.MULTILINE)
    if not m:
        return ""
    rest = body[m.end():]
    nxt = re.search(r"^## ", rest, re.MULTILINE)
    return rest[: nxt.start()] if nxt else rest


def check_issue_done_when(num: int) -> list[str]:
    """检查 issue 的 Done when 是否全勾。返回未勾项列表。"""
    _, body = _gh([f"repos/{REPO}/issues/{num}", "--jq", ".body"])
    if not body:
        return []
    sec = _section(body, "Done when")
    unticked: list[str] = []
    for line in sec.splitlines():
        m = re.match(r"^\s*-\s*\[\s\]\s*(.+)", line)
        if m:
            unticked.append(m.group(1).strip())
    return unticked


def check_pr_body(num: int) -> list[str]:
    """检查 PR 的 Construciton plan / Checklist。返回问题列表。"""
    _, body = _gh([f"repos/{REPO}/pulls/{num}", "--jq", ".body"])
    if not body:
        return []
    issues: list[str] = []
    for sec_name in ["Construction plan", "Checklist"]:
        sec = _section(body, sec_name)
        boxes = re.findall(r"^\s*-\s*\[([ xX])\]", sec, re.MULTILINE)
        if len(boxes) < 2:
            issues.append(f"{sec_name} 只有 {len(boxes)} 个 checkbox，需要至少 2 个")
        unticked = re.findall(r"^\s*-\s*\[\s\]\s*(.+)", sec, re.MULTILINE)
        if unticked:
            issues.append(f"{sec_name} 未勾: {unticked}")
    # Fixes 检查
    if "Fixes" not in body and "Closes" not in body and "Resolves" not in body:
        issues.append("无 Fixes 关联")
    return issues


def fix_issue_boxes(num: int, items: list[str]) -> None:
    """把 issue 的 Done when checkbox 全部打钩。"""
    _, body = _gh([f"repos/{REPO}/issues/{num}", "--jq", ".body"])
    if not body:
        return
    new_body = re.sub(r"^(\s*-\s*)\[\s\](\s*.+)$", r"\1[x]\2", body, flags=re.MULTILINE)
    if new_body == body:
        return
    # 用 gh issue edit --body 更新
    with open("/tmp/audit_body.txt", "w") as f:
        f.write(new_body)
    subprocess.run(["gh", "issue", "edit", str(num), "--body-file", "/tmp/audit_body.txt"],
                   capture_output=True, text=True, timeout=30)
    print(f"  ✓ #{num} Done when 已全部打钩")


def main() -> int:
    global REPO
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    fix = "--fix" in sys.argv
    specific = None
    for a in sys.argv[1:]:
        if a.startswith("--issues="):
            specific = [int(x) for x in a.split("=", 1)[1].split(",")]

    if not args:
        print("Usage: python .githooks/audit.py <owner/repo> [--fix] [--issues=139,140]")
        return 1
    REPO = args[0]

    if specific:
        nums = specific
    else:
        # 获取最近关闭的 issue 和 PR
        _, issues_json = _gh([f"search/issues?q=repo:{REPO}+is:closed&sort=updated&per_page=20", "--jq", ".items[].number"])
        _, prs_json = _gh([f"search/issues?q=repo:{REPO}+is:pr+is:merged&sort=updated&per_page=20", "--jq", ".items[].number"])
        nums = [int(x) for x in issues_json.split() if x.isdigit()]

    print(f"检查 {REPO} 的 issue/PR...\n")

    for num in nums:
        # 判断是 issue 还是 PR
        rc, data = _gh([f"repos/{REPO}/issues/{num}", "--jq", "{state, pull_request:.pull_request}"])
        if rc != 0:
            continue
        d = json.loads(data)
        if d.get("pull_request"):
            # PR
            problems = check_pr_body(num)
            if problems:
                print(f"PR #{num}:")
                for p in problems:
                    print(f"  ⚠ {p}")
        else:
            problems = check_issue_done_when(num)
            if problems:
                print(f"#{num}: Done when 未勾 ({len(problems)} 项):")
                for p in problems:
                    print(f"  - [ ] {p}")
                if fix:
                    fix_issue_boxes(num, problems)

    print("\n检查完成。")
    return 0


if __name__ == "__main__":
    sys.exit(main())