"""Conditional and array `coalesce` (R56, R57) through the Python API."""

import json
from pathlib import Path

import pytest

import pycanonic

SCHEMA = {
    "type": "object",
    "properties": {"ref": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}}},
}
FIELDS = {
    "ref": {"coalesce": [{"from": "share", "when": {"starts_with": "urn:example:post:"}}, "id"]},
    "tags[]": {"coalesce": ["tags_v2", {"from": "tags", "when": {"path": "ver", "lt": 3}}], "wrap": True},
}
BODIES = [
    {"share": "urn:example:post:1", "id": "a", "tags_v2": "x"},
    {"share": "urn:example:user:1", "id": "a", "tags": ["t"], "ver": 2},
    {"share": None, "id": "a", "tags_v2": [], "tags": "t", "ver": 3},
    {"id": "a", "tags_v2": "", "tags": ["t"], "ver": 1},
    {},
]
WANT = [
    {"ref": "urn:example:post:1", "tags": ["x"]},
    {"ref": "a", "tags": ["t"]},
    {"ref": "a", "tags": None},
    {"ref": "a", "tags": [""]},
    {"ref": None, "tags": None},
]


def _spec(fields):
    return {"language": 2, "provider": "acme", "endpoint": "posts", "version": 1, "status": "promoted", "fields": fields}


def test_plan_saves_loads_and_maps_the_same(tmp_path: Path):
    a = pycanonic.compile_spec(_spec(FIELDS), SCHEMA)
    assert json.loads(a.plan_json())["format"] == 2
    path = tmp_path / "plan.json"
    path.write_text(a.plan_json(), encoding="utf-8")
    b = pycanonic.load(path)
    for body, want in zip(BODIES, WANT, strict=True):
        raw = json.dumps(body).encode()
        assert a.map(body) == b.map(body) == a.map(raw) == b.map(raw) == want
        assert json.loads(b.map_json(raw)) == want


def test_plan_source_prints_both_forms():
    a = pycanonic.compile_spec(_spec(FIELDS), SCHEMA)
    assert a.source == (
        'ref <- coalesce(share when share starts_with "urn:example:post:", id)\n'
        "tags[] <- coalesce(tags_v2, tags when ver lt 3.0)[] | wrap\n"
    )


def test_misses_are_counted():
    a = pycanonic.compile_spec(_spec(FIELDS), SCHEMA)
    a.map({})
    assert a.stats_snapshot()["fields"]["tags[]"] == {"fill": 0, "errors": 0}
    a.map({"id": "a", "tags_v2": "x"})
    assert a.stats_snapshot()["fields"]["tags[]"] == {"fill": 1, "errors": 0}


def test_routing_field_refuses_a_conditional_entry():
    schema = {"type": "object", "properties": {"email": {"type": "string", "semantic": "email", "routing": True}}}
    spec = _spec({"email": {"coalesce": [{"from": "a", "when": {"contains": "@"}}, "b"]}})
    with pytest.raises(pycanonic.SpecError, match="routing fields can't have when"):
        pycanonic.compile_spec(spec, schema)
