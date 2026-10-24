"""Every case in conformance/cases-v2 through `map(bytes)`, `map(parsed)` and `map_json`.

See conformance/cases-v2/README.md for the case format and conformance/README.md for the comparison
rule (conformance/compare.py).
"""

import json
import sys
from pathlib import Path

import pytest

import pycanonic

CONFORMANCE = Path(__file__).resolve().parents[2] / "conformance"
sys.path.insert(0, str(CONFORMANCE))
from compare import diff  # noqa: E402

CASES = sorted((CONFORMANCE / "cases-v2").glob("**/*.json"))


def test_there_are_cases():
    assert CASES, "no cases in conformance/cases-v2"


def _ways(a, inp):
    """`(way, map the input)` for each way in. A raw input is not JSON, so it has no parsed way."""
    if ("body" in inp) == ("raw" in inp) or not isinstance(inp.get("raw", ""), str):
        raise ValueError("an input needs exactly one of body, raw (a string)")
    if "raw" in inp:
        raw = inp["raw"].encode()
    else:
        raw = json.dumps(inp["body"]).encode()
        yield "map(parsed)", lambda: a.map(inp["body"])
    yield "map(bytes)", lambda: a.map(raw)
    yield "map_json", lambda: json.loads(a.map_json(raw))


@pytest.mark.parametrize("path", CASES, ids=lambda p: p.relative_to(CONFORMANCE / "cases-v2").with_suffix("").as_posix())
def test_case(path):
    case = json.loads(path.read_text(encoding="utf-8"))
    if "error" in case:
        with pytest.raises(pycanonic.SpecError) as e:
            pycanonic.compile_spec(case["spec"], case["schema"])
        assert str(e.value) == case["error"]
        return

    a = pycanonic.compile_spec(case["spec"], case["schema"])
    fails = []
    for i, inp in enumerate(case["inputs"]):
        want = inp["expected"]
        # Tasks 8 and 9 add context and Rejected. Until then a case that uses them fails.
        if inp.get("context") is not None:
            fails.append(f"inputs[{i}]: context is not implemented yet")
            continue
        if isinstance(want, dict) and "$rejected" in want:
            fails.append(f"inputs[{i}]: $rejected is not implemented yet")
            continue
        for way, map_one in _ways(a, inp):
            fails.append(diff(map_one(), want, f"{way}[{i}]"))
    fails = [f for f in fails if f]
    assert not fails, "\n".join(fails)
