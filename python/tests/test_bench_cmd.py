"""`canonic bench` and the shared timing loop `measure`, ported from bench.ts and bench/measure.js."""

from __future__ import annotations

import re
import time
from pathlib import Path

import pytest

from pycanonic.cli.__main__ import main
from pycanonic.cli.measure import measure

FIXTURES = Path(__file__).resolve().parent / "fixtures"
SCHEMA = str(FIXTURES / "specs/canonical/person.enrich.json")
GLOBEX = str(FIXTURES / "specs/globex.json")
SAMPLES = str(FIXTURES / "cli/bench-samples")

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
    # pad(name, 30) as bench.ts: a 30-character name gets one space; then the value right-aligned in 10.
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
    assert 500_000 < a < 2_000_000
    assert 0 < b < a
