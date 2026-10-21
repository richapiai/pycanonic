import json

import pytest

from pycanonic import _core

CANONICAL = {
    "type": "object",
    "properties": {
        "full_name": {"type": "string"},
        "pair": {"type": "string"},
        "org_keys": {"type": "string"},
    },
}


def spec(fields):
    return {"provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted", "fields": fields}


def test_strip_honorific_from_the_docs():
    def strip_honorific(v, arg):
        import re

        return re.sub(r"^(Dr|Mr|Mrs|Ms)\.? ", "", v) if isinstance(v, str) else None

    a = _core.compile(
        spec({"full_name": {"from": "name", "transform": ["trim", "strip_honorific"]}}),
        CANONICAL,
        {"strip_honorific": strip_honorific},
    )
    assert a.map({"name": " Dr. Ada Lovelace "})["full_name"] == "Ada Lovelace"
    assert a.map(b'{"name": " Dr. Ada Lovelace "}')["full_name"] == "Ada Lovelace"


def test_arg_and_value_types_reach_the_callable():
    seen = []

    def tag(v, arg):
        seen.append((v, arg))
        return f"{v}{arg}"

    a = _core.compile(
        spec({"full_name": {"from": "n", "transform": {"tag": "!"}}}), CANONICAL, {"tag": tag}
    )
    assert a.map(b'{"n": 7}')["full_name"] == "7!"
    assert a.map({"n": True})["full_name"] == "True!"
    assert seen == [(7, "!"), (True, "!")]


@pytest.mark.parametrize(
    "fn",
    [lambda v, arg: {v}, lambda v, arg: 1 / 0],
    ids=["returns_a_set", "raises"],
)
def test_bad_result_or_exception_is_null_and_one_error(fn, capsys):
    a = _core.compile(spec({"full_name": {"from": "n", "transform": "bad"}}), CANONICAL, {"bad": fn})
    assert a.map(b'{"n": "x"}')["full_name"] is None
    assert a.stats_snapshot()["fields"]["full_name"] == {"fill": 0, "errors": 1}
    assert json.loads(a.map_json(b'{"n": "x"}'))["full_name"] is None
    assert a.stats_snapshot()["fields"]["full_name"] == {"fill": 0, "errors": 2}
    out, err = capsys.readouterr()
    assert out == err == ""


def test_from_many_hands_the_callable_a_list():
    seen = []

    def pair(v, arg):
        seen.append(v)
        return "|".join(map(str, v))

    a = _core.compile(spec({"pair": {"from": ["a", "b.c"], "transform": "pair"}}), CANONICAL, {"pair": pair})
    assert a.map(b'{"a": "x", "b": {"c": 2}}')["pair"] == "x|2"
    assert a.map({"a": "x"})["pair"] == "x|None"
    assert seen == [["x", 2], ["x", None]]


def test_an_object_path_hands_the_callable_a_dict():
    seen = []

    def keys(v, arg):
        seen.append(v)
        return ",".join(v)

    a = _core.compile(spec({"org_keys": {"from": "org", "transform": "keys"}}), CANONICAL, {"keys": keys})
    body = {"org": {"name": "Acme", "size": [1, {"x": None}]}}
    assert a.map(json.dumps(body).encode())["org_keys"] == "name,size"
    assert a.map(body)["org_keys"] == "name,size"
    assert json.loads(a.map_json(json.dumps(body)))["org_keys"] == "name,size"
    assert seen == [body["org"]] * 3


def test_unknown_and_unregistered_transforms_raise():
    with pytest.raises(ValueError, match='unknown transform "shout"'):
        _core.compile(spec({"full_name": {"from": "n", "transform": "shout"}}), CANONICAL, None)
    a = _core.compile(spec({"full_name": {"from": "n", "transform": "x"}}), CANONICAL, {"x": lambda v, a: v})
    with pytest.raises(ValueError, match='transform "x" is not registered'):
        _core.load_plan(a.plan_json(), None)
    assert _core.load_plan(a.plan_json(), {"x": lambda v, a: v}).map({"n": "y"})["full_name"] == "y"
