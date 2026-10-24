"""Every language 2 input through every way in: `map(bytes)`, `map(dict)`, `map_json`, and the same
three on the plan loaded back from `plan_json()`. All give the same output and the same stats."""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

import pycanonic

CASES_V2 = Path(__file__).resolve().parents[2] / "conformance/cases-v2"
CASES = [p for p in sorted(CASES_V2.glob("**/*.json")) if "error" not in json.loads(p.read_text(encoding="utf-8"))]


def _ways(a: pycanonic.Artifact, inp: dict[str, Any]) -> list[tuple[str, Callable[[], Any]]]:
    ctx = inp.get("context")
    if "raw" in inp:
        raw = inp["raw"].encode()
        ways = []
    else:
        raw = json.dumps(inp["body"]).encode()
        ways = [("map(dict)", lambda: a.map(inp["body"], context=ctx))]
    return [
        ("map(bytes)", lambda: a.map(raw, context=ctx)),
        *ways,
        ("map_json", lambda: json.loads(a.map_json(raw, context=ctx))),
    ]


def _run(a: pycanonic.Artifact, inp: dict[str, Any]) -> list[tuple[str, Any, Any]]:
    """(way, output or ("rejected", reason), stats after) for each way, stats reset before each."""
    out = []
    for way, map_one in _ways(a, inp):
        a.stats_reset()
        try:
            got: Any = map_one()
        except pycanonic.Rejected as e:
            got = ("rejected", e.reason)
        out.append((way, got, a.stats_snapshot()))
    return out


def test_there_are_mapping_cases() -> None:
    assert len(CASES) > 50


@pytest.mark.parametrize("path", CASES, ids=lambda p: p.relative_to(CASES_V2).with_suffix("").as_posix())
def test_every_way_in_agrees(path: Path) -> None:
    case = json.loads(path.read_text(encoding="utf-8"))
    a = pycanonic.compile_spec(case["spec"], case["schema"])
    plan = a.plan_json()
    b = pycanonic.load(plan)
    assert json.loads(plan)["format"] in (1, 2)
    assert b.plan_json() == plan
    for i, inp in enumerate(case["inputs"]):
        runs = _run(a, inp) + [(f"loaded {w}", g, s) for w, g, s in _run(b, inp)]
        _, first, stats = runs[0]
        for way, got, snap in runs[1:]:
            assert got == first, f"input {i}: {way} gave {got!r}, map(bytes) gave {first!r}"
            assert snap == stats, f"input {i}: {way} stats {snap!r}, map(bytes) stats {stats!r}"


def _spec(fields: dict[str, Any]) -> dict[str, Any]:
    return {"language": 2, "provider": "acme", "endpoint": "people", "version": 1, "status": "promoted", "fields": fields}


def _three(a: pycanonic.Artifact, body: Any, ctx: Any = None) -> list[Any]:
    raw = json.dumps(body).encode()
    return [a.map(raw, context=ctx), a.map(body, context=ctx), json.loads(a.map_json(raw, context=ctx))]


NAMES = {"type": "array", "items": {"type": "string"}}


def test_container_transform_reads_group() -> None:
    schema = {
        "type": "object",
        "properties": {
            "teams": {"type": "array", "items": {"type": "object", "properties": {"team": {"type": "string"}, "members": NAMES}}}
        },
    }
    spec = _spec(
        {"teams[]": {"from": "staff", "group_by": "team", "consecutive": False, "item": {"team": "team", "members[]": {"from": "$group", "transform": "names"}}}}
    )
    a = pycanonic.compile_spec(spec, schema, transforms={"names": lambda g: [p["name"] for p in g]})
    body = {"staff": [{"name": "ada", "team": "core"}, {"name": "bo", "team": "web"}, {"name": "cy", "team": "core"}]}
    want = {"teams": [{"team": "core", "members": ["ada", "cy"]}, {"team": "web", "members": ["bo"]}]}
    assert _three(a, body) == [want] * 3


def test_container_transform_reads_input() -> None:
    # $input is a scalar from only, so the container is an x-open object, not a list.
    schema = {"type": "object", "properties": {"meta": {"type": "object", "x-open": True}}}
    a = pycanonic.compile_spec(
        _spec({"meta": {"from": "$input.filters", "transform": "keys"}}), schema, transforms={"keys": lambda v: {"n": len(v)}}
    )
    assert _three(a, {}, {"filters": {"a": 1, "b": 2}}) == [{"meta": {"n": 2}}] * 3
    assert _three(a, {}) == [{"meta": None}] * 3


def test_container_transform_x_empty_list_on_missing_source() -> None:
    calls: list[Any] = []

    def f(v: Any) -> Any:
        calls.append(v)
        return v

    schema = {"type": "object", "properties": {"tags": {**NAMES, "x-empty": "[]"}}}
    a = pycanonic.compile_spec(_spec({"tags[]": {"from": "t", "transform": "f"}}), schema, transforms={"f": f})
    assert _three(a, {}) == [{"tags": []}] * 3
    assert calls == []
    assert a.stats_snapshot()["fields"]["tags[]"] == {"fill": 0, "errors": 0}


def test_rejected_without_args_has_no_reason() -> None:
    assert pycanonic.Rejected().reason is None
