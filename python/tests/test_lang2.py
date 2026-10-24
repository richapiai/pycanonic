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


def _reject_artifact():
    schema = {"type": "object", "properties": {"email": {"type": "string", "semantic": "email", "routing": True}, "name": {"type": "string"}}}
    spec = _spec({"email": "contact.email", "name": "data.name"})
    spec["reject"] = [{"missing": "data.name"}]
    return pycanonic.compile_spec(spec, schema)


def test_rejected_is_exception():
    import pycanonic._core

    assert issubclass(pycanonic.Rejected, Exception)
    assert pycanonic.Rejected is pycanonic._core.Rejected
    assert pycanonic.Rejected.__module__ == "pycanonic._core"


def test_rejected_counts():
    import pytest

    body = {"contact": {"email": "ada@example.com"}, "data": {}}
    raw = json.dumps(body).encode()
    for way in [lambda a: a.map(raw), lambda a: a.map(body), lambda a: a.map_json(raw)]:
        a = _reject_artifact()
        with pytest.raises(pycanonic.Rejected) as e:
            way(a)
        assert e.value.reason == '{"missing":"data.name"}'
        assert str(e.value) == e.value.reason
        s = a.stats_snapshot()
        assert s["rejected"] == 1
        assert s["responses"] == 0
        a.stats.reset()
        assert a.stats.snapshot()["rejected"] == 0
    assert "rejected" not in pycanonic.compile_spec(_spec({"name": "n"}), SCHEMA).stats_snapshot()


def test_extract_ignores_reject():
    a = _reject_artifact()
    raw = json.dumps({"contact": {"email": "ada@example.com"}, "data": {}}).encode()
    assert a.extract(raw) == {"email": "ada@example.com"}
    s = a.stream()
    s.push(raw)
    assert s.result() == {"email": "ada@example.com"}


def _ways_with(a, body, ctx):
    raw = json.dumps(body).encode()
    return [
        lambda: a.map(raw, context=ctx),
        lambda: a.map(body, context=ctx),
        lambda: json.loads(a.map_json(raw, context=ctx)),
    ]


def test_context_is_validated_the_same_on_every_path():
    import math
    import types

    import pytest

    a = pycanonic.compile_spec(_spec({"name": "$input.who", "kind": "k"}), SCHEMA)
    body = {"k": "x"}
    good = {"who": "Ada", "deep": [1, 2.5, {"n": None, "b": True}], "big": 2**70}
    for way in _ways_with(a, body, good):
        assert way() == {"name": "Ada", "kind": "x"}
    bad = [
        {1: "x"},
        {"who": {1: "x"}},
        {"who": math.nan},
        {"who": [math.inf]},
        {"who": object()},
        {"who": types.MappingProxyType({"a": 1})},
        [1],
        "x",
    ]
    for ctx in bad:
        for way in _ways_with(a, body, ctx):
            with pytest.raises(TypeError) as e:
                way()
            assert str(e.value).startswith("context must be a JSON object: "), (ctx, str(e.value))


def test_context_is_ignored_when_the_plan_never_reads_input():
    import math

    a = pycanonic.compile_spec(_spec({"name": "n", "kind": "k"}), SCHEMA)
    for ctx in [{"who": math.nan}, {1: object()}, [1], "x"]:
        for way in _ways_with(a, {"n": "a", "k": "x"}, ctx):
            assert way() == {"name": "a", "kind": "x"}


def test_reject_and_when_on_input_validate_the_context():
    import math

    import pytest

    spec = _spec({"name": "n"})
    spec["reject"] = [{"path": "$input.stop", "eq": True}]
    a = pycanonic.compile_spec(spec, SCHEMA)
    for way in _ways_with(a, {"n": "a"}, {"stop": math.nan}):
        with pytest.raises(TypeError):
            way()
    for way in _ways_with(a, {"n": "a"}, {"stop": True}):
        with pytest.raises(pycanonic.Rejected):
            way()
    b = pycanonic.compile_spec(_spec({"name": {"from": "n", "when": {"path": "$input.on", "eq": True}}}), SCHEMA)
    for way in _ways_with(b, {"n": "a"}, {"on": math.nan}):
        with pytest.raises(TypeError):
            way()


def test_rejected_reason_comes_from_args():
    import pickle

    import pytest

    assert pycanonic.Rejected("x").reason == "x"
    a = _reject_artifact()
    with pytest.raises(pycanonic.Rejected) as e:
        a.map(b'{"data": {}}')
    back = pickle.loads(pickle.dumps(e.value))
    assert type(back) is pycanonic.Rejected
    assert back.reason == e.value.reason == '{"missing":"data.name"}'
