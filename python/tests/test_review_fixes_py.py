"""Regression tests for the Python package, the CLI and the scripts: one test per fixed defect."""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path
from typing import Any

from pycanonic.cli.__main__ import main
from pycanonic.cli.fit import verify
from pycanonic.cli.fit.compare import Tolerance, match, same
from pycanonic.cli.fit.derive import derive_schema
from pycanonic.cli.fit.fit import FitOptions, fit_pairs
from pycanonic.cli.fit.pairs import Pair
from pycanonic.cli.infer.ask import ask_with
from pycanonic.cli.infer.infer import infer

NAMES = ["Ada", "Bob", "Cy", "Dee", "Eve", "Fay", "Gus", "Hal", "Ivy", "Jo"]


def _spec(fields: dict[str, Any]) -> dict[str, Any]:
    return {"language": 2, "provider": "p", "endpoint": "e", "version": 1, "status": "draft", "fields": fields}


def test_fit_never_autos_a_field_that_verify_fails() -> None:
    """A rule that writes a value where the default has none counts against the field, so fit
    does not call it auto when verify then fails the same spec on the same pairs."""
    pairs = []
    for i, n in enumerate(NAMES):
        d = {"name": n if i < 8 else None}
        p = {"full_name": n if i < 8 else f"JUNK{i}", "x": i}
        pairs.append(Pair(f"p{i}.json", d, p, None))
    schema = {"type": "object", "properties": {"name": {"type": "string"}}}
    r = fit_pairs(pairs, schema, FitOptions("p", "e"))
    row = next(x for x in r.fields if x.field == "name")
    ok = verify.run(r.spec, schema, pairs, 0.95, None).ok
    assert ok or row.tier != "auto", (row.tier, row.support, row.total)
    assert row.total == 10


def test_fit_never_autos_an_item_leaf_that_verify_fails() -> None:
    pairs = []
    for i, n in enumerate(NAMES):
        d = {"jobs": [{"title": n if i < 8 else None, "org": f"o{i}"}]}
        p = {"roles": [{"t": n if i < 8 else f"JUNK{i}", "o": f"o{i}"}]}
        pairs.append(Pair(f"p{i}.json", d, p, None))
    item = {"type": "object", "properties": {"title": {"type": "string"}, "org": {"type": "string"}}}
    schema = {"type": "object", "properties": {"jobs": {"type": "array", "items": item}}}
    r = fit_pairs(pairs, schema, FitOptions("p", "e"))
    v = verify.run(r.spec, schema, pairs, 0.95, None)
    title = next(x for x in r.fields if x.field == "jobs[].title")
    assert v.fields["jobs[].title"] == (8, 10)
    assert (title.tier, title.total) == ("review", 10)


def test_empty_row_the_runtime_misses_is_manual_with_a_note() -> None:
    """An `empty` row whose runtime output disagrees with the default's empty form is manual, not
    review with the one-value note."""
    pairs = [Pair(f"p{i}.json", {"name": n, "tags": []}, {"full_name": n}, None) for i, n in enumerate(NAMES)]
    schema = {"type": "object", "properties": {"name": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}}}}
    row = next(x for x in fit_pairs(pairs, schema, FitOptions("p", "e")).fields if x.field == "tags[]")
    assert row.tier == "manual"
    assert "Runtime check" in row.note and "one value" not in row.note


def test_empty_row_the_runtime_matches_is_review_without_a_note() -> None:
    pairs = [Pair(f"p{i}.json", {"name": n, "tags": []}, {"full_name": n}, None) for i, n in enumerate(NAMES)]
    schema = {"type": "object", "properties": {"name": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}, "x-empty": "[]"}}}
    row = next(x for x in fit_pairs(pairs, schema, FitOptions("p", "e")).fields if x.field == "tags[]")
    assert (row.tier, row.support, row.total, row.note) == ("review", 10, 10, "")


def test_derive_schema_skips_object_values_shared_by_every_key_set() -> None:
    s = [{"meta": {"a": 1}, "kind": "a", "x": 1}, {"meta": {"a": 2}, "kind": "a", "x": 2}, {"meta": {"a": 1}, "kind": "b", "y": 1}, {"meta": {"a": 3}, "kind": "b", "y": 2}]
    out = derive_schema(s)
    assert [b["title"] for b in out["oneOf"]] == ["a", "b"]
    assert derive_schema([{"o": x} for x in s])["properties"]["o"]["oneOf"]


def test_derive_schema_skips_list_values_too() -> None:
    s = [{"l": [1], "kind": "a", "x": 1}, {"l": [2], "kind": "a", "x": 2}, {"l": [1], "kind": "b", "y": 1}, {"l": [3], "kind": "b", "y": 2}]
    assert [b["title"] for b in derive_schema(s)["oneOf"]] == ["a", "b"]


def test_verify_scores_an_item_leaf_whose_name_has_a_dot() -> None:
    item = {"type": "object", "properties": {"a.b": {"type": "string"}, "t": {"type": "string"}}}
    schema = {"type": "object", "properties": {"jobs": {"type": "array", "items": item}}}
    pairs = [Pair(f"p{i}", {"jobs": [{"a.b": f"right{i}", "t": "x"}]}, {"r": [{"w": "WRONG", "t": "x"}]}, None) for i in range(5)]
    spec = _spec({"jobs[]": {"from": "r", "item": {'["a.b"]': "w", "t": "t"}}})
    v = verify.run(spec, schema, pairs, 0.95, None)
    assert not v.ok
    assert v.fields['jobs[].["a.b"]'] == (0, 5)


def test_verify_lists_an_unmapped_dotted_item_leaf() -> None:
    item = {"type": "object", "properties": {"a.b": {"type": "string"}, "t": {"type": "string"}}}
    schema = {"type": "object", "properties": {"jobs": {"type": "array", "items": item}}}
    pairs = [Pair(f"p{i}", {"jobs": [{"a.b": f"v{i}", "t": "x"}]}, {"r": [{"t": "x"}]}, None) for i in range(5)]
    v = verify.run(_spec({"jobs[]": {"from": "r", "item": {"t": "t"}}}), schema, pairs, 0.95, None)
    assert list(v.unmapped) == ['jobs[].["a.b"]']


def test_compare_huge_integers_do_not_crash() -> None:
    big = json.loads("1" + "0" * 400)
    assert same(big, big)
    assert not same(big, big + 1)
    assert not same(big, 1.0)
    assert match(big, big, Tolerance(num=0.01)) == "exact"
    assert match(big, big + 1, Tolerance(num=0.01)) == "miss"  # past the double range: exact only
    assert match(big, 1.0, Tolerance(num=0.01)) == "miss"


def test_fit_warns_when_the_old_out_file_is_not_json(tmp_path: Path, capsys: Any) -> None:
    d, p = tmp_path / "d", tmp_path / "p"
    d.mkdir()
    p.mkdir()
    for i, n in enumerate(NAMES):
        (d / f"{i}.json").write_text(json.dumps({"name": n}))
        (p / f"{i}.json").write_text(json.dumps({"full_name": n}))
    out = tmp_path / "spec.json"
    out.write_text("not json")
    code = main(["fit", "--default", str(d), "--provider", str(p), "--provider-name", "p", "--endpoint", "e", "--out", str(out), "--report", str(tmp_path / "r")])
    assert code in (0, 1)
    assert json.loads(out.read_text())["fields"]
    assert "is not JSON" in capsys.readouterr().err


def test_ask_drops_an_answer_that_a_join_already_reads() -> None:
    schema = {"type": "object", "properties": {"full_name": {"type": "string"}, "bio": {"type": "string"}}}
    samples = [{"first_name": "Ann", "last_name": "Lee"}, {"first_name": "Bob", "last_name": "Ray"}]
    spec, _ = infer(samples, schema, provider="p", endpoint="e", ask="echo first_name")
    assert "bio" not in spec["fields"], spec["fields"]


def test_ask_command_times_out(capsys: Any) -> None:
    ask = ask_with(f'"{sys.executable}" -c "import time; time.sleep(30)"', timeout=0.5)
    start = time.monotonic()
    assert ask({"field": "bio"}) is None
    assert time.monotonic() - start < 10
    assert "--ask failed for bio: timed out" in capsys.readouterr().err


def test_ask_timeout_flag_is_validated(tmp_path: Path) -> None:
    schema = tmp_path / "s.json"
    schema.write_text(json.dumps({"type": "object", "properties": {"a": {"type": "string"}}}))
    sample = tmp_path / "x.json"
    sample.write_text(json.dumps({"a": "b"}))
    assert main(["infer", "--schema", str(schema), "--provider", "p", "--endpoint", "e", "--ask", "true", "--ask-timeout", "-1", str(sample)]) == 2
    assert main(["infer", "--schema", str(schema), "--provider", "p", "--endpoint", "e", "--ask", "true", "--ask-timeout", "5", str(sample)]) == 0

