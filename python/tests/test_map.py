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
