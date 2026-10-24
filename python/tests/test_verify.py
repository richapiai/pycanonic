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
        "fields": {"name": {"match": 3, "total": 3, "share": 1.0}, "email": {"match": 3, "total": 3, "share": 1.0}},
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
    assert r.fields == {"jobs[]": (1, 1)} and r.shape_failures == [("p0.json", "jobs[0]", "key order")]


def test_shape_checks_common_keys_when_keys_differ() -> None:
    got, want = {"x": {"b": 1, "a": 2, "e": 0}}, {"x": {"a": "2", "b": 1, "c": 3}}
    assert shape(got, want, {}) == [("x.c", "missing key"), ("x.e", "extra key"), ("x", "key order"), ("x.a", "type number against string")]
