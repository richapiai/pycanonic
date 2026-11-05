import json

import pytest

from pycanonic import _core

# The quick start from canonic/README.md.
CANONICAL = {
    "type": "object",
    "properties": {
        "full_name": {"type": "string"},
        "email": {"type": "string", "semantic": "email", "routing": True},
        "company": {
            "type": "object",
            "properties": {
                "domain": {"type": "string", "semantic": "domain", "routing": True},
                "headcount": {"type": "integer"},
            },
        },
    },
}

SPEC = {
    "provider": "acme",
    "endpoint": "person.enrich",
    "version": 1,
    "status": "promoted",
    "fields": {
        "full_name": "profile.fullName",
        "email": {
            "coalesce": ["contact.workEmail", "contact.emails[0].address"],
            "transform": ["trim", "lower"],
        },
        "company.domain": {"from": "org.website", "transform": "to_domain"},
        "company.headcount": {"from": "org.size", "transform": {"cast": "integer"}},
    },
}

BODY = (
    '{"profile":{"fullName":"Ada Lovelace"},"contact":{"workEmail":" Ada@Acme.com "},'
    '"org":{"website":"https://www.acme.com/about","size":"120"}}'
)
WANT = {
    "full_name": "Ada Lovelace",
    "email": "ada@acme.com",
    "company": {"domain": "acme.com", "headcount": 120},
}


def test_quick_start():
    acme = _core.compile(SPEC, CANONICAL, None)
    assert acme.map(json.loads(BODY)) == WANT
    assert acme.routing == ["email", "company.domain"]
    assert (acme.provider, acme.endpoint, acme.version, acme.status) == (
        "acme",
        "person.enrich",
        1,
        "promoted",
    )
    assert "email" in acme.source
    snap = acme.stats_snapshot()
    assert snap["responses"] == 1
    assert snap["fields"]["full_name"] == {"fill": 1, "errors": 0}
    assert list(snap["fields"]) == ["full_name", "email", "company.domain", "company.headcount"]
    acme.stats_reset()
    assert acme.stats_snapshot()["responses"] == 0


def test_spec_and_schema_as_json_text():
    acme = _core.compile(json.dumps(SPEC), json.dumps(CANONICAL).encode(), None)
    assert acme.map(BODY) == WANT


def test_bytes_str_and_dict_agree_in_schema_order(bench):
    a, schema, bodies = bench
    for raw, want in zip(bodies["raws"], bodies["expected"]):
        out = a.map(raw.encode())
        assert out == want
        assert list(out) == list(schema["properties"])
        assert json.dumps(out) == json.dumps(want)
        assert a.map(raw) == out
        assert a.map(json.loads(raw)) == out
        assert json.loads(a.map_json(raw.encode())) == out
        assert json.loads(a.map_json(raw)) == out


def test_map_json_returns_bytes(bench):
    a, _, bodies = bench
    assert isinstance(a.map_json(bodies["raws"][0]), bytes)


def test_unknown_field_raises_the_exact_message():
    spec = dict(SPEC, fields={"emial": "contact.workEmail"})
    with pytest.raises(ValueError) as e:
        _core.compile(spec, CANONICAL, None)
    assert str(e.value) == 'acme person.enrich v1: fields["emial"]: canonical schema has no field "emial"'


def test_lone_surrogate_maps_to_replacement_character():
    acme = _core.compile(SPEC, CANONICAL, None)
    raw = '{"profile":{"fullName":"a\\ud800b"}}'
    assert acme.map(raw.encode())["full_name"] == "a\ufffdb"
    assert acme.map(raw)["full_name"] == "a\ufffdb"
    assert json.loads(acme.map_json(raw))["full_name"] == "a\ufffdb"
    # A str that holds the lone surrogate itself, not its escape.
    assert acme.map('{"profile":{"fullName":"a\ud800b"}}')["full_name"] == "a\ufffdb"


@pytest.mark.parametrize("n", [2**53 + 1, 2**70, -(2**64) - 3])
def test_big_ints_pass_through_exactly(n):
    acme = _core.compile(SPEC, CANONICAL, None)
    raw = '{"profile":{"fullName":%d}}' % n
    assert acme.map(raw.encode())["full_name"] == n
    assert acme.map({"profile": {"fullName": n}})["full_name"] == n
    assert json.loads(acme.map_json(raw))["full_name"] == n


def test_dict_input_passes_containers_through_as_the_same_object():
    acme = _core.compile(SPEC, CANONICAL, None)
    name = {"given": "Ada"}
    assert acme.map({"profile": {"fullName": name}})["full_name"] is name


def test_unparseable_and_absent_input_maps_to_nulls():
    acme = _core.compile(SPEC, CANONICAL, None)
    empty = {"full_name": None, "email": None, "company": {"domain": None, "headcount": None}}
    assert acme.map(b"{not json") == empty
    assert acme.map(None) == empty
    assert acme.map([1, 2]) == empty


def test_plan_json_round_trips():
    acme = _core.compile(SPEC, CANONICAL, None)
    loaded = _core.load_plan(acme.plan_json(), None)
    assert loaded.map(BODY) == WANT
    assert loaded.routing == acme.routing


def test_validate_spec_and_canonical_return_issues():
    assert _core.validate_spec(SPEC) == []
    issues = _core.validate_spec({"provider": "acme"})
    assert issues and all(isinstance(i, tuple) and len(i) == 2 for i in issues)
    assert _core.validate_canonical(CANONICAL) == []
    assert _core.validate_canonical({"type": "string"}) != []


def test_default_transform_and_validator():
    assert _core.default_transform("trim", " x ") == "x"
    assert _core.default_transform("cast", "12", "integer") == 12
    assert _core.default_transform("lower", 3) is None
    assert _core.default_transform("join", ["a", None, 2], "-") == "a-2"
    with pytest.raises(ValueError):
        _core.default_transform("shout", "x")
    with pytest.raises(ValueError):
        _core.default_transform("cast", "1", "bogus")
    assert _core.validator("email", "a@b.co") == "a@b.co"
    assert _core.validator("email", "nope") is None
    with pytest.raises(ValueError):
        _core.validator("colour", "x")


def test_a_lone_surrogate_in_a_dict_str_maps_like_the_escape_in_bytes(bench):
    a, _, _ = bench
    from_dict = a.map({"profile": {"fullName": "a\udc00b"}})
    from_bytes = a.map(b'{"profile":{"fullName":"a\\udc00b"}}')
    assert from_dict["full_name"] == from_bytes["full_name"] == "a\ufffdb"
    assert from_dict == from_bytes


def test_buffers_map_like_bytes():
    from types import MappingProxyType

    acme = _core.compile(SPEC, CANONICAL, None)
    routed = acme.extract(BODY)
    for src in (bytearray(BODY.encode()), memoryview(BODY.encode())):
        assert acme.map(src) == WANT
        assert json.loads(acme.map_json(src)) == WANT
        assert acme.view(src).to_dict() == WANT
        assert acme.extract(src) == routed
        s = acme.stream()
        s.push(src)
        assert s.result() == routed
    proxy = MappingProxyType(json.loads(BODY))
    assert acme.map(proxy) == WANT
    assert acme.view(proxy).to_dict() == WANT


@pytest.mark.parametrize("root", [7, None, True, 1.5])
def test_a_scalar_root_maps_like_its_json_text(root):
    acme = _core.compile(SPEC, CANONICAL, None)
    assert acme.map(root) == acme.map(json.dumps(root))


@pytest.mark.parametrize("root", [object(), {1, 2}, frozenset(), range(3)])
def test_a_root_that_is_not_json_raises(root):
    acme = _core.compile(SPEC, CANONICAL, None)
    with pytest.raises(TypeError, match="expected JSON text"):
        acme.map(root)
    with pytest.raises(TypeError, match="expected JSON text"):
        acme.view(root)


def test_a_body_too_deep_where_the_plan_reads_maps_to_all_nulls():
    canonical = {"type": "object", "properties": {"a": {"type": "string"}, "d": {}}}
    spec = {"provider": "p", "endpoint": "e", "version": 1, "status": "promoted", "fields": {"a": "a", "d": "deep"}}
    a = _core.compile(spec, canonical, None)
    deep = "[" * 600 + "]" * 600
    assert a.map('{"a":"hi","deep":' + deep + "}") == {"a": None, "d": None}
    assert a.map('{"a":"hi","other":' + deep + "}") == {"a": "hi", "d": None}
    with pytest.raises(ValueError, match="Depth"):
        _core.shape(deep.encode())


def test_cast_of_an_integer_string_past_2_53_goes_through_a_double():
    canonical = {"type": "object", "properties": {"n": {"type": "integer"}, "s": {"type": "integer"}}}
    cast = {"cast": "integer"}
    fields = {"n": {"from": "n", "transform": cast}, "s": {"from": "s", "transform": cast}}
    spec = {"provider": "p", "endpoint": "e", "version": 1, "status": "promoted", "fields": fields}
    a = _core.compile(spec, canonical, None)
    out = a.map('{"n":9007199254740993,"s":"9007199254740993"}')
    assert out == {"n": 9007199254740993, "s": 9007199254740992}


def test_load_plan_checks_the_format_stamp():
    acme = _core.compile(SPEC, CANONICAL, None)
    plan = json.loads(acme.plan_json())
    assert plan["format"] == 1
    del plan["format"]
    with pytest.raises(ValueError, match="missing field `format`"):
        _core.load_plan(json.dumps(plan), None)
    plan["format"] = 2
    assert json.loads(_core.load_plan(json.dumps(plan), None).plan_json())["format"] == 2
    plan["format"] = 3
    with pytest.raises(ValueError, match=r"plan format 3 is not supported\. This build reads format 1 or 2\. Rebuild the plan\."):
        _core.load_plan(json.dumps(plan), None)


def _on_missing_spec(fields):
    return {"language": 2, "provider": "acme", "endpoint": "items", "version": 1, "status": "promoted", "fields": fields}


def test_on_missing_custom_gets_none():
    import pycanonic

    schema = {"type": "object", "properties": {"slug": {"type": "string"}}}
    seen = []

    def f(v):
        seen.append(v)
        return "none" if v is None else v.upper()

    spec = _on_missing_spec({"slug": {"from": "name", "transform": "f", "on_missing": "transform"}})
    a = pycanonic.compile_spec(spec, schema, transforms={"f": f})
    assert a.map({}) == {"slug": "none"}
    assert a.map({"name": None}) == {"slug": "none"}
    assert a.map({"name": "ada"}) == {"slug": "ADA"}
    assert seen == [None, None, "ada"]
    assert json.loads(a.plan_json())["format"] == 2
    assert "on_missing" in a.plan_json()
    # Without the option the transform is skipped.
    plain = pycanonic.compile_spec(
        _on_missing_spec({"slug": {"from": "name", "transform": "f"}}), schema, transforms={"f": f}
    )
    assert plain.map({}) == {"slug": None}
    assert "on_missing" not in plain.plan_json()


def test_on_missing_custom_container_gets_none():
    import pycanonic

    schema = {
        "type": "object",
        "properties": {"hq": {"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "string"}}}},
    }
    seen = []

    def f(v):
        seen.append(v)
        return {"a": None, "b": None} if v is None else v

    spec = _on_missing_spec({"hq": {"from": "office", "transform": "f", "on_missing": "transform"}})
    a = pycanonic.compile_spec(spec, schema, transforms={"f": f})
    assert a.map({}) == {"hq": {"a": None, "b": None}}
    assert a.map({"office": {"a": 1, "b": "x"}}) == {"hq": {"a": 1, "b": "x"}}
    assert seen[0] is None
    assert json.loads(a.map_json(b"{}")) == {"hq": {"a": None, "b": None}}


def test_on_missing_null_result_is_a_miss_not_an_error():
    import pycanonic

    schema = {"type": "object", "properties": {"slug": {"type": "string"}}}
    spec = _on_missing_spec(
        {"slug": {"from": "name", "transform": "f", "on_missing": "transform", "default": "n/a"}}
    )
    a = pycanonic.compile_spec(spec, schema, transforms={"f": lambda v: None})
    assert a.map({}) == {"slug": "n/a"}
    assert a.stats_snapshot()["fields"]["slug"]["errors"] == 0


def test_on_missing_routing_field_fails_compile():
    import pycanonic

    schema = {"type": "object", "properties": {"email": {"type": "string", "semantic": "email", "routing": True}}}
    spec = _on_missing_spec({"email": {"from": "e", "transform": "f", "on_missing": "transform"}})
    with pytest.raises(pycanonic.SpecError, match="routing fields can't have on_missing"):
        pycanonic.compile_spec(spec, schema, transforms={"f": lambda v: v})


def test_on_missing_container_null_result_and_plan_round_trip():
    import pycanonic

    schema = {"type": "object", "properties": {"hq": {"type": "object", "properties": {"a": {"type": "integer"}}}}}
    spec = _on_missing_spec({"hq": {"from": "office", "transform": "f", "on_missing": "transform"}})
    a = pycanonic.compile_spec(spec, schema, transforms={"f": lambda v: v})
    assert a.map({}) == {"hq": None}
    assert a.stats_snapshot()["fields"]["hq"]["errors"] == 0
    b = pycanonic.load(a.plan_json(), transforms={"f": lambda v: {"a": 7} if v is None else v})
    assert b.map({}) == {"hq": {"a": 7}}
