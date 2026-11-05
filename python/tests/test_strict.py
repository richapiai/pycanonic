"""`"x-shape": "strict"` (R48): every node with no usable value writes its empty form. The
conformance cases in conformance/cases-v2/strict hold the outputs. These tests pin what a case
cannot: stats, plan format and custom container transforms."""

import json
from pathlib import Path

import pytest

import pycanonic

STRICT = Path(__file__).resolve().parents[2] / "conformance" / "cases-v2" / "strict"


def _spec(fields):
    return {"language": 2, "provider": "acme", "endpoint": "orgs", "version": 1, "status": "promoted", "fields": fields}


def _strict(schema):
    return {"x-shape": "strict", **schema}


def _ways(a, body):
    raw = json.dumps(body).encode()
    return [lambda: a.map(raw), lambda: a.map(body), lambda: json.loads(a.map_json(raw))]


@pytest.mark.parametrize("name", ["array-paths", "strict-nested-error"])
def test_strict_keeps_error_and_miss_counts(name):
    strict = json.loads((STRICT / f"{name}.json").read_text())
    plain = json.loads((STRICT / f"{name}-plain.json").read_text())
    a = pycanonic.compile_spec(strict["spec"], strict["schema"])
    b = pycanonic.compile_spec(plain["spec"], plain["schema"])
    assert "x-shape" not in plain["schema"]
    for s, p in zip(strict["inputs"], plain["inputs"], strict=True):
        for way in _ways(a, s["body"]):
            assert way() == s["expected"]
        for way in _ways(b, p["body"]):
            assert way() == p["expected"]
    assert a.stats_snapshot() == b.stats_snapshot()
    errors = sum(f["errors"] for f in a.stats_snapshot()["fields"].values())
    assert errors > 0


def test_strict_plan_is_format_2_and_round_trips():
    case = json.loads((STRICT / "array-paths.json").read_text())
    a = pycanonic.compile_spec(case["spec"], case["schema"])
    plan = json.loads(a.plan_json())
    assert plan["format"] == 2 and plan["strict"] is True
    b = pycanonic.load(a.plan_json())
    for inp in case["inputs"]:
        for way in _ways(b, inp["body"]):
            assert way() == inp["expected"]
    # Without x-shape the plan has no strict key.
    plain = json.loads((STRICT / "array-paths-plain.json").read_text())
    assert "strict" not in pycanonic.compile_spec(plain["spec"], plain["schema"]).plan_json()
    # A leaf-only strict schema is still a strict plan.
    leaf = pycanonic.compile_spec(_spec({"name": "n"}), _strict({"type": "object", "properties": {"name": {"type": "string"}}}))
    assert json.loads(leaf.plan_json())["format"] == 2


HQ = {
    "type": "object",
    "properties": {
        "hq": {
            "type": "object",
            "properties": {
                "city": {"type": "string"},
                "tags": {"type": "array", "items": {"type": "string"}},
                "gone": {"type": "string", "x-empty": "missing"},
                "links": {"type": "array", "items": {"type": "string"}, "x-empty": "null"},
                "sub": {"type": "object", "properties": {"ids": {"type": "array", "items": {"type": "string"}}}},
            },
        },
        "teams": {"type": "array", "items": {"type": "object", "properties": {"team": {"type": "string"}}}},
    },
}
HQ_EMPTY = {"city": None, "tags": [], "links": None, "sub": {"ids": []}}


def boom(v):
    raise ValueError("no")


@pytest.mark.parametrize("f", [boom, lambda v: {"extra": 1}, lambda v: [1]], ids=["raises", "extra-key", "wrong-type"])
def test_container_failure_writes_the_empty_form(f):
    spec = _spec({"hq": {"from": "office", "transform": "f"}, "teams[]": {"from": "staff", "transform": "f"}})
    body = {"office": {"x": 1}, "staff": [{"x": 1}]}
    a = pycanonic.compile_spec(spec, _strict(HQ), transforms={"f": f})
    b = pycanonic.compile_spec(spec, HQ, transforms={"f": f})
    for way in _ways(a, body):
        assert way() == {"hq": HQ_EMPTY, "teams": []}
    for way in _ways(b, body):
        assert way() == {"hq": None, "teams": None}
    assert a.stats_snapshot() == b.stats_snapshot()
    assert a.stats_snapshot()["fields"]["hq"]["errors"] == 3


def test_container_missing_source_and_missing_children():
    spec = _spec({"hq": {"from": "office", "transform": "f"}, "teams[]": {"from": "staff", "transform": "f"}})
    a = pycanonic.compile_spec(spec, _strict(HQ), transforms={"f": lambda v: {"city": "Oslo"}})
    for way in _ways(a, {}):
        assert way() == {"hq": HQ_EMPTY, "teams": []}
    for way in _ways(a, {"office": {}}):
        assert way() == {"hq": {**HQ_EMPTY, "city": "Oslo"}, "teams": []}
    assert a.stats_snapshot()["fields"]["hq"]["errors"] == 0
    b = pycanonic.compile_spec(spec, HQ, transforms={"f": lambda v: {"city": "Oslo"}})
    for way in _ways(b, {"office": {}}):
        assert way() == {"hq": {"city": "Oslo", "tags": None, "links": None, "sub": None}, "teams": None}


def test_custom_failure_deep_inside_writes_every_key():
    schema = {
        "type": "object",
        "properties": {
            "orders": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "ship": {
                            "type": "object",
                            "properties": {
                                "lines": {"type": "array", "items": {"type": "object", "properties": {"sku": {"type": "string"}}}},
                                "box": {"type": "object", "properties": {"ids": {"type": "array", "items": {"type": "string"}}}},
                            },
                        },
                    },
                },
            }
        },
    }
    item = {"id": "id", "ship.lines[]": {"from": "ship.lines", "transform": "f"}, "ship.box": {"from": "ship.box", "transform": "f"}}
    spec = _spec({"orders[]": {"from": "orders", "item": item}})
    body = {"orders": [{"id": "o1", "ship": {"lines": [{"sku": "a"}], "box": {}}}, {"id": "o2"}]}
    a = pycanonic.compile_spec(spec, _strict(schema), transforms={"f": boom})
    empty = {"lines": [], "box": {"ids": []}}
    for way in _ways(a, body):
        assert way() == {"orders": [{"id": "o1", "ship": empty}, {"id": "o2", "ship": empty}]}
    b = pycanonic.compile_spec(spec, schema, transforms={"f": boom})
    for way in _ways(b, body):
        assert way() == {"orders": [{"id": "o1", "ship": {"lines": None, "box": None}}, {"id": "o2", "ship": {"lines": None, "box": None}}]}
    assert a.stats_snapshot() == b.stats_snapshot()
