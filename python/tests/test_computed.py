"""Computed sources in a `from` list (R52) through the Python API."""

import json
from pathlib import Path

import pycanonic

SCHEMA = {"type": "object", "properties": {"last": {"type": "boolean"}, "n": {"type": "integer"}}}
LAST = {
    "from": [{"from": [{"from": "$input.page", "transform": {"add": 1}}, "$input.size"], "transform": "mul"}, "total"],
    "transform": "gte",
}


def _spec(fields):
    return {"language": 2, "provider": "acme", "endpoint": "items", "version": 1, "status": "promoted", "fields": fields}


def test_plan_round_trips_through_a_file(tmp_path: Path):
    a = pycanonic.compile_spec(_spec({"last": LAST}), SCHEMA)
    assert json.loads(a.plan_json())["format"] == 2
    path = tmp_path / "plan.json"
    path.write_text(a.plan_json(), encoding="utf-8")
    b = pycanonic.load(path)
    body, ctx = {"total": 45}, {"page": 4, "size": 10}
    assert b.map(body, context=ctx) == a.map(body, context=ctx) == {"last": True, "n": None}
    assert json.loads(b.map_json(json.dumps(body).encode(), context=ctx)) == {"last": True, "n": None}
    assert b.map(body, context={"page": 0, "size": 10}) == {"last": False, "n": None}


def test_custom_outer_transform_gets_none_for_a_missing_inner_value():
    seen = []

    def spy(v):
        seen.append(v)
        return "ok"

    spec = _spec({"n": {"from": [{"from": "a", "transform": {"add": 1}}, "b"], "transform": "spy"}})
    schema = {"type": "object", "properties": {"n": {"type": "string"}}}
    a = pycanonic.compile_spec(spec, schema, transforms={"spy": spy})
    assert a.map({"b": 3}) == {"n": "ok"}
    assert a.map({"a": 1, "b": 3}) == {"n": "ok"}
    assert a.map({"a": "x", "b": 3}) == {"n": "ok"}
    assert seen == [[None, 3], [2, 3], [None, 3]]
    assert a.map({}) == {"n": None}
    assert a.stats_snapshot()["fields"]["n"] == {"fill": 3, "errors": 0}


def test_missing_inner_source_is_one_error_and_a_miss():
    spec = _spec({"n": {"from": [{"from": "a", "transform": {"add": 1}}, "b"], "transform": "mul"}})
    a = pycanonic.compile_spec(spec, SCHEMA)
    assert a.map({"b": 3}) == {"last": None, "n": None}
    assert a.stats_snapshot()["fields"]["n"] == {"fill": 0, "errors": 1}
    a.stats_reset()
    assert a.map({}) == {"last": None, "n": None}
    assert a.stats_snapshot()["fields"]["n"] == {"fill": 0, "errors": 0}
