"""Spec language 2 rules through the Python API. Every map runs `map(bytes)`, `map(dict)` and `map_json`."""

import json

import pytest

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


def _ways(a, body):
    raw = json.dumps(body).encode()
    return [lambda: a.map(raw), lambda: a.map(body), lambda: json.loads(a.map_json(raw))]


def _errors(a, field):
    return a.stats_snapshot()["fields"][field]["errors"]


TEAMS = {
    "type": "object",
    "properties": {
        "teams": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {"team": {"type": "string"}, "members": {"type": "array", "items": {"type": "string"}}},
            },
        }
    },
}


def by_team(people):
    out = {}
    for p in people:
        out.setdefault(p["team"], []).append(p["name"])
    return [{"team": t, "members": m} for t, m in out.items()]


def test_container_transform_returns_list():
    body = {"staff": [{"name": "ada", "team": "core"}, {"name": "bo", "team": "web"}, {"name": "cy", "team": "core"}]}
    want = {"teams": [{"team": "core", "members": ["ada", "cy"]}, {"team": "web", "members": ["bo"]}]}
    spec = _spec({"teams[]": {"from": "staff", "transform": "by_team"}})
    for way in _ways(pycanonic.compile_spec(spec, TEAMS, transforms={"by_team": by_team}), body):
        assert way() == want
    a = pycanonic.compile_spec(spec, TEAMS, transforms={"by_team": by_team})
    b = pycanonic.load(a.plan_json(), transforms={"by_team": by_team})
    assert json.loads(a.plan_json())["format"] == 2
    for way in _ways(b, body):
        assert way() == want
    assert b.stats_snapshot()["fields"]["teams[]"] == {"fill": 3, "errors": 0}


HQ = {
    "type": "object",
    "properties": {"hq": {"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "string"}}}},
}


def test_container_transform_bad_shape_is_null():
    bad = [lambda v: {"extra": 1}, lambda v: [1], lambda v: {"a": [1]}, lambda v: {"a": {1, 2}}, lambda v: None]

    def boom(v):
        raise ValueError("no")

    for f in [*bad, boom]:
        a = pycanonic.compile_spec(_spec({"hq": {"from": "office", "transform": "f"}}), HQ, transforms={"f": f})
        for way in _ways(a, {"office": {"x": 1}}):
            assert way() == {"hq": None}
        assert _errors(a, "hq") == 3


def test_container_transform_fills_missing_keys():
    schema = {
        "type": "object",
        "properties": {
            "hq": {
                "type": "object",
                "properties": {
                    "a": {"type": "integer"},
                    "b": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}, "x-empty": "[]"},
                    "gone": {"type": "string", "x-empty": "missing"},
                    "meta": {"type": "object", "x-open": True},
                },
            }
        },
    }
    a = pycanonic.compile_spec(
        _spec({"hq": {"from": "office", "transform": "f"}}), schema, transforms={"f": lambda v: {"meta": {"any": [1]}, "a": 1}}
    )
    for way in _ways(a, {"office": {}}):
        out = way()
        assert out == {"hq": {"a": 1, "b": None, "tags": [], "meta": {"any": [1]}}}
        assert list(out["hq"]) == ["a", "b", "tags", "meta"]
    # A missing source does not call the transform.
    for way in _ways(a, {}):
        assert way() == {"hq": None}
    assert _errors(a, "hq") == 0


def test_container_transform_is_language_2_only():
    import pytest

    lang1 = {**_spec({"hq": {"from": "office", "transform": "f"}}), "language": 1}
    with pytest.raises(pycanonic.SpecError, match="canonical field is an object"):
        pycanonic.compile_spec(lang1, HQ, transforms={"f": lambda v: v})
    with pytest.raises(pycanonic.SpecError, match="a container transform takes only from and transform"):
        pycanonic.compile_spec(_spec({"teams[]": {"from": "s", "transform": "f", "limit": 2}}), TEAMS, transforms={"f": by_team})


def test_scalar_transform_still_rejects_dict():
    for lang in [1, 2]:
        spec = {**_spec({"name": {"from": "n", "transform": "f"}}), "language": lang}
        a = pycanonic.compile_spec(spec, SCHEMA, transforms={"f": lambda v: {"a": v}})
        for way in _ways(a, {"n": "x"}):
            assert way() == {"name": None, "kind": None}
        assert _errors(a, "name") == 3
    # Language 1: a custom transform on an array field still maps each element.
    spec = {**_spec({"teams[]": {"from": "t", "transform": "f"}}), "language": 1}
    tags = {"type": "object", "properties": {"teams": {"type": "array", "items": {"type": "string"}}}}
    a = pycanonic.compile_spec(spec, tags, transforms={"f": lambda v: v.upper()})
    for way in _ways(a, {"t": ["a", "b"]}):
        assert way() == {"teams": ["A", "B"]}


def test_definition_stats_follow_each_use():
    spec = {
        **_spec({"people[]": {"from": "p", "item": {"$ref": "person"}}, "lead[]": {"from": "l", "item": {"$ref": "person"}}}),
        "definitions": {"person": {"name": "n"}},
    }
    person = {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}}}}
    a = pycanonic.compile_spec(spec, {"type": "object", "properties": {"people": person, "lead": person}})
    for way in _ways(a, {"p": [{"n": "a"}], "l": [{}]}):
        assert way() == {"people": [{"name": "a"}], "lead": [{"name": None}]}
    fields = a.stats_snapshot()["fields"]
    assert fields["people[].name"]["fill"] == 3
    assert fields["lead[].name"]["fill"] == 0


def test_array_root_three_paths():
    schema = {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}}}, "x-empty": "[]"}
    a = pycanonic.compile_spec(_spec({"[]": {"from": "$", "item": {"name": "n"}}}), schema)
    for body, want in [([{"n": "a"}, {"x": 1}], [{"name": "a"}, {"name": None}]), (None, []), ({"n": "a"}, None)]:
        for way in _ways(a, body):
            assert way() == want
    assert list(a.stats_snapshot()["fields"]) == ["[]", "[].name"]
    assert a.extract(b'[{"n": "a"}]') == {}
    r = pycanonic.compile_spec({**_spec({"[]": {"from": "$", "item": {"name": "n"}}}), "reject": [{"empty": "$"}]}, schema)
    for way in _ways(r, []):
        with pytest.raises(pycanonic.Rejected):
            way()
    assert r.stats_snapshot()["rejected"] == 3
    s = a.stream()
    s.push(b'[{"n": "a"}]')
    assert s.result() == {}


def test_array_root_container_transform():
    body = {"staff": [{"name": "ada", "team": "core"}, {"name": "bo", "team": "web"}]}
    spec = _spec({"[]": {"from": "staff", "transform": "by_team"}})
    a = pycanonic.compile_spec(spec, TEAMS["properties"]["teams"], transforms={"by_team": by_team})
    for way in _ways(a, body):
        assert way() == [{"team": "core", "members": ["ada"]}, {"team": "web", "members": ["bo"]}]
