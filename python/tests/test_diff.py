"""`canonic diff`, ported from the diff cases of the TypeScript CLI's commands.test.ts.

The `fixtures/cli/ts/diff-*` files are the TypeScript command's own stdout and stderr, captured once
from the canonic directory with A=packages/cli/test/.aot/commands (the fixtures here are copies of those):

  node packages/cli/dist/cli.js diff $A/samples $A/renamed                          # diff-renamed
  node packages/cli/dist/cli.js diff --schema $A/canonical.json --spec $A/spec.json \\
    $A/samples $A/renamed                                                            # diff-renamed-spec
  node packages/cli/dist/cli.js diff --schema $A/canonical.json --spec $A/private-spec.json \\
    --transforms $A/private.mjs --learn $A/approved-title.json $A/priv-old $A/priv-new  # diff-priv
  node packages/cli/dist/cli.js diff --schema $A/list-canonical.json --spec $A/list-spec.json \\
    $A/list-old $A/list-new                                                          # diff-list
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from pycanonic.cli.__main__ import main

CLI = Path(__file__).resolve().parent / "fixtures/cli"
TS = CLI / "ts"
SCHEMA = str(CLI / "canonical.json")
SPEC_FILE = str(CLI / "spec.json")
SPEC: dict[str, Any] = json.loads((CLI / "spec.json").read_text())
SAMPLES = str(CLI / "samples")
RENAMED = str(CLI / "renamed")  # samples with contact.mobile renamed to contact.phoneNumber
PRIV = ["--schema", SCHEMA, "--spec", str(CLI / "private-spec.json"), str(CLI / "priv-old"), str(CLI / "priv-new")]
LIST = ["--schema", str(CLI / "list-canonical.json"), "--spec", str(CLI / "list-spec.json"), str(CLI / "list-old"), str(CLI / "list-new")]


def run(capsys: pytest.CaptureFixture[str], *args: str) -> tuple[int, str, str]:
    code = main(["diff", *args])
    out, err = capsys.readouterr()
    return code, out, err


def ts(name: str, out: str) -> tuple[int, str, str]:
    """The TypeScript command's exit code (1: every captured run found drift), stdout and stderr."""
    return 1, (TS / f"{name}.{out}").read_text(), (TS / f"{name}.err.txt").read_text()


def test_help_exits_0_and_bad_usage_exits_2(capsys: pytest.CaptureFixture[str]) -> None:
    code, out, _ = run(capsys, "--help")
    assert code == 0 and out.startswith("canonic diff")
    assert run(capsys)[0] == 2
    assert run(capsys, SAMPLES)[0] == 2
    assert run(capsys, "--schema", SCHEMA, SAMPLES, RENAMED)[0] == 2  # --schema needs --spec
    assert run(capsys, "--spec", SPEC_FILE, SAMPLES, RENAMED)[0] == 2
    assert run(capsys, "--bogus", SAMPLES, RENAMED)[0] == 2


def test_the_same_samples_twice_is_no_drift(capsys: pytest.CaptureFixture[str]) -> None:
    assert run(capsys, SAMPLES, SAMPLES) == (0, "", "")
    assert run(capsys, "--schema", SCHEMA, "--spec", SPEC_FILE, SAMPLES, SAMPLES) == (0, "", "")


def test_a_renamed_key_shows_up_as_drift(capsys: pytest.CaptureFixture[str]) -> None:
    code, out, err = run(capsys, SAMPLES, RENAMED)
    assert code == 1 and err == ""
    assert "- contact.mobile (was in 100%)" in out
    assert "+ contact.phoneNumber (in 100%)" in out
    assert (code, out, err) == (1, (TS / "diff-renamed.out.txt").read_text(), "")


def test_with_a_spec_only_the_broken_field_gets_a_proposal(capsys: pytest.CaptureFixture[str]) -> None:
    r = run(capsys, "--schema", SCHEMA, "--spec", SPEC_FILE, SAMPLES, RENAMED)
    assert r[0] == 1
    assert "field phone: filled 100% -> 0%" in r[2]
    nxt = json.loads(r[1])
    assert nxt["version"] == 2 and nxt["status"] == "shadow"
    assert nxt["fields"]["phone"] == {"from": "contact.phoneNumber", "transform": "normalize_phone"}
    # Only the broken field changes. The reviewed rules stay as they were.
    assert {**nxt["fields"], "phone": SPEC["fields"]["phone"]} == SPEC["fields"]
    # A re-mapped routing field gets a fresh anchor.
    assert nxt["anchors"] == {"phone": '"phoneNumber":'}
    assert r == ts("diff-renamed-spec", "out.json")


def test_out_writes_the_spec_and_the_report_goes_to_stdout(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    out = tmp_path / "next.json"
    code, stdout, err = run(capsys, "--schema", SCHEMA, "--spec", SPEC_FILE, "--out", str(out), SAMPLES, RENAMED)
    _, want, report = ts("diff-renamed-spec", "out.json")
    assert (code, stdout, err) == (1, report, "")
    assert out.read_text() == want


def test_diff_compiles_a_spec_with_private_transforms_and_re_infers_with_learned_synonyms_and_fresh_anchors(
    capsys: pytest.CaptureFixture[str],
) -> None:
    code, _, err = run(capsys, *PRIV)
    assert code == 2 and 'unknown transform "shout"' in err
    r = run(capsys, *PRIV, "--transforms", str(CLI / "private.py"), "--learn", str(CLI / "approved-title.json"))
    assert r[0] == 1
    assert "- e_mail (was in 100%)" in r[2] and "field email: filled 100% -> 0%" in r[2]
    nxt = json.loads(r[1])
    assert nxt["fields"]["title"] == "posten"
    assert nxt["fields"]["email"] == {"from": "mail_addr", "transform": ["trim", "lower"]}
    assert nxt["anchors"] == {"email": '"mail_addr":'}
    assert r == ts("diff-priv", "out.json")


def test_diff_sees_a_key_renamed_inside_list_items_and_proposes_the_lists_rule_again(capsys: pytest.CaptureFixture[str]) -> None:
    r = run(capsys, *LIST)
    assert r[0] == 1
    assert "field jobs[].title: filled 100% -> 0%" in r[2]
    assert json.loads(r[1])["fields"]["jobs[]"] == {"from": "history", "item": {"title": "title", "company": "org"}}
    assert r == ts("diff-list", "out.json")


def test_a_missing_directory_exits_2(capsys: pytest.CaptureFixture[str]) -> None:
    assert run(capsys, SAMPLES, "no-such-dir")[0] == 2
