"""`canonic test` and `canonic build`, ported from the TypeScript CLI's commands.test.ts and build.test.ts."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

import pycanonic
from pycanonic.cli.__main__ import main

SPECS = Path(__file__).resolve().parent / "fixtures/specs"
SCHEMA = str(SPECS / "canonical/person.enrich.json")
ACME = str(SPECS / "acme.json")


def run(capsys: pytest.CaptureFixture[str], *args: str) -> tuple[int, str, str]:
    """The CLI in-process: exit code, stdout, stderr."""
    code = main(list(args))
    out, err = capsys.readouterr()
    return code, out, err


def write(path: Path, value: Any) -> str:
    path.write_text(value if isinstance(value, str) else json.dumps(value, indent=2))
    return str(path)


def acme_case(i: int) -> Any:
    return json.loads((SPECS / "fixtures/acme.json").read_text())["cases"][i]


@pytest.mark.parametrize("cmd", ["test", "build"])
def test_help_exits_0_and_bad_usage_exits_2(capsys: pytest.CaptureFixture[str], cmd: str) -> None:
    code, out, _ = run(capsys, cmd, "--help")
    assert code == 0 and f"canonic {cmd}" in out
    assert run(capsys, cmd)[0] == 2


def test_the_command_runs_as_a_module_and_exits_with_the_verdict(tmp_path: Path) -> None:
    def sub(*args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run([sys.executable, "-m", "pycanonic.cli", *args], capture_output=True, text=True)

    ok = sub("test", "--schema", SCHEMA, ACME, str(SPECS / "fixtures/acme.json"))
    assert (ok.returncode, ok.stdout, ok.stderr) == (0, "3 cases, 0 failed\n", "")
    bad = write(tmp_path / "bad.json", {"payload": acme_case(1)["payload"], "expected": {"company": {"headcount": 99}}})
    assert sub("test", "--schema", SCHEMA, ACME, bad).returncode == 1
    assert sub("test", "--schema", SCHEMA, ACME, "no-such-file.json").returncode == 2
    assert sub("--help").returncode == 0


@pytest.mark.parametrize("provider", ["acme", "globex"])
def test_the_example_specs_pass_their_fixtures(capsys: pytest.CaptureFixture[str], provider: str) -> None:
    spec, fixtures = str(SPECS / f"{provider}.json"), str(SPECS / f"fixtures/{provider}.json")
    assert run(capsys, "test", "--schema", SCHEMA, spec, fixtures) == (0, "3 cases, 0 failed\n", "")


def test_passes_matching_fixtures_and_names_the_field_that_fails(
    capsys: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    first = acme_case(0)
    good = write(tmp_path / "good.json", {"payload": first["payload"], "expected": {"company": {"headcount": 10}, "skills": ["sql", "sales"]}})
    bad = write(tmp_path / "bad.json", {"cases": [{"payload": acme_case(1)["payload"], "expected": {"company": {"headcount": 99}}}]})
    assert run(capsys, "test", "--schema", SCHEMA, ACME, good) == (0, "1 cases, 0 failed\n", "")
    code, out, err = run(capsys, "test", "--schema", SCHEMA, ACME, good, bad)
    assert code == 1
    assert out == "2 cases, 1 failed\n"
    assert err == "bad.json#0 company.headcount: expected 99, got 47\n"


def test_a_directory_stands_for_its_json_files(capsys: pytest.CaptureFixture[str]) -> None:
    globex = str(SPECS / "globex.json")
    # Both fixture files run; acme's payloads through globex's spec fail.
    code, out, _ = run(capsys, "test", "--schema", SCHEMA, globex, str(SPECS / "fixtures"))
    assert code == 1 and out.startswith("6 cases, ")


def test_a_fixture_without_payload_and_expected_cannot_run(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    odd = write(tmp_path / "odd.json", {"payload": {}})
    code, _, err = run(capsys, "test", "--schema", SCHEMA, ACME, odd)
    assert code == 2
    assert err == f"canonic: {odd}: a fixture needs payload and expected, or cases\n"


def test_private_transforms_load_from_a_python_file(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    mod = write(tmp_path / "private.py", "def shout(v):\n    return v.upper() if isinstance(v, str) else None\n\n\ndef _helper(v):\n    return v\n")
    priv = write(
        tmp_path / "private-spec.json",
        {
            "provider": "acme",
            "endpoint": "person.enrich",
            "version": 1,
            "status": "promoted",
            "anchors": {"email": '"e_mail":'},
            "fields": {"title": {"from": "t", "transform": "shout"}, "email": {"from": "e_mail", "transform": ["trim", "lower"]}},
        },
    )
    fixture = write(tmp_path / "private-fixture.json", {"payload": {"t": "cto", "e_mail": "a@b.co"}, "expected": {"title": "CTO"}})
    code, _, err = run(capsys, "test", "--schema", SCHEMA, priv, fixture)
    assert code == 2 and 'unknown transform "shout"' in err
    assert run(capsys, "test", "--schema", SCHEMA, "--transforms", mod, priv, fixture) == (0, "1 cases, 0 failed\n", "")
    assert run(capsys, "build", "--schema", SCHEMA, "--transforms", mod, priv)[0] == 0
    assert run(capsys, "test", "--schema", SCHEMA, priv, fixture, "--transforms")[0] == 2


LIST_CANONICAL = {
    "type": "object",
    "properties": {"jobs": {"type": "array", "items": {"type": "object", "properties": {"title": {}, "company": {}, "start": {}}}}},
}
LIST_SPEC = {
    "provider": "acme",
    "endpoint": "jobs",
    "version": 1,
    "status": "promoted",
    "fields": {"jobs[]": {"from": "history", "item": {"title": "role", "company": "org"}}},
}


def test_checks_only_the_item_keys_a_fixture_lists(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    schema, spec = write(tmp_path / "list-canonical.json", LIST_CANONICAL), write(tmp_path / "list-spec.json", LIST_SPEC)
    body = {"history": [{"role": "Engineer 0", "org": "Acme"}, {"role": "CTO", "org": "Globex 0"}]}
    ok = write(tmp_path / "list-ok.json", {"payload": body, "expected": {"jobs": [{"company": "Acme", "title": "Engineer 0"}, {"title": "CTO"}]}})
    assert run(capsys, "test", "--schema", schema, spec, ok) == (0, "1 cases, 0 failed\n", "")
    bad = write(tmp_path / "list-bad.json", {"payload": body, "expected": {"jobs": [{"title": "Engineer 0"}, {"title": "CEO"}]}})
    code, _, err = run(capsys, "test", "--schema", schema, spec, bad)
    assert code == 1 and 'list-bad.json#0 jobs[1].title: expected "CEO", got "CTO"' in err


def test_1_is_a_verdict_and_anything_that_stops_a_command_is_2(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    assert run(capsys, "test", "--schema", SCHEMA, ACME, "no-such-file.json")[0] == 2
    assert run(capsys, "test", "--bogus")[0] == 2
    assert run(capsys, "test", "--schema", write(tmp_path / "broken.json", "{not json"), ACME, ACME)[0] == 2
    assert run(capsys, "nope")[0] == 2


def test_help_goes_to_stdout(capsys: pytest.CaptureFixture[str]) -> None:
    code, out, err = run(capsys, "--help")
    assert code == 0 and err == "" and "usage: canonic <command>" in out


def test_build_writes_a_plan_that_equals_compile_spec_output(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    spec = {"provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted", "fields": {"email": {"from": "e", "transform": ["trim", "private_fix"]}}}
    spec_path = write(tmp_path / "acme.json", spec)
    code, out, _ = run(capsys, "build", "--schema", SCHEMA, "--transform", "private_fix", spec_path)
    assert code == 0
    plan = tmp_path / "acme.plan.json"
    assert out == f"{spec_path} -> {plan}\n"

    def private_fix(v: Any) -> Any:
        return v.lower() if isinstance(v, str) else None

    expected = pycanonic.compile_spec(spec, Path(SCHEMA).read_text(), transforms={"private_fix": private_fix})
    assert plan.read_text() == expected.plan_json()
    assert pycanonic.load(plan, transforms={"private_fix": private_fix}).map({"e": " A@B.CO "})["email"] == "a@b.co"


def test_build_fails_with_the_field_path_on_a_bad_spec_and_exits_2(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    bad = write(tmp_path / "bad.json", {"provider": "acme", "endpoint": "e", "version": 1, "status": "draft", "fields": {"nickname": "n"}})
    code, _, err = run(capsys, "build", "--schema", SCHEMA, bad)
    assert code == 2  # 1 is kept for a verdict: a failed fixture, drift
    assert 'fields["nickname"]: canonical schema has no field' in err
    assert not (tmp_path / "bad.plan.json").exists()
    assert run(capsys, "build", "--schema", SCHEMA, "no-such-file.json")[0] == 2
    assert run(capsys, "build")[0] == 2
    assert run(capsys)[0] == 2
    assert run(capsys, "--help")[0] == 0


GROUP_CANONICAL = {
    "type": "object",
    "properties": {
        "teams": {"type": "array", "items": {"type": "object", "properties": {"team": {"type": "string"}, "count": {"type": "number"}}}},
        "page": {"type": "number"},
    },
}
GROUP_SPEC = {
    "language": 2,
    "provider": "acme",
    "endpoint": "teams",
    "version": 1,
    "status": "promoted",
    "reject": [{"missing": "staff"}],
    "fields": {
        "teams[]": {"from": "staff", "group_by": "team", "item": {"team": "team", "count": {"aggregate": "count", "from": "$group"}}},
        "page": "$input.page",
    },
}


def test_build_and_test_language_2(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    schema, spec = write(tmp_path / "teams-canonical.json", GROUP_CANONICAL), write(tmp_path / "teams.json", GROUP_SPEC)
    code, out, _ = run(capsys, "build", "--schema", schema, spec)
    assert code == 0
    plan = pycanonic.load(tmp_path / "teams.plan.json")
    body = {"staff": [{"team": "core"}, {"team": "core"}, {"team": "web"}]}
    teams = [{"team": "core", "count": 2}, {"team": "web", "count": 1}]
    assert plan.map(body, context={"page": 2}) == {"teams": teams, "page": 2}
    reason = '{"missing":"staff"}'
    good = write(
        tmp_path / "good.json",
        {
            "cases": [
                {"payload": body, "context": {"page": 2}, "expected": {"teams": teams, "page": 2}},
                {"payload": body, "expected": {"page": None}},
                {"payload": {}, "expected": {"$rejected": reason}},
            ]
        },
    )
    assert run(capsys, "test", "--schema", schema, spec, good) == (0, "3 cases, 0 failed\n", "")
    bad = write(
        tmp_path / "bad.json",
        {
            "cases": [
                {"payload": {}, "expected": {"page": None}},
                {"payload": body, "expected": {"$rejected": reason}},
                {"payload": {}, "expected": {"$rejected": '{"missing":"other"}'}},
            ]
        },
    )
    code, out, err = run(capsys, "test", "--schema", schema, spec, bad)
    assert (code, out) == (1, "3 cases, 3 failed\n")
    assert err.splitlines() == [
        f"bad.json#0 rejected {reason}",
        f"bad.json#1 expected rejected {reason}, got an output",
        f"bad.json#2 expected rejected {{\"missing\":\"other\"}}, got rejected {reason}",
    ]
