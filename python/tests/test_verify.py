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
    assert shape({"tags": None}, {"tags": []}, {}) == [("tags", "null against [] or {}")]
    assert shape({"m": {}}, {"m": None}, {}) == [("m", "null against [] or {}")]
    assert shape({"tags": {}}, {"tags": []}, {}) == [("tags", "type object against array")]
    assert shape({"jobs": [{"n": 1}]}, {"jobs": [{"n": "1"}]}, {}) == [("jobs[0].n", "type number against string")]
    assert shape({"name": None, "hq": None}, {"name": "a", "hq": {"city": "x"}}, {}) == []  # a value miss, not a shape one


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
