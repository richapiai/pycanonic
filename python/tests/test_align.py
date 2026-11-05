"""`--align-items`: pairs list items by id for fit and verify. Invented data only."""

from __future__ import annotations

import json
import random
from pathlib import Path
from typing import Any

import pytest

from pycanonic.cli.__main__ import main
from pycanonic.cli.fit.align import align, item_schema
from pycanonic.cli.fit.derive import derive_schema
from pycanonic.cli.fit.pairs import Pair


def urn(n: int) -> dict[str, Any]:
    return {"id": f"urn:example:item:{n}"}


def two_pairs() -> list[Pair]:
    d = {"items": [urn(7), urn(8), urn(9)]}
    p = {"results": [{"id": "9"}, {"id": "7"}, {"id": "10"}]}
    return [Pair("p0", d, p, {"q": "a"}), Pair("p1", d, p, None)]


def test_align_matches_by_digits() -> None:
    pairs, counts = align(two_pairs(), "items", "results", "id", "id", None)
    assert [(c.pair, c.matched, c.default_only, c.provider_only) for c in counts] == [("p0", 2, 1, 1), ("p1", 2, 1, 1)]
    assert [p.name for p in pairs] == ["p0#0", "p0#1", "p1#0", "p1#1"]
    assert [p.default["id"] for p in pairs[:2]] == ["urn:example:item:7", "urn:example:item:9"]  # default order
    assert [p.provider["id"] for p in pairs[:2]] == ["7", "9"]
    assert pairs[0].inputs == {"q": "a"} and pairs[2].inputs is None


def test_id_pattern_group_one() -> None:
    d = {"items": [{"id": "urn:example:item:ab1"}, {"id": "urn:example:item:cd"}]}
    p = {"results": [{"id": "item:cd"}, {"id": "item:ab1"}, {"id": "other"}]}
    pairs, counts = align([Pair("p", d, p, None)], "items", "results", "id", "id", r"item:(\w+)")
    assert (counts[0].matched, counts[0].default_only, counts[0].provider_only) == (2, 0, 1)  # "other" has no id
    assert pairs[0].provider == {"id": "item:ab1"}
    assert align([Pair("p", d, p, None)], "items", "results", "id", "id", r"item:\w+")[1][0].matched == 2  # no group: the whole match


def test_no_digits_no_match_and_whole_match() -> None:
    d = {"items": [{"id": "abc"}]}
    p = {"results": [{"id": "abc"}]}
    assert align([Pair("p", d, p, None)], "items", "results", "id", "id", None)[1][0].matched == 0
    assert align([Pair("p", d, p, None)], "items", "results", "id", "id", "[a-z]+")[1][0].matched == 1


def test_duplicate_id_pairs_first_only() -> None:
    d = {"items": [{"id": "5", "v": 1}, {"id": "5", "v": 2}]}
    p = {"results": [{"id": "5", "w": 1}]}
    pairs, counts = align([Pair("p", d, p, None)], "items", "results", "id", "id", None)
    assert [x.default["v"] for x in pairs] == [1]
    assert (counts[0].matched, counts[0].default_only, counts[0].provider_only) == (1, 1, 0)


def test_missing_list_gives_zero() -> None:
    pairs, counts = align([Pair("p", {"x": 1}, {"results": "no"}, None)], "items", "results", "id", "id", None)
    assert pairs == [] and (counts[0].matched, counts[0].default_only, counts[0].provider_only) == (0, 0, 0)


def test_bad_regex() -> None:
    with pytest.raises(ValueError, match="--id-pattern"):
        align(two_pairs(), "items", "results", "id", "id", "(")


def test_item_schema() -> None:
    schema = derive_schema([{"data": {"items": [{"id": "1"}]}, "n": 1, "tags": ["a"]}])
    assert item_schema(schema, "data.items") == {"type": "object", "properties": {"id": {"type": "string"}}}
    for bad in ("n", "tags", "data", "nope"):
        with pytest.raises(ValueError, match="array of objects"):
            item_schema(schema, bad)


def corpus(root: Path) -> None:
    rng = random.Random(3)
    (root / "d").mkdir()
    (root / "p").mkdir()
    for i in range(8):
        ids = rng.sample(range(100, 140), 9)
        d = [{"id": f"urn:example:item:{n}", "title": f"Title {n}", "score": n * 3} for n in ids[:8]]
        p = [{"id": str(n), "name": f"Title {n}", "points": n * 3} for n in ids[1:]]
        rng.shuffle(p)
        (root / "d" / f"r{i}.json").write_text(json.dumps({"items": d}))
        (root / "p" / f"r{i}.json").write_text(json.dumps({"results": p}))


def fit_args(root: Path, *extra: str) -> list[str]:
    return ["fit", "--default", str(root / "d"), "--provider", str(root / "p"), "--provider-name", "globex", "--endpoint", "list", "--out", str(root / "spec.json"), "--report", str(root / "rep"), *extra]


def test_fit_aligned_finds_auto_fields(tmp_path: Path) -> None:
    corpus(tmp_path)
    main(fit_args(tmp_path))
    whole = json.loads((tmp_path / "rep.json").read_text())["summary"]
    assert whole["auto"] == 0
    main(fit_args(tmp_path, "--align-items", "items", "results", "--by", "id", "id"))
    rep = json.loads((tmp_path / "rep.json").read_text())
    assert rep["summary"]["auto"] >= 3 and rep["pairs"] == 56
    assert rep["aligned"][0] == {"pair": "r0.json", "matched": 7, "default_only": 1, "provider_only": 1}
    assert "aligned  r0.json  7 matched" in (tmp_path / "rep.md").read_text()
    spec = json.loads((tmp_path / "spec.json").read_text())
    assert spec["fields"]["title"] == "name" and spec["fields"]["score"] == "points"


def test_verify_aligned(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    corpus(tmp_path)
    main(fit_args(tmp_path, "--align-items", "items", "results", "--by", "id", "id"))
    schema = tmp_path / "schema.json"
    schema.write_text(json.dumps(derive_schema([{"items": [{"id": "urn:example:item:1", "title": "t", "score": 1}]}])))
    args = ["verify", str(tmp_path / "spec.json"), "--schema", str(schema), "--default", str(tmp_path / "d"), "--provider", str(tmp_path / "p"), "--align-items", "items", "results", "--by", "id", "id", "--ignore", "id"]
    capsys.readouterr()
    assert main([*args, "--json"]) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["pairs"] == 56 and out["aligned"][0]["matched"] == 7
    assert main(args) == 0 and "aligned  r0.json" in capsys.readouterr().out


@pytest.mark.parametrize("extra", [["--align-items", "items", "results"], ["--by", "id", "id"], ["--align-items", "items", "results", "--by", "id", "id", "--id-pattern", "("], ["--align-items", "n", "n", "--by", "id", "id"]])
def test_bad_flags_exit_2(tmp_path: Path, extra: list[str]) -> None:
    corpus(tmp_path)
    assert main(fit_args(tmp_path, *extra)) == 2
