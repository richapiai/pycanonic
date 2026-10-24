"""`canonic bench` and the shared timing loop `measure`, ported from bench.ts and bench/measure.js."""

from __future__ import annotations

import json
import re
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from pycanonic.cli.__main__ import main
from pycanonic.cli.commands import bench
from pycanonic.cli.measure import measure

FIXTURES = Path(__file__).resolve().parent / "fixtures"
SCHEMA = str(FIXTURES / "specs/canonical/person.enrich.json")
GLOBEX = str(FIXTURES / "specs/globex.json")
SAMPLES = str(FIXTURES / "cli/bench-samples")
CLI = FIXTURES / "cli"

ROWS = [
    "json.loads",
    "map",
    "view + routing reads",
    "extract (routing, from text)",
    "map (bytes in)",
    "map_json (bytes in, bytes out)",
]


def test_bench_prints_the_header_and_one_timed_row_per_path(capsys: pytest.CaptureFixture[str]) -> None:
    code = main(["bench", "--schema", SCHEMA, GLOBEX, SAMPLES])
    out, err = capsys.readouterr()
    assert (code, err) == (0, "")
    head, blank, *rows = out.splitlines()
    assert re.fullmatch(r"globex person\.enrich v1: 3 samples, \d+\.\d KB average, \d+ routing fields", head)
    assert blank == ""
    # pad(name, 30) as bench.ts: a 30-character name gets one space, then the value right-aligned in 10.
    assert [r[:-10].rstrip() for r in rows] == ROWS
    assert all(len(r[:-10]) == max(30, len(name) + 1) for r, name in zip(rows, ROWS))
    assert all(re.fullmatch(r" *(\d+\.\d\d µs|\d+ ns)", r[-10:]) for r in rows), rows


def test_bench_help_exits_0_and_bad_usage_exits_2(capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["bench", "--help"]) == 0
    assert capsys.readouterr().out.startswith("canonic bench --schema")
    assert main(["bench"]) == 2
    assert main(["bench", "--schema", SCHEMA, GLOBEX]) == 2


def test_measure_returns_median_ns_per_op_for_each_function_round_robin() -> None:
    a, b = measure([("sleep", lambda i: time.sleep(0.001)), ("noop", lambda i: i)], rounds=3)
    assert 500_000 < a < 5_000_000
    assert 0 < b < a


def test_measure_with_a_modulus_visits_every_index_past_63() -> None:
    seen: set[int] = set()
    plain: set[int] = set()
    measure([("visit", seen.add)], rounds=1, batch_ns=100_000, modulus=70)
    measure([("plain", plain.add)], rounds=1, batch_ns=100_000)  # the default keeps the i & 63 contract
    assert seen == set(range(70))
    assert plain == set(range(64))


def test_bench_times_every_sample_the_header_counts(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    for i in range(70):  # the e-mail is the marker: p64..p69 sort after the first 64 files
        (tmp_path / f"p{i:02}.json").write_text(json.dumps({"e_mail": f"p{i}@acme.com", "fname": "Ada"}))
    emails: set[str] = set()
    real = measure

    def spy(fns: list[tuple[str, Callable[[int], Any]]], **kw: Any) -> list[float]:
        name, mapped = fns[1]
        assert name == "map"
        fns[1] = (name, lambda i: emails.add(mapped(i)["email"]))
        return real(fns, **kw)

    monkeypatch.setattr(bench, "measure", spy)
    assert main(["bench", "--schema", SCHEMA, GLOBEX, str(tmp_path)]) == 0
    head = capsys.readouterr().out.splitlines()[0]
    count = int(re.search(r": (\d+) samples", head).group(1))  # type: ignore[union-attr]
    assert count == len(emails) == 70


def test_bench_loads_private_transforms_from_a_module(capsys: pytest.CaptureFixture[str]) -> None:
    args = ["bench", "--schema", str(CLI / "canonical.json"), str(CLI / "private-spec.json"), str(CLI / "priv-old")]
    assert main(args) == 2
    assert 'unknown transform "shout"' in capsys.readouterr().err
    assert main([*args[:1], "--transforms", str(CLI / "private.py"), *args[1:]]) == 0


def test_bench_runs_a_language_2_spec_without_the_view_row_or_rejected_samples(
    capsys: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    schema = {"type": "object", "properties": {"email": {"type": "string", "semantic": "email", "routing": True}, "tags": {"type": "array", "items": {"type": "string"}}}}
    spec = {
        "language": 2,
        "provider": "acme",
        "endpoint": "people",
        "version": 1,
        "status": "promoted",
        "reject": [{"missing": "e_mail"}],
        "fields": {"email": {"from": "e_mail", "transform": "lower"}, "tags[]": {"from": "t", "wrap": True}},
    }
    (tmp_path / "s").mkdir()
    for i, body in enumerate([{"e_mail": "A@acme.com", "t": "x"}, {"e_mail": "b@acme.com"}, {"t": "gone"}]):
        (tmp_path / f"s/{i}.json").write_text(json.dumps(body))
    (tmp_path / "schema.json").write_text(json.dumps(schema))
    (tmp_path / "spec.json").write_text(json.dumps(spec))
    code = main(["bench", "--schema", str(tmp_path / "schema.json"), str(tmp_path / "spec.json"), str(tmp_path / "s")])
    out, err = capsys.readouterr()
    assert (code, err) == (0, "")
    head, _, *rows = out.splitlines()
    assert re.fullmatch(r"acme people v1: 2 samples \(1 rejected, left out\), \d+\.\d KB average, 1 routing fields", head)
    assert [r[:-10].rstrip() for r in rows] == [r for r in ROWS if r != "view + routing reads"]
