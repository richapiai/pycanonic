"""Every case of the vendored TypeScript corpus through `map(bytes)`, `map(parsed)` and `map_json`.

See conformance/README.md for the case shape and the comparison rule (conformance/compare.py).
"""

import json
import re
import sys
from pathlib import Path

import pytest

import pycanonic

CONFORMANCE = Path(__file__).resolve().parents[2] / "conformance"
sys.path.insert(0, str(CONFORMANCE))
from compare import diff, fffd  # noqa: E402

CASES = sorted((CONFORMANCE / "cases").glob("**/*.json"))
EXCLUDED = set(re.findall(r"^- `([^`]+)`", (CONFORMANCE / "EXCLUDED.md").read_text(), re.M))


def blank(schema):
    """What the TypeScript artifact maps a body it never parsed to: the canonical schema's objects,
    every other field null."""
    props = schema.get("properties")
    if schema.get("type") == "object" and isinstance(props, dict):
        return {k: blank(x) for k, x in props.items()}
    return None


def test_the_corpus_is_vendored():
    assert CASES, "no corpus: run conformance/sync.sh"


def test_compare_rule():
    assert diff(2**53 + 1, 2**53) is None
    assert diff(2**53 + 4, 2**53) is not None
    assert diff(1, 1.0) is None
    assert diff(True, 1) is not None
    assert diff({"a": 1, "b": 2}, {"b": 2, "a": 1}) is not None
    assert diff("x" + chr(0xD800), "x" + chr(0xFFFD)) is None
    assert fffd(chr(0xD83D) + chr(0xDE00)) == chr(0x1F600)


def _paths(a, bodies):
    """`(path, map one body)` for each way in: bytes, the parsed object, and map_json."""
    yield "map(bytes)", lambda i: a.map(bodies[i])
    yield "map(parsed)", lambda i: a.map(json.loads(bodies[i]))
    yield "map_json", lambda i: json.loads(a.map_json(bodies[i]))


@pytest.mark.parametrize("path", CASES, ids=lambda p: p.relative_to(CONFORMANCE / "cases").with_suffix("").as_posix())
def test_case(path):
    case = json.loads(path.read_text(encoding="utf-8"))
    if case["name"] in EXCLUDED:
        pytest.skip("listed in conformance/EXCLUDED.md")
    if "error" in case:
        with pytest.raises(pycanonic.SpecError) as e:
            pycanonic.compile_spec(case["spec"], case["canonical"])
        assert fffd(str(e.value)) == fffd(case["error"])
        return

    a = pycanonic.compile_spec(case["spec"], case["canonical"])
    maps, want_stats = case["expected"]["map"], case["expected"]["stats"]
    bodies = [fffd(s).encode() for s in case["inputs"]]
    shape = blank(case["canonical"])
    fails = []
    for name, map_one in _paths(a, bodies):
        a.stats.reset()
        for i, want in enumerate(maps):
            if want is not None:
                fails.append(diff(map_one(i), want, f"{name} map[{i}]"))
        # TypeScript never maps a body JSON.parse rejects, so these go after the stats snapshot.
        fails.append(diff(a.stats.snapshot(), want_stats, f"{name} stats"))
        if name != "map(parsed)":
            for i, want in enumerate(maps):
                if want is None:
                    fails.append(diff(map_one(i), shape, f"{name} map[{i}] (unparsed)"))
    fails = [f for f in fails if f]
    assert not fails, "\n".join(fails)
