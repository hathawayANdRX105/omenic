"""Unit tests for .githooks/install_gh_gate.py helper functions.

Run from repo root:
    python -m pytest .githooks/tests/test_gh_gate.py -v
"""
import sys
from pathlib import Path
from unittest.mock import patch, MagicMock

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import importlib
_mod = importlib.import_module("install_gh_gate")
_section = _mod._section
_check_done_when_fully_ticked = _mod._check_done_when_fully_ticked
_extract = _mod._extract
_gh_args = _mod._gh_args


# ---------------------------------------------------------------------------
# _section
# ---------------------------------------------------------------------------

def test_section_extracts_under_heading():
    body = "## Goal\n内容\n\n## Done when\n- [x] 完成\n\n## Background\n背景"
    sec = _section(body, "Done when")
    assert "- [x] 完成" in sec
    assert "## Background" not in sec


def test_section_missing_heading():
    assert _section("## Goal\n内容", "Missing") == ""


# ---------------------------------------------------------------------------
# _check_done_when_fully_ticked
# ---------------------------------------------------------------------------

def test_all_ticked():
    body = "## Done when\n- [x] 完成\n- [x] 验证\n"
    ok, unticked = _check_done_when_fully_ticked(body)
    assert ok is True
    assert len(unticked) == 0


def test_some_unticked():
    body = "## Done when\n- [x] 完成\n- [ ] 待做\n- [x] 验证\n- [ ] 待测\n"
    ok, unticked = _check_done_when_fully_ticked(body)
    assert ok is False
    assert len(unticked) == 2
    assert "待做" in unticked
    assert "待测" in unticked


def test_no_done_when_section():
    body = "## Goal\n内容\n"
    ok, unticked = _check_done_when_fully_ticked(body)
    assert ok is True
    assert len(unticked) == 0


def test_custom_heading():
    plan = "## Construction plan\n- [x] step1\n- [ ] step2\n"
    ok, unticked = _check_done_when_fully_ticked(plan, "Construction plan")
    assert ok is False
    assert len(unticked) == 1


def test_empty_section():
    body = "## Done when\n"
    ok, unticked = _check_done_when_fully_ticked(body)
    assert ok is True
    assert len(unticked) == 0


# ---------------------------------------------------------------------------
# _extract
# ---------------------------------------------------------------------------

def test_extract_title_body_labels():
    args = ["--title", "DEMO test", "--body", "正文内容", "--label", "sub,enhancement", "--label", "bug"]
    title, body, labels, head, parent = _extract(args)
    assert title == "DEMO test"
    assert body == "正文内容"
    assert "sub" in labels
    assert "enhancement" in labels
    assert "bug" in labels
    assert head == ""
    assert parent == ""


def test_extract_head_parent():
    args = ["--title", "feat: test", "--head", "feat/xxx", "--parent", "124"]
    title, body, labels, head, parent = _extract(args)
    assert title == "feat: test"
    assert head == "feat/xxx"
    assert parent == "124"


def test_extract_equals_form():
    args = ["--title=DEMO", "--body=内容", "--label=bug", "--head=feat/x", "--parent=125"]
    title, body, labels, head, parent = _extract(args)
    assert title == "DEMO"
    assert body == "内容"
    assert "bug" in labels
    assert head == "feat/x"
    assert parent == "125"


def test_extract_empty():
    title, body, labels, head, parent = _extract([])
    assert title == ""
    assert body == ""
    assert labels == []
    assert head == ""
    assert parent == ""


# ---------------------------------------------------------------------------
# _gh_args
# ---------------------------------------------------------------------------

def test_gh_args_removes_parent():
    args = ["--title", "x", "--parent", "124", "--body", "y"]
    result = _gh_args(args)
    assert "--parent" not in result
    assert "124" not in result
    assert "--title" in result
    assert "--body" in result


def test_gh_args_keeps_other_args():
    args = ["--title", "x", "--label", "bug,enhancement", "--head", "feat/x"]
    result = _gh_args(args)
    assert result == args


def test_gh_args_equals_parent():
    args = ["--title=x", "--parent=124", "--body=y"]
    result = _gh_args(args)
    assert "--parent=124" not in result
    assert result == ["--title=x", "--body=y"]