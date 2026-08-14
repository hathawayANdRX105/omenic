#!/usr/bin/env python3
"""Parity regression: run Python validators on a reference set of issues/PRs.

Usage:
    python regress/parity.py [--strict]

Default: skips issues below cutoff. --strict validates all.
Checks that validators execute without crashing (exit 0/1 with RESULT output).
Content FAILs are legitimate validator results, not crashes.
"""



import sys as _sys
_sys.dont_write_bytecode = True  # 不生成 __pycache__
import sys
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GITHOOKS = ROOT / ".githooks"

# Reference set: issues and PRs covering sub-issue, parent, and PR modes
REFERENCE_ISSUES = [97, 100, 101, 102, 103, 105, 106, 107, 108, 109, 110, 111]
REFERENCE_PRS = [95, 114, 115, 116]
REPO = "hathawayANdRX105/omenic"


def run_python(script: str, *args: str) -> tuple[int, str]:
    """Run a Python validator and return (exit_code, output)."""
    cmd = [sys.executable, str(script)] + list(args)
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
    out = (proc.stdout or "") + (proc.stderr or "")
    return proc.returncode, out


def main() -> int:
    strict = "--strict" in sys.argv
    flags = ["--strict"] if strict else []
    crashed = 0
    total = 0

    print("=== Parity Regression: Python Validators ===")
    print(f"Repo: {REPO}\n")

    print("--- Issues ---")
    for num in REFERENCE_ISSUES:
        script = GITHOOKS / "github" / "issues.py"
        rc, out = run_python(script, REPO, str(num), *flags)
        total += 1
        if "Traceback" in out:
            print(f"  CRASH #{num}")
            crashed += 1
        elif "RESULT:" in out:
            print(f"  OK    #{num} (exit {rc})")
        else:
            print(f"  ???   #{num}")
            crashed += 1

    print("\n--- PRs ---")
    for num in REFERENCE_PRS:
        script = GITHOOKS / "github" / "pull_requests.py"
        rc, out = run_python(script, REPO, str(num), *flags)
        total += 1
        if "Traceback" in out:
            print(f"  CRASH PR #{num}")
            crashed += 1
        elif "RESULT:" in out:
            print(f"  OK    PR #{num} (exit {rc})")
        else:
            print(f"  ???   PR #{num}")
            crashed += 1

    print(f"\n=== Result: {total} total, {crashed} crashed ===")
    return 1 if crashed > 0 else 0


if __name__ == "__main__":
    sys.exit(main())

