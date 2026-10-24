"""Spec language 2 rules through the Python API. Every map runs `map(bytes)`, `map(dict)` and `map_json`."""

import json

import pycanonic

SCHEMA = {"type": "object", "properties": {"name": {"type": "string"}, "kind": {"type": "string"}}}


def _spec(fields):
    return {"language": 2, "provider": "acme", "endpoint": "orgs", "version": 1, "status": "promoted", "fields": fields}


def test_const_counts_as_hit():
    body = {"other": 1}
    ways = [
        lambda a: a.map(json.dumps(body).encode()),
        lambda a: a.map(body),
        lambda a: json.loads(a.map_json(json.dumps(body).encode())),
    ]
    for way in ways:
        a = pycanonic.compile_spec(_spec({"name": "org_name", "kind": {"const": "COMPANY"}}), SCHEMA)
        assert way(a) == {"name": None, "kind": "COMPANY"}
        fields = a.stats_snapshot()["fields"]
        assert fields["kind"] == {"fill": 1, "errors": 0}
        assert fields["name"] == {"fill": 0, "errors": 0}
