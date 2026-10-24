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


def test_const_plan_round_trips():
    a = pycanonic.compile_spec(_spec({"name": "org_name", "kind": {"const": "COMPANY"}}), SCHEMA)
    plan = a.plan_json()
    assert json.loads(plan)["format"] == 2
    b = pycanonic.load(plan)
    body = {"org_name": "Widget Works"}
    want = {"name": "Widget Works", "kind": "COMPANY"}
    assert b.map(json.dumps(body).encode()) == want
    assert b.map(body) == want
    assert json.loads(b.map_json(json.dumps(body).encode())) == want


def test_x_empty_missing_leaves_the_key_out():
    schema = {"type": "object", "properties": {"k": {"type": "string", "x-empty": "missing"}, "name": {"type": "string"}}}
    a = pycanonic.compile_spec(_spec({"name": "n", "k": "v"}), schema)
    for body in [{"n": "a"}, {"n": "a", "v": None}]:
        raw = json.dumps(body).encode()
        text = a.map_json(raw)
        assert b'"k":' not in text
        assert json.loads(text) == a.map(raw) == a.map(body) == {"name": "a"}


def test_view_rejects_language_2():
    import pytest

    schema = {"type": "object", "properties": {"hq": {"type": "object", "properties": {"city": {"type": "string"}}}}}
    a = pycanonic.compile_spec(_spec({"hq": {"present": "d"}, "hq.city": "d.city"}), schema)
    for src in [{}, b"{}"]:
        with pytest.raises(ValueError) as e:
            a.view(src)
        assert str(e.value) == "view supports language 1 specs only"
    # A language 2 spec without a language 2 rule still has a view.
    plain = pycanonic.compile_spec(_spec({"name": "n"}), SCHEMA)
    assert plain.view({"n": "a"}).name == "a"


def test_context_mapping_and_errors():
    import types

    import pytest

    a = pycanonic.compile_spec(_spec({"name": "$input.who", "kind": {"from": "k", "when": {"path": "$input.on", "eq": True}}}), SCHEMA)
    body = {"k": "x"}
    raw = json.dumps(body).encode()
    ctx = types.MappingProxyType({"who": "Ada", "on": True})
    want = {"name": "Ada", "kind": "x"}
    assert a.map(raw, context=ctx) == a.map(body, context=ctx) == json.loads(a.map_json(raw, context=ctx)) == want
    assert a.map(raw) == a.map(body) == json.loads(a.map_json(raw)) == {"name": None, "kind": None}
    for bad in [[1], "x"]:
        with pytest.raises(TypeError):
            a.map(raw, context=bad)
        with pytest.raises(TypeError):
            a.map(body, context=bad)
