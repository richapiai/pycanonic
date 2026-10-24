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
    def strip_honorific(v):
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
    [lambda v: {v}, lambda v: 1 / 0],
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

    def pair(v):
        seen.append(v)
        return "|".join(map(str, v))

    a = _core.compile(spec({"pair": {"from": ["a", "b.c"], "transform": "pair"}}), CANONICAL, {"pair": pair})
    assert a.map(b'{"a": "x", "b": {"c": 2}}')["pair"] == "x|2"
    assert a.map({"a": "x"})["pair"] == "x|None"
    assert seen == [["x", 2], ["x", None]]


def test_an_object_path_hands_the_callable_a_dict():
    seen = []

    def keys(v):
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
    a = _core.compile(spec({"full_name": {"from": "n", "transform": "x"}}), CANONICAL, {"x": lambda v: v})
    with pytest.raises(ValueError, match='transform "x" is not registered'):
        _core.load_plan(a.plan_json(), None)
    assert _core.load_plan(a.plan_json(), {"x": lambda v: v}).map({"n": "y"})["full_name"] == "y"


def test_a_bare_name_calls_f_value_and_an_argument_calls_f_value_arg():
    a = _core.compile(
        spec({"full_name": {"from": "n", "transform": "one"}, "pair": {"from": "n", "transform": {"two": None}}}),
        CANONICAL,
        {"one": lambda v: f"<{v}>", "two": lambda v, arg: f"{v}/{arg}"},
    )
    assert a.map({"n": "x"}) == {"full_name": "<x>", "pair": "x/None", "org_keys": None}


def test_a_5000_digit_int_reads_as_a_float_like_json_parse():
    seen = []

    def see(v):
        seen.append(v)
        return "ok"

    a = _core.compile(
        spec({"full_name": "n", "pair": {"from": "n", "transform": "see"}}), CANONICAL, {"see": see}
    )
    out = a.map(b'{"n":' + b"1" * 5000 + b"}")
    assert out["full_name"] == float("inf")
    assert out["pair"] == "ok"
    big = 10**5000
    out = a.map({"n": -big})
    assert out["full_name"] == -big
    assert out["pair"] == "ok"
    assert seen == [float("inf"), float("-inf")]
    assert all(f["errors"] == 0 for f in a.stats_snapshot()["fields"].values())


def test_a_cyclic_container_is_null_and_one_error_not_a_crash():
    a = _core.compile(
        spec({"org_keys": {"from": "org", "transform": "keys"}}), CANONICAL, {"keys": lambda v: ",".join(v)}
    )
    org = {"name": "Acme"}
    org["self"] = org
    assert a.map({"org": org})["org_keys"] is None
    assert a.stats_snapshot()["fields"]["org_keys"] == {"fill": 0, "errors": 1}


def test_a_base_exception_that_is_not_an_exception_propagates():
    def stop(v):
        raise KeyboardInterrupt

    a = _core.compile(spec({"full_name": {"from": "n", "transform": "stop"}}), CANONICAL, {"stop": stop})
    with pytest.raises(KeyboardInterrupt):
        a.map({"n": "x"})
    with pytest.raises(KeyboardInterrupt):
        a.map(b'{"n": "x"}')
    with pytest.raises(KeyboardInterrupt):
        a.map_json(b'{"n": "x"}')
    assert a.map({}) == {"full_name": None, "pair": None, "org_keys": None}


def test_a_container_that_refers_to_itself_twice_is_null_and_one_error_not_a_hang():
    a = _core.compile(
        spec({"org_keys": {"from": "org", "transform": "keys"}}), CANONICAL, {"keys": lambda v: ",".join(v)}
    )
    org = {}
    org["a"] = org["b"] = org
    assert a.map({"org": org})["org_keys"] is None
    assert a.stats_snapshot()["fields"]["org_keys"] == {"fill": 0, "errors": 1}


def test_a_container_that_shares_its_children_is_null_and_one_error_not_a_hang():
    # No cycle, but 2**60 paths: the copy for the transform stops at its node budget.
    a = _core.compile(
        spec({"org_keys": {"from": "org", "transform": "keys"}}), CANONICAL, {"keys": lambda v: ",".join(v)}
    )
    org = {}
    for _ in range(60):
        org = {"a": org, "b": org}
    assert a.map({"org": org})["org_keys"] is None
    assert a.stats_snapshot()["fields"]["org_keys"] == {"fill": 0, "errors": 1}


def test_language_2_string_transforms_in_default_transforms():
    from pycanonic import default_transforms as T

    assert T["regex"]("https://x.com/company/acme/?a=1", "/company/([^/?#]+)") == "acme"
    assert T["strip_prefix"]("urn:li:person:", "urn:li:person:") is None
    assert T["strip_suffix"]("repo.git", ".git") == "repo"
    assert T["replace"]("a-b", {"from": "-", "to": " "}) == "a b"
    assert T["lookup"](1, {"table": {"1": "one"}}) == "one"
    assert T["lookup"]("LOVE", {"table": {"LIKE": "liked"}, "else": "keep"}) == "LOVE"
    assert T["null_if"](0.0, ["", "N/A", 0]) is None
    assert T["upper"]("àda") == "ÀDA"
    assert T["title"]("ada LOVELACE") == "Ada Lovelace"
    assert T["split"]("Ada King Lovelace", {"sep": " ", "index": -1}) == "Lovelace"
    assert T["to_string"](12) == "12"
    assert T["upper"]([1]) is None
    with pytest.raises(ValueError, match="invalid argument"):
        T["regex"]("x", "(unclosed")
    with pytest.raises(ValueError, match="invalid argument"):
        T["split"]("x", {"sep": " "})


def test_language_2_transform_names_need_language_2():
    fields = {"full_name": {"from": "name", "transform": "upper"}}
    with pytest.raises(ValueError, match='unknown transform "upper"'):
        _core.compile(spec(fields), CANONICAL)
    a = _core.compile({**spec(fields), "language": 2}, CANONICAL)
    for body in ({"name": "àda"}, json.dumps({"name": "àda"}).encode()):
        assert a.map(body)["full_name"] == "ÀDA"
    assert json.loads(a.map_json(b'{"name": "\\u00e0da"}'))["full_name"] == "ÀDA"
    # A custom transform with a new name works in language 1.
    a = _core.compile(spec(fields), CANONICAL, {"upper": lambda v: "custom"})
    assert a.map({"name": "x"})["full_name"] == "custom"
