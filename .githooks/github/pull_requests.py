#!/usr/bin/env python3
"""Validate PR content against .githooks/spec/github_pull_requests.yaml.

Replaces bin/validate_pr.sh (non-review P-* rules).
"""

from __future__ import annotations



import sys as _sys
_sys.dont_write_bytecode = True  # 不生成 __pycache__
import re
import sys
from pathlib import Path
from typing import Any, Optional

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / ".."))

from _shared import (  # noqa: E402
    Finding,
    Severity,
    aggregate_result,
    gh_api_get,
    load_yaml,
    print_findings,
)

CUTOFF = 55

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

CJK_RE = re.compile(r"[\u4e00-\u9fff]")
FULLWIDTH_BRACKETS = "（）「」【】『』《》〈〉《》﹁﹂"
H1_RE = re.compile(r"^# [^#]", re.MULTILINE)
HEADING_RE = re.compile(r"^#{1,6} ", re.MULTILINE)
CHECKBOX_RE = re.compile(r"^\s*-\s*\[([ xX])\]", re.MULTILINE)
TABLE_RE = re.compile(r"^\|[- ]+\|", re.MULTILINE)

TICKED = ("x", "X")


def _has_cjk(s: str) -> bool:
    return bool(CJK_RE.search(s))


def _section(body: str, heading: str) -> str:
    m = re.search(rf"^## {re.escape(heading)}\s*$", body, re.MULTILINE)
    if not m:
        return ""
    rest = body[m.end():]
    nxt = re.search(r"^## ", rest, re.MULTILINE)
    return rest[: nxt.start()] if nxt else rest


def _headings(body: str) -> list[str]:
    return [
        line.strip().lstrip("#").strip()
        for line in body.splitlines()
        if HEADING_RE.match(line)
    ]


def _explicit_bool(v: Any, default: bool) -> bool:
    if isinstance(v, bool):
        return v
    if isinstance(v, str):
        return v.strip().lower() in ("true", "1", "yes")
    return default


# ---------------------------------------------------------------------------
# rule registry
# ---------------------------------------------------------------------------

def _load_config() -> dict[str, Any]:
    cfg = load_yaml(ROOT / "spec" / "github_pull_requests.yaml")
    cfg.setdefault("required_body_headings", ["Issue", "What", "Why"])
    cfg.setdefault("required_title_headings", ["Goal", "Background"])
    cfg.setdefault("forbidden_brackets_in_title", list(FULLWIDTH_BRACKETS))
    cfg.setdefault("forbidden_keywords", [])
    cfg.setdefault("ci_check_mode", "WARN")
    cfg.setdefault("done_when_check_mode", "FAIL")
    cfg.setdefault("keyword_label_suggestions", {})
    return cfg


def _extract_fixes(body: str) -> list[str]:
    """Extract all 'Fixes #N' / 'Closes #N' / 'Resolves #N' lines."""
    return re.findall(r"(?:Fixes|Closes|Resolves)\s+#(\d+)", body)


# ---------------------------------------------------------------------------
# rule functions
# ---------------------------------------------------------------------------

def run(repo: str, num: int, mode: str = "", strict: bool = False) -> list[Finding]:
    cfg = _load_config()
    findings: list[Finding] = []

    if not strict and num < CUTOFF:
        print(f"== PR #{num}: SKIP (below cutoff {CUTOFF}; legacy exempt.) ==")
        return []

    pr = gh_api_get(f"repos/{repo}/pulls/{num}")
    if not pr:
        findings.append(Finding("P-00", Severity.FAIL, "PR fetch returned nothing"))
        return findings

    title: str = pr.get("title", "")
    body: str = pr.get("body") or ""
    state: str = pr.get("state", "open").lower()
    labels: list[str] = [l.get("name", "") for l in pr.get("labels", []) or []]
    mergeable: bool = pr.get("mergeable", True)

    print(f"== PR #{num}: {title} (state={state}, mergeable={mergeable}) ==")

    # ---- P-01 / P-02 title ----
    print("--- title ---")
    # P-01: title must be English (no CJK)
    if _has_cjk(title):
        findings.append(Finding("P-01", Severity.FAIL, "title contains CJK (title should be English)"))
    else:
        findings.append(Finding("P-01", Severity.INFO, "title is English"))

    # P-02: conventional commit check (WARN only — repo allows natural English)
    if re.match(r"^(feat|fix|chore|docs|style|refactor|test|ci|build|perf|revert)(\(.+\))?:\s+", title):
        findings.append(Finding("P-02", Severity.INFO, "conventional commit title"))
    else:
        findings.append(Finding("P-02", Severity.WARN, f"title not conventional commit (repo template allows natural English): {title}"))

    # ---- Body structure headings ----
    print("--- body structure ---")
    body_headings = set(_headings(body))
    for h in cfg.get("required_body_headings", ["What", "Why", "Issue", "Construction plan", "Delivery record", "How to test", "Checklist"]):
        if h in body_headings:
            findings.append(Finding("P-xx", Severity.INFO, f"heading present: {h}"))
        else:
            findings.append(Finding("P-xx", Severity.FAIL, f"missing heading: ## {h}"))

    # ---- P-10 heading English / What Chinese ----
    print("--- headings ---")
    bad_h = [h for h in _headings(body) if _has_cjk(h)]
    if bad_h:
        findings.append(Finding("P-10", Severity.FAIL, f"headings contain CJK (headings must be English): {bad_h}"))
    else:
        findings.append(Finding("P-10", Severity.INFO, "headings are English only"))

    what = _section(body, "What")
    if _has_cjk(what):
        findings.append(Finding("P-10", Severity.INFO, "What section has Chinese prose"))
    else:
        findings.append(Finding("P-10", Severity.WARN, "What section has no Chinese prose (template requires Chinese)"))

    # ---- P-14 / P-14b labels ----
    print("--- labels ---")
    all_labels = gh_api_get(f"repos/{repo}/labels")
    valid_names = {l.get("name", "") for l in all_labels or []}
    unknown = [l for l in labels if l and l not in valid_names]
    if unknown:
        findings.append(Finding("P-14", Severity.FAIL, f"labels not in repo: {unknown}"))
    else:
        findings.append(Finding("P-14", Severity.INFO, "labels all exist in repo"))
    type_labels_cfg = cfg.get("type_labels_cfg", ["bug", "enhancement", "feature", "documentation", "chore", "refactor", "tests", "epic"])
    if any(l in type_labels_cfg for l in labels):
        findings.append(Finding("P-14b", Severity.INFO, "type label present"))
    else:
        findings.append(Finding("P-14b", Severity.FAIL, "no type label (expected one of the type set)"))

    # P-14b keyword suggestions
    kw_map = cfg.get("keyword_label_suggestions", {})
    if kw_map:
        haystack = f"{title}\n{body}".lower()
        missing_suggestions: list[str] = []
        for keyword, suggested_label in kw_map.items():
            if keyword.lower() in haystack and suggested_label not in labels:
                missing_suggestions.append(suggested_label)
        if missing_suggestions:
            findings.append(Finding("P-14b", Severity.WARN, f"based on content keywords, consider also labeling: {' '.join(sorted(set(missing_suggestions)))}"))
        else:
            findings.append(Finding("P-14b", Severity.INFO, "content keywords align with assigned labels"))
    else:
        findings.append(Finding("P-14b", Severity.INFO, "no keyword suggestions (or policy not configured)"))

    # ---- P-11 / P-12 / P-13 Issue linkage ----
    print("--- issue link ---")
    fixes = _extract_fixes(body)
    fixes_unique = sorted(set(fixes), key=int)
    fixes_count = len(fixes_unique)

    # P-11: premature Fixes while open
    if state == "open" and fixes_count > 0:
        findings.append(Finding("P-11", Severity.WARN, "open PR already uses Fixes # (may close issue prematurely)"))
    else:
        findings.append(Finding("P-11", Severity.INFO, "no premature Fixes while open (or PR not open)"))

    # P-12: exactly one Fixes #
    draft = pr.get("draft", False)
    if fixes_count == 1:
        findings.append(Finding("P-12", Severity.INFO, f"exactly one Fixes #"))
    elif fixes_count == 0:
        if draft:
            findings.append(Finding("P-12", Severity.INFO, "draft PR, Fixes may appear at merge authorization"))
        else:
            findings.append(Finding("P-12", Severity.WARN, "no Fixes # yet (needs one primary issue before merge)"))
    else:
        findings.append(Finding("P-12", Severity.WARN, f"multiple Fixes # ({fixes_count}): one PR should close one issue"))

    # P-13: one primary issue
    if fixes_count <= 1:
        findings.append(Finding("P-13", Severity.INFO, "one primary issue"))
    else:
        findings.append(Finding("P-13", Severity.FAIL, "one PR should close one primary issue"))

    # ---- P-16 / P-17 / P-18 / P-19 repo repo not here → skipped
    # ---- P-19 whitespace ratio — skipped (needs git diff)
    # ---- P-39 closing reference ----
    print("--- closing reference ---")
    # P-39: check closing reference
    # Find the primary issue from Fixes #
    if fixes:
        primary = fixes[0]
        # closing reference check beyond script scope
        # simplified: just note presence of Fixes
        findings.append(Finding("P-39", Severity.INFO, f"Fixes # present: #{primary}; closing reference check requires base branch = default)"))

    # ---- P-16 repo not a git repo → skipped (handled in hook) ----

    # ---- P-17 / P-18 / P-19 repo not a git repo → skipped ----

    # ---- P-19 whitespace ratio — skipped ----

    # ---- P-20 / P-21 / P-21b labels ----
    print("--- labels (PR) ---")
    valid_names = {l.get("name", "") for l in (gh_api_get(f"repos/{repo}/labels") or [])}
    unknown = [l for l in labels if l and l not in valid_names]
    if unknown:
        findings.append(Finding("P-20", Severity.FAIL, f"labels not in repo: {unknown}"))
    else:
        findings.append(Finding("P-20", Severity.INFO, "labels all exist in repo"))
    type_labels_cfg = cfg.get("type_labels_cfg", ["bug", "enhancement", "feature", "documentation", "chore", "refactor", "tests", "epic"])
    if any(l in type_labels_cfg for l in labels):
        findings.append(Finding("P-21", Severity.INFO, "type label present"))
    else:
        findings.append(Finding("P-21", Severity.FAIL, "no type label (expected one of the type set)"))

    # P-21b keyword suggestions (same as issue)
    kw_map = cfg.get("keyword_label_suggestions", {})
    if kw_map:
        haystack = f"{title}\n{body}".lower()
        missing_suggestions: list[str] = []
        for keyword, suggested_label in kw_map.items():
            if keyword.lower() in haystack and suggested_label not in labels:
                missing_suggestions.append(suggested_label)
        if missing_suggestions:
            findings.append(Finding("P-21b", Severity.WARN, f"based on content keywords, consider also labeling: {' '.join(sorted(set(missing_suggestions)))}"))
        else:
            findings.append(Finding("P-21b", Severity.INFO, "content keywords align with assigned labels"))
    else:
        findings.append(Finding("P-21b", Severity.INFO, "no keyword suggestions (or policy not configured)"))

    # ---- P-25 / P-24 / P-22 inline review / reply — review 相关跳过，见 #103 ----
    # ---- P-26 / P-27 CI 状态 — 此处仅 WARN，实际检查由 CI 系统 —
    # ---- P-30 Done when — PR 没有 Done whenアイデ (只针对 sub-issue, PR 模式跳过) ----

    # ---- P-31 branch name ----
    print("--- branch ---")
    head_ref = pr.get("head", {}).get("ref", "")
    allowed = cfg.get("allowed_branch_prefixes", ["feat/", "fix/", "chore/", "epic/", "main", "master", "release/"])
    # 检查分支名是否以允许的前缀开头
    import fnmatch
    prefix_ok = any(fnmatch.fnmatch(head_ref, f"*{p}*") for p in allowed)
    # 分支名必须以 allowed 前缀开头
    if not head_ref or not any(head_ref.startswith(p) for p in allowed):
        findings.append(Finding("P-31", Severity.FAIL, f"branch name not allowed: {head_ref} (allowed prefixes: {allowed})"))
    else:
        findings.append(Finding("P-31", Severity.INFO, f"branch name OK: {head_ref} (prefixes: {allowed})"))

    # ---- P-35 / P-36 review 前缀 — review 相关跳过，见 #103 ----

    # ---- P-38 maintainer review — 由人工决定, 脚本只 WARN ----
    findings.append(Finding("P-38", Severity.WARN, "no maintainer review (COMMENTED/APPROVED/CHANGES_REQUESTED) — human required"))

    # ---- aggregate result — tasks depend on repo hooks interpreting exit code ----
    return findings


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    strict = "--strict" in sys.argv
    if len(args) < 2:
        print(__doc__)
        return 2
    repo = args[0]
    num = int(args[1])
    mode = args[2] if len(args) > 2 else ""
    findings = run(repo, num, mode, strict=strict)
    print_findings(findings)
    return aggregate_result(findings)


if __name__ == "__main__":
    sys.exit(main())
