"""`canonic verify`: shape, values and rejects of a spec mapped over sample pairs."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from pycanonic.cli.__main__ import main
from pycanonic.cli.fit.derive import derive_schema
from pycanonic.cli.fit.pairs import Pair
from pycanonic.cli.fit.verify import run, shape

FIXTURE = Path(__file__).parent / "fixtures" / "fit" / "acme-to-globex"
S = {"type": "string"}


def spec(fields: dict[str, Any], reject: list[Any] | None = None) -> dict[str, Any]:
    out: dict[str, Any] = {"language": 2, "provider": "globex", "endpoint": "e", "version": 1, "status": "draft", "fields": fields}
    return {**out, "reject": reject} if reject else out


def pairs(rows: list[tuple[Any, Any]]) -> list[Pair]:
    return [Pair(f"p{i}.json", d, p, None) for i, (d, p) in enumerate(rows)]


def test_shape_missing_key() -> None:
    rows = pairs([({"name": "a", "nick": "x"}, {"n": "a"}), ({"name": "b"}, {"n": "b"})])
    r = run(spec({"name": "n"}), derive_schema([p.default for p in rows]), rows, 0.95, None)
    assert r.shape_failures == [("p0.json", "nick", "missing key")] and not r.ok
    assert shape({"a": 1, "b": 2}, {"a": 1}, {}) == [("b", "extra key")]


def test_shape_key_order() -> None:
    schema = {"type": "object", "properties": {"b": S, "a": S}}
    rows = pairs([({"a": "1", "b": "2"}, {"a": "1", "b": "2"})])
    r = run(spec({"a": "a", "b": "b"}), schema, rows, 0.95, None)
    assert r.shape_failures == [("p0.json", "$", "key order")] and not r.ok
    assert shape({"x": {"b": 1, "a": 2}}, {"x": {"a": 2, "b": 1}}, {}) == [("x", "key order")]


def test_shape_null_vs_list() -> None:
    assert shape({"tags": None}, {"tags": []}, {}) == [("tags", "null against array")]
    assert shape({"m": {}}, {"m": None}, {}) == [("m", "null against object")]
    assert shape({"tags": {}}, {"tags": []}, {}) == [("tags", "type object against array")]
    assert shape({"jobs": [{"n": 1}]}, {"jobs": [{"n": "1"}]}, {}) == [("jobs[0].n", "type number against string")]
    assert shape({"name": None}, {"name": "a"}, {}) == []  # a value miss, not a shape one (R35)


def test_shape_null_against_container_both_ways() -> None:
    """an object or array against null is a shape failure in both directions, at any
    depth, empty or not. An object of nulls where the default sends null breaks the contract."""
    assert shape({"loc": {"city": None, "country": None}}, {"loc": None}, {}) == [("loc", "null against object")]
    assert shape({"loc": None}, {"loc": {"city": "x"}}, {}) == [("loc", "null against object")]
    assert shape({"tags": ["a"]}, {"tags": None}, {}) == [("tags", "null against array")]
    assert shape({"tags": None}, {"tags": ["a"]}, {}) == [("tags", "null against array")]
    nested = shape({"jobs": [{"org": {"name": None}}, {"org": None}]}, {"jobs": [{"org": None}, {"org": {"name": "o"}}]}, {})
    assert nested == [("jobs[0].org", "null against object"), ("jobs[1].org", "null against object")]
    assert shape(None, [1], {}) == [("$", "null against array")]
    rows = pairs([({"name": "a", "loc": None}, {"n": "a"})])
    schema = {"type": "object", "properties": {"name": S, "loc": {"type": "object", "properties": {"city": S}}}}
    r = run(spec({"name": "n", "loc.city": "c"}), schema, rows, 0.95, None)
    assert r.shape_failures == [("p0.json", "loc", "null against object")] and not r.ok


def test_open_map_keys_may_differ() -> None:
    node = {"type": "object", "properties": {"m": {"type": "object", "x-open": True}}}
    assert shape({"m": {"a": 1}}, {"m": {"b": 2}}, node) == []
    assert shape({"m": {"a": 1}}, {"m": {"b": 2}}, {}) == [("m.b", "missing key"), ("m.a", "extra key")]
    root = {"type": "array", "items": node}
    assert shape([{"m": {"a": 1}}], [{"m": {"b": 2}}], root) == []


def test_unmapped_field_skipped_in_values() -> None:
    rows = pairs([({"name": f"n{i}", "status": "on"}, {"n": f"n{i}"}) for i in range(4)])
    r = run(spec({"name": "n"}), derive_schema([p.default for p in rows]), rows, 0.95, None)
    assert r.fields == {"name": (4, 4)} and r.shape_failures == [] and r.ok


def test_min_match_gate() -> None:
    rows = pairs([({"name": f"n{i}"}, {"n": f"n{i}" if i else "wrong"}) for i in range(10)])
    schema = derive_schema([p.default for p in rows])
    assert run(spec({"name": "n"}), schema, rows, 0.95, None).fields == {"name": (9, 10)}
    assert not run(spec({"name": "n"}), schema, rows, 0.95, None).ok
    assert run(spec({"name": "n"}), schema, rows, 0.9, None).ok


def reject_rows(n: int, lacking: int) -> list[Pair]:
    """`n` pairs; the first `lacking` have no `full_name`, so the provider's name is incomplete."""
    return pairs([({"name": f"n{i}", "email": f"e{i}"}, {**({"full_name": f"n{i}"} if i >= lacking else {}), "mail": f"e{i}"}) for i in range(n)])


def test_reject_over_incomplete_answer_passes() -> None:
    rows = reject_rows(20, 1)
    r = run(spec({"name": "full_name", "email": "mail"}, [{"missing": "full_name"}]), derive_schema([p.default for p in rows]), rows, 0.95, None)
    assert (r.rejected, r.rejected_wrongly, r.ok) == (["p0.json"], [], True)
    assert r.fields == {"name": (19, 19), "email": (19, 19)}


def test_reject_counts_as_failure() -> None:
    """A reject that discards an answer the spec maps to exactly the default's output."""
    rows = reject_rows(20, 1)
    r = run(spec({"name": "mail", "email": "mail"}, [{"missing": "full_name"}]), derive_schema([{"name": f"e{i}", "email": f"e{i}"} for i in range(2)]), [Pair(p.name, {"name": p.provider["mail"], "email": p.provider["mail"]}, p.provider, None) for p in rows], 0.95, None)
    assert (r.rejected, r.rejected_wrongly, r.ok) == (["p0.json"], ["p0.json"], False)


def test_too_many_rejects_fail() -> None:
    rows = reject_rows(10, 3)
    s, schema = spec({"name": "full_name", "email": "mail"}, [{"missing": "full_name"}]), derive_schema([p.default for p in rows])
    r = run(s, schema, rows, 0.95, None)
    assert (r.rejected, r.rejected_wrongly, r.ok) == (["p0.json", "p1.json", "p2.json"], [], False)
    assert run(s, schema, rows, 0.95, None, max_reject=0.3).ok


def write(path: Path, value: Any) -> str:
    path.write_text(json.dumps(value))
    return str(path)


def test_json_output_layout(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    (tmp_path / "d").mkdir()
    (tmp_path / "p").mkdir()
    for p in reject_rows(4, 1):
        write(tmp_path / "d" / p.name, p.default)
        write(tmp_path / "p" / p.name, p.provider)
    s = write(tmp_path / "spec.json", spec({"name": "full_name", "email": "mail"}, [{"missing": "full_name"}]))
    sc = write(tmp_path / "schema.json", derive_schema([p.default for p in reject_rows(4, 1)]))
    args = ["verify", s, "--schema", sc, "--default", str(tmp_path / "d"), "--provider", str(tmp_path / "p")]
    assert main([*args, "--json"]) == 1  # 1 of 4 rejected is over 0.10
    out = json.loads(capsys.readouterr().out)
    assert out == {
        "ok": False,
        "pairs": 4,
        "skipped": [],
        "shape_failures": [],
        "fields": {"name": {"match": 3, "total": 3, "share": 1.0, "misses": []}, "email": {"match": 3, "total": 3, "share": 1.0, "misses": []}},
        "unmapped": {},
        "rejected": ["p0.json"],
        "rejected_wrongly": [],
        "reject_share": 0.25,
    }
    assert main([*args, "--max-reject", "0.25"]) == 0
    table = capsys.readouterr().out
    assert "rejected  p0.json" in table and "verify: ok" in table


def test_verify_passes_on_fit_output(tmp_path: Path) -> None:
    d, p = str(FIXTURE / "default"), str(FIXTURE / "provider")
    fit = ["fit", "--default", d, "--provider", p, "--provider-name", "globex", "--endpoint", "person.enrich"]
    assert main([*fit, "--out", str(tmp_path / "spec.json"), "--report", str(tmp_path / "report")]) == 2
    defaults = [json.loads(f.read_text()) for f in sorted((FIXTURE / "default").iterdir())]
    sc = write(tmp_path / "schema.json", derive_schema(defaults))
    args = ["verify", str(tmp_path / "spec.json"), "--schema", sc, "--default", d, "--provider", p]
    assert main([*args, "--min-match", "0.9"]) == 0


# --- fix round: and review ---


def test_both_null_pairs_do_not_count() -> None:
    """Null on both sides in most pairs, wrong in the rest: the field fails --min-match."""
    rows = pairs([({"name": None}, {}) for _ in range(8)] + [({"name": f"n{i}"}, {"n": "wrong"}) for i in range(2)])
    r = run(spec({"name": "n"}), {"type": "object", "properties": {"name": {**S, "x-empty": "null"}}}, rows, 0.5, None)
    assert r.fields == {"name": (0, 2)} and not r.ok


def test_null_output_against_value_is_a_value_miss() -> None:
    rows = pairs([({"name": "a"}, {})])
    r = run(spec({"name": "n"}), derive_schema([p.default for p in rows]), rows, 0.95, None)
    assert (r.fields, r.shape_failures) == ({"name": (0, 1)}, [])


def test_skipped_pairs_fail(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    (tmp_path / "d").mkdir()
    (tmp_path / "p").mkdir()
    rows = reject_rows(2, 0)
    for p in rows:
        write(tmp_path / "d" / p.name, p.default)
        write(tmp_path / "p" / p.name, p.provider)
    write(tmp_path / "d" / "extra.json", rows[0].default)
    s = write(tmp_path / "spec.json", spec({"name": "full_name", "email": "mail"}))
    sc = write(tmp_path / "schema.json", derive_schema([p.default for p in rows]))
    args = ["verify", s, "--schema", sc, "--default", str(tmp_path / "d"), "--provider", str(tmp_path / "p")]
    assert main(args) == 1
    assert "skipped  extra.json  only in default" in capsys.readouterr().out
    assert not run(json.loads(Path(s).read_text()), json.loads(Path(sc).read_text()), rows, 0.95, None, skipped=[("extra.json", "only in default")]).ok


def test_transforms_module(tmp_path: Path) -> None:
    (tmp_path / "d").mkdir()
    (tmp_path / "p").mkdir()
    for i in range(3):
        write(tmp_path / "d" / f"p{i}.json", {"name": f"N{i}!"})
        write(tmp_path / "p" / f"p{i}.json", {"n": f"N{i}"})
    mod = tmp_path / "shout.py"
    mod.write_text("def shout(v):\n    return None if v is None else v + '!'\n")
    s = write(tmp_path / "spec.json", spec({"name": {"from": "n", "transform": "shout"}}))
    sc = write(tmp_path / "schema.json", {"type": "object", "properties": {"name": S}})
    args = ["verify", s, "--schema", sc, "--default", str(tmp_path / "d"), "--provider", str(tmp_path / "p")]
    assert main([*args, "--transforms", str(mod)]) == 0
    assert main(args) == 2  # the spec names a transform it was not given


def test_key_order_only_in_shape() -> None:
    schema = {"type": "object", "properties": {"jobs": {"type": "array", "items": {"type": "object", "properties": {"b": S, "a": S}}}}}
    rows = pairs([({"jobs": [{"a": "1", "b": "2"}]}, {"x": [{"a": "1", "b": "2"}]})])
    r = run(spec({"jobs[]": {"from": "x", "item": {"a": "a", "b": "b"}}}), schema, rows, 0.95, None)
    assert r.fields["jobs[]"] == (1, 1) and r.shape_failures == [("p0.json", "jobs[0]", "key order")]


def test_shape_checks_common_keys_when_keys_differ() -> None:
    got, want = {"x": {"b": 1, "a": 2, "e": 0}}, {"x": {"a": "2", "b": 1, "c": 3}}
    assert shape(got, want, {}) == [("x.c", "missing key"), ("x.e", "extra key"), ("x", "key order"), ("x.a", "type number against string")]


# --- final fix wave: and the miss list ---


def test_unmapped_leaf_listed_and_fail_flag() -> None:
    """R43: a leaf the default fills but the spec does not map is listed; --fail-unmapped fails it."""
    rows = pairs([({"name": f"n{i}", "status": "on" if i < 3 else None, "hq": {"city": "c", "zip": "z"}}, {"n": f"n{i}", "c": "c"}) for i in range(4)])
    schema = derive_schema([p.default for p in rows])
    s = spec({"name": "n", "hq.city": "c"})
    r = run(s, schema, rows, 0.95, None)
    assert r.unmapped == {"status": 3, "hq.zip": 4} and r.ok
    assert not run(s, schema, rows, 0.95, None, fail_unmapped=True).ok
    open_schema = {"type": "object", "properties": {"m": {"type": "object", "x-open": True, "properties": {"a": S}}}}
    whole = run(spec({"m": "x"}), open_schema, pairs([({"m": {"a": "1"}}, {"x": {"a": "1"}})]), 0.95, None)
    assert whole.unmapped == {}  # a path on an open map copies its leaves


def test_unmapped_and_misses_in_cli(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    (tmp_path / "d").mkdir()
    (tmp_path / "p").mkdir()
    for i in range(12):
        write(tmp_path / "d" / f"p{i:02}.json", {"name": f"n{i}", "status": "on"})
        write(tmp_path / "p" / f"p{i:02}.json", {"n": f"n{i}" if i > 6 else "x"})
    s = write(tmp_path / "spec.json", spec({"name": "n"}))
    sc = write(tmp_path / "schema.json", {"type": "object", "properties": {"name": S, "status": S}})
    args = ["verify", s, "--schema", sc, "--default", str(tmp_path / "d"), "--provider", str(tmp_path / "p"), "--min-match", "0"]
    assert main(args) == 0
    table = capsys.readouterr().out
    assert "not mapped: status (12 pairs with a value)" in table
    assert "name  5/12  0.42  misses p00.json, p01.json, p02.json, p03.json, p04.json\n" in table
    assert main([*args, "--json"]) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["unmapped"] == {"status": {"pairs": 12}}
    assert out["fields"]["name"]["misses"] == ["p00.json", "p01.json", "p02.json", "p03.json", "p04.json"]
    assert main([*args, "--fail-unmapped"]) == 1


# --- Task 12: item leaves, count rows, --ignore ---

ITEMS = {
    "type": "object",
    "properties": {"name": S, "items": {"type": "array", "items": {"type": "object", "properties": {"name": S, "url": S}}}},
}
ITEM_SPEC = {"name": "n", "items[]": {"from": "xs", "item": {"name": "n", "url": "u"}}}


def item_rows(lens: tuple[int, int] = (3, 3)) -> list[Pair]:
    mk = lambda c, side: [{"name": f"i{j}", "url": f"u{j}" if side == "d" else "other"} for j in range(c)]
    return pairs([({"name": "a", "items": mk(lens[0], "d")}, {"n": "a", "xs": [{"n": f"i{j}", "u": "other"} for j in range(lens[1])]}) for _ in range(2)])


def test_item_leaves_scored() -> None:
    r = run(spec(ITEM_SPEC), ITEMS, item_rows(), 0.95, None)
    assert r.fields["items[]"] == (0, 2)
    assert r.fields["items[].url"] == (0, 6) and r.fields["items[].name"] == (6, 6) and not r.ok
    assert r.misses["items[].url"] == ["p0.json#0", "p0.json#1", "p0.json#2", "p1.json#0", "p1.json#1"]


def test_item_count_row() -> None:
    r = run(spec(ITEM_SPEC), ITEMS, item_rows((2, 3)), 0.95, None)
    assert r.fields["items[] (count)"] == (0, 2) and r.fields["items[].name"] == (4, 4)


def test_ignore_waives_field() -> None:
    rows = item_rows((2, 3))
    assert not run(spec(ITEM_SPEC), ITEMS, rows, 0.95, None).ok
    r = run(spec(ITEM_SPEC), ITEMS, rows, 0.95, None, ignore=["items"])
    assert list(r.fields) == ["name"] and r.ok and r.shape_failures == []
    assert [f for f in run(spec(ITEM_SPEC), ITEMS, rows, 0.95, None).shape_failures if f[1].startswith("items")] == []


def test_json_item_rows(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    (tmp_path / "d").mkdir()
    (tmp_path / "p").mkdir()
    for p in item_rows():
        write(tmp_path / "d" / p.name, p.default)
        write(tmp_path / "p" / p.name, p.provider)
    s, sc = write(tmp_path / "s.json", spec(ITEM_SPEC)), write(tmp_path / "sc.json", ITEMS)
    args = ["verify", s, "--schema", sc, "--default", str(tmp_path / "d"), "--provider", str(tmp_path / "p")]
    assert main([*args, "--json"]) == 1
    f = json.loads(capsys.readouterr().out)["fields"]
    assert {"items[]", "items[] (count)", "items[].name", "items[].url"} <= set(f)
    assert f["items[].url"]["match"] == 0 and f["items[].url"]["total"] == 6
    assert main([*args, "--ignore", "items"]) == 0 and "items" not in capsys.readouterr().out


def test_nested_list_and_union_leaves() -> None:
    nested = {"type": "object", "properties": {"a": {"type": "array", "items": {"type": "object", "properties": {"b": {"type": "array", "items": {"type": "object", "properties": {"c": S}}}}}}}}
    row = ({"a": [{"b": [{"c": "1"}, {"c": "2"}]}]}, {"x": [{"y": [{"c": "1"}, {"c": "z"}]}]})
    r = run(spec({"a[]": {"from": "x", "item": {"b[]": {"from": "y", "item": {"c": "c"}}}}}), nested, pairs([row]), 0.0, None)
    assert r.fields["a[].b[].c"] == (1, 2) and r.fields["a[].b[] (count)"] == (1, 1)
    union = {"type": "object", "properties": {"u": {"type": "object", "oneOf": [{"title": "A", "type": "object", "properties": {"p": S}}, {"title": "B", "type": "object", "properties": {"q": S}}]}}}
    r = run(spec({"u": {"one_of": [{"branch": "B", "when": {"path": "k", "eq": "b"}, "fields": {"q": "v"}}]}}), union, pairs([({"u": {"q": "1"}}, {"k": "b", "v": "1"})]), 0.95, None)
    assert r.fields["u(B).q"] == (1, 1) and r.shape_failures == []


def test_item_leaf_outside_item_map_is_unmapped() -> None:
    """R43 inside items: a leaf the item map omits is listed, and gates only with --fail-unmapped."""
    rows = pairs([({"name": "a", "items": [{"name": "i", "url": "u"}]}, {"n": "a", "xs": [{"n": "i"}]}) for _ in range(2)])
    s = spec({"name": "n", "items[]": {"from": "xs", "item": {"name": "n"}}})
    r = run(s, ITEMS, rows, 0.0, None)  # 0.0: the whole-list row `items[]` still misses the omitted url
    assert r.unmapped == {"items[].url": 2} and "items[].url" not in r.fields and r.ok
    assert not run(s, ITEMS, rows, 0.0, None, fail_unmapped=True).ok
    ref = spec({"name": "n", "items[]": {"from": "xs", "item": {"$ref": "it"}}})
    assert run({**ref, "definitions": {"it": {"name": "n", "url": "u"}}}, ITEMS, item_rows(), 0.0, None).unmapped == {}


def test_ignore_item_leaf_waives_whole_list_row() -> None:
    rows = item_rows()
    assert not run(spec(ITEM_SPEC), ITEMS, rows, 0.95, None).ok
    r = run(spec(ITEM_SPEC), ITEMS, rows, 0.95, None, ignore=["items[].url"])
    assert r.ok and "items[].url" not in r.fields and "items[]" not in r.fields and r.fields["items[].name"] == (6, 6)
