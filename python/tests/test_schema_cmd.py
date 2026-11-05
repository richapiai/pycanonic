"""`canonic schema`: the canonical schema derived from the default provider's samples, the sample pairs and the comparison rule."""

from __future__ import annotations

import json
import random
from pathlib import Path
from typing import Any

import pytest

import pycanonic
from pycanonic.cli.__main__ import main
from pycanonic.cli.fit.compare import same
from pycanonic.cli.fit.derive import derive_schema, flags
from pycanonic.cli.fit.pairs import Pair, load_pairs

def _types(node: dict[str, Any]) -> list[str]:
    t = node.get("type", [])
    return t if isinstance(t, list) else [t]


def _fields(node: dict[str, Any]) -> dict[str, Any]:
    """A const per field of an object node; leaves copy their own key."""
    return {
        json.dumps([k]): {"const": None} if {"object", "array"} & set(_types(n)) else json.dumps([k])
        for k, n in node["properties"].items()
    }


def derive(samples: list[Any]) -> dict[str, Any]:
    """derive_schema, checked to compile with a trivial language 2 spec: `_fields` for an object
    root, and a `"[]"` rule from `$` for an array root."""
    schema = derive_schema(samples)
    if "properties" in schema:
        fields = _fields(schema)
    else:
        items = schema.get("items", {})
        fields = {"[]": {"from": "$", "item": _fields(items)} if "properties" in items else {"from": "$"}}
    spec = {"language": 2, "provider": "acme", "endpoint": "orgs", "version": 1, "status": "promoted", "fields": fields}
    pycanonic.compile_spec(spec, schema)
    return schema


def test_key_order_first_appearance() -> None:
    s = derive([{"b": 1, "a": {"y": 1}}, {"c": 1, "a": {"x": 1, "y": 2}, "b": 2}])
    assert list(s["properties"]) == ["b", "a", "c"]
    assert list(s["properties"]["a"]["properties"]) == ["y", "x"]


def test_mixed_types_listed() -> None:
    s = derive([{"a": 1}, {"a": "x"}, {"a": 2.5}])
    assert s["properties"]["a"]["type"] == ["number", "string"]
    assert ("a", "mixed types") in flags(s)


def test_x_empty_null_vs_list() -> None:
    s = derive([{"t": ["a"], "u": []}, {"u": ["b"], "t": []}, {"u": [], "k": 1}])
    t, u = s["properties"]["t"], s["properties"]["u"]
    assert t["x-mixed-empty"] is True and t["type"] == "array"
    assert ("t", "mixed empty") in flags(s)
    assert u == {"type": "array", "items": {"type": "string"}, "x-empty": "[]"}


def test_object_empty_forms() -> None:
    s = derive([{"o": {"a": 1}, "p": {"a": 1}}, {"o": {}, "p": {"a": 2}}, {"o": {}, "p": {"a": 3}}, {"o": None, "p": None}])
    assert s["properties"]["o"]["x-empty"] == "{}" and s["properties"]["o"]["x-mixed-empty"] is True
    assert s["properties"]["p"] == {"type": "object", "properties": {"a": {"type": "number"}}, "x-empty": "null"}


def test_leaf_missing() -> None:
    s = derive([{"a": 1, "b": "x"}, {"b": "y"}, {"a": 2, "b": None}])
    assert s["properties"]["a"] == {"type": "number", "x-empty": "missing"}
    assert s["properties"]["b"] == {"type": "string", "x-empty": "null"}


def test_always_null_has_no_type() -> None:
    s = derive([{"a": None}, {"a": None}])
    assert s["properties"]["a"] == {"x-empty": "null"}
    assert flags(s) == [("a", "always null")]


def test_open_map() -> None:
    rng = random.Random(7)
    keys = [f"k{i}" for i in range(100)]
    samples = [{"id": i, "m": {k: 1 for k in rng.sample(keys, 3)}} for i in range(30)]
    s = derive(samples)
    assert s["properties"]["m"] == {"type": "object", "x-open": True}
    # An object with many optional keys that each sample mostly carries stays closed.
    many = [f"f{i}" for i in range(25)]
    closed = derive([{"m": {k: 1 for k in many if k != many[i]}} for i in range(25)])
    assert "x-open" not in closed["properties"]["m"]


def test_bare_array_root() -> None:
    s = derive([[{"a": 1}, {"a": 2, "b": "x"}], [], [{"b": "y"}]])
    assert s == {
        "type": "array",
        "items": {
            "type": "object",
            "properties": {"a": {"type": "number", "x-empty": "missing"}, "b": {"type": "string", "x-empty": "missing"}},
        },
        "x-empty": "[]",
        "x-shape": "strict",
    }
    assert flags(s) == []
    # The root node is `$`. Its elements are `[]` and their fields `[].a`: the spec's `"[]"` key
    # and the stat slots `[]` and `[].a` read the same nodes, as `t` and `t[]` do for a field `t`.
    mixed = derive([[{"a": 1}, {"a": "x"}, 2], [], None])
    assert flags(mixed) == [("$", "mixed empty"), ("[]", "mixed types"), ("[].a", "mixed types")]


def test_nested_arrays_and_quoted_keys() -> None:
    s = derive([{"grid": [[1, 2], [3]], "first-name": "a"}, {"grid": [], "first-name": None}])
    assert s["properties"]["grid"] == {"type": "array", "items": {"type": "array", "items": {"type": "number"}}, "x-empty": "[]"}
    assert s["properties"]["first-name"] == {"type": "string", "x-empty": "null"}
    assert flags(s) == []


def test_derive_is_deterministic() -> None:
    samples = [{"a": [{"x": 1}, {"y": True}], "b": {"q": None}}, {"b": {"q": "s"}, "a": None}]
    assert json.dumps(derive(samples)) == json.dumps(derive(json.loads(json.dumps(samples))))


def _write(d: Path, name: str, text: str) -> None:
    d.mkdir(exist_ok=True)
    (d / name).write_text(text)


def test_load_pairs_skips(tmp_path: Path) -> None:
    dflt, prov, inp = tmp_path / "default", tmp_path / "provider", tmp_path / "inputs"
    _write(dflt, "ok.json", '{"a": 1}')
    _write(prov, "ok.json", '[{"b": 1}]')
    _write(inp, "ok.json", '{"q": "x"}')
    _write(dflt, "solo.json", "{}")
    _write(prov, "other.json", "{}")
    _write(dflt, "broken.json", "{}")
    _write(prov, "broken.json", "{nope")
    _write(dflt, "scalar.json", "42")
    _write(prov, "scalar.json", "{}")
    for name, text in [("badin.json", "{nope"), ("listin.json", "[1]")]:
        _write(dflt, name, "{}")
        _write(prov, name, "{}")
        _write(inp, name, text)
    _write(dflt, "noinput.json", "{}")
    _write(prov, "noinput.json", "{}")
    pairs, skipped = load_pairs(dflt, prov, inp)
    assert pairs == [
        Pair("noinput.json", {}, {}, None),
        Pair("ok.json", {"a": 1}, [{"b": 1}], {"q": "x"}),
    ]
    assert skipped == [
        ("badin.json", "inputs not JSON"),
        ("broken.json", "not JSON"),
        ("listin.json", "inputs not an object"),
        ("other.json", "only in provider"),
        ("scalar.json", "not an object or array"),
        ("solo.json", "only in default"),
    ]
    pairs, _ = load_pairs(dflt, prov, None)
    assert [p.name for p in pairs] == ["badin.json", "listin.json", "noinput.json", "ok.json"]
    assert all(p.inputs is None for p in pairs)


def test_compare_same_rule() -> None:
    assert same(1, 1.0)
    assert not same({"a": 1, "b": 2}, {"b": 2, "a": 1})
    assert same({"a": [1, "\ud800"]}, {"a": [1.0, "�"]})
    assert not same(True, 1) and not same(None, 0) and not same([1], [1, 2])


def test_schema_command(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    d = tmp_path / "s"
    _write(d, "1.json", '{"a": 1, "n": null, "t": []}')
    _write(d, "2.json", '{"a": "x", "n": null}')
    out = tmp_path / "schema.json"
    assert main(["schema", str(d), "--out", str(out)]) == 0
    _, err = capsys.readouterr()
    assert json.loads(out.read_text()) == derive([{"a": 1, "n": None, "t": []}, {"a": "x", "n": None}])
    assert err.splitlines() == ["a  mixed types", "n  always null", "t  mixed empty"]
    assert main(["schema", str(d)]) == 0
    assert json.loads(capsys.readouterr().out) == json.loads(out.read_text())
    _write(d, "3.json", "7")
    assert main(["schema", str(d)]) == 2
    assert "not an object or array" in capsys.readouterr().err
    assert main(["schema"]) == 2


def test_schema_file_keeps_index_like_key_order(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    d = tmp_path / "s"
    _write(d, "1.json", '{"b": 1, "10": 2, "2": 3}')
    out = tmp_path / "schema.json"
    assert main(["schema", str(d), "--out", str(out)]) == 0
    assert list(json.loads(out.read_text())["properties"]) == ["b", "10", "2"]


PEOPLE = [{"kind": "person", "name": f"p{i}", "title": "t"} for i in range(3)]
ORGS = [{"kind": "org", "name": f"o{i}", "size": i} for i in range(3)]


def test_schema_writes_strict() -> None:
    assert derive([{"a": 1}])["x-shape"] == "strict"
    assert derive([[{"a": 1}]])["x-shape"] == "strict"  # an array root takes it too


def test_schema_proposes_union() -> None:
    s = derive([{"id": i, "author": a} for i, a in enumerate([*PEOPLE, *ORGS])])
    author = s["properties"]["author"]
    assert "properties" not in author and author["type"] == "object"
    assert [b["title"] for b in author["oneOf"]] == ["person", "org"]
    assert list(author["oneOf"][0]["properties"]) == ["kind", "name", "title"]
    assert list(author["oneOf"][1]["properties"]) == ["kind", "name", "size"]
    assert ("author", "oneOf proposed") in flags(s)
    # no discriminating scalar: numbered branches
    plain = derive([{"a": {"x": 1}}, {"a": {"x": 2}}, {"a": {"y": 1}}, {"a": {"y": 2}}])
    assert [b["title"] for b in plain["properties"]["a"]["oneOf"]] == ["branch1", "branch2"]


def test_schema_no_union_for_noise() -> None:
    s = derive([{"author": a} for a in [*PEOPLE, {**PEOPLE[0], "extra": 1}]])
    assert "oneOf" not in s["properties"]["author"] and "extra" in s["properties"]["author"]["properties"]
    assert "oneOf" not in derive([{"author": a} for a in PEOPLE])["properties"]["author"]
