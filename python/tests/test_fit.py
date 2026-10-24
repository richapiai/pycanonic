"""`canonic fit`: the leaf search (index, transform chains, rule choice, holdout)."""

from __future__ import annotations

from typing import Any

from pycanonic.cli.fit.chains import Chain, apply, candidates
from pycanonic.cli.fit.index import Occurrence, scalars
from pycanonic.cli.fit.leaves import LeafFit, fit_leaf
from pycanonic.cli.fit.pairs import Pair

# --- leaf section ---


def leaf(field: str, rows: list[tuple[Any, Any]], lookups: bool = True) -> LeafFit:
    """fit_leaf over pairs built from (target, provider body) rows."""
    pairs = [Pair(f"p{i}.json", {field: t}, body, None) for i, (t, body) in enumerate(rows)]
    return fit_leaf(field, pairs, [t for t, _ in rows], [scalars(p.provider) for p in pairs], lookups)


def test_index_paths() -> None:
    body = {"name": "Ada", "positions": [{"title": "a"}, {"title": "b", "n": None}], "odd key": True}
    assert scalars(body) == [
        Occurrence("name", "Ada"),
        Occurrence("positions[0].title", "a"),
        Occurrence("positions[1].title", "b"),
        Occurrence('["odd key"]', True),
    ]
    assert scalars([{"a": 1}, 2]) == [Occurrence("[0].a", 1), Occurrence("[1]", 2)]
    assert scalars("x") == [Occurrence("$", "x")]


def test_chain_apply_and_candidates() -> None:
    assert apply(Chain([("strip_prefix", "urn:x:"), ("cast", "integer")]), "urn:x:42") == 42
    assert apply(Chain([]), {"a": 1}) == {"a": 1}
    assert Chain([]) in candidates("Ada", "Ada")
    got = candidates(42, "urn:x:42")
    assert Chain([("strip_prefix", "urn:x:"), ("cast", "integer")]) in got
    assert [len(c.steps) for c in got] == sorted(len(c.steps) for c in got)
    assert Chain([("join", ", ")]) in candidates("Redmond, Washington", ["Redmond", "Washington"])


def test_exact_path() -> None:
    f = leaf("title", [(f"t{i}", {"job": {"title": f"t{i}"}, "x": i}) for i in range(6)])
    assert (f.rule, f.kind, f.support, f.total, f.holdout) == ({"from": "job.title"}, "exact", 6, 6, 1.0)


def test_cast_integer() -> None:
    f = leaf("size", [(42 + i, {"size": str(42 + i)}) for i in range(6)])
    assert f.rule == {"from": "size", "transform": [{"cast": "integer"}]}
    assert (f.kind, f.support) == ("transform", 6)


def test_strip_prefix_learned() -> None:
    f = leaf("id", [(f"{i}a{i}", {"urn": f"urn:x:{i}a{i}"}) for i in range(6)])
    assert f.rule == {"from": "urn", "transform": [{"strip_prefix": "urn:x:"}]}
    assert f.holdout == 1.0


def test_date_layout_learned() -> None:
    f = leaf("start", [(f"2021-0{i}-01T00:00:00.000Z", {"since": f"2021-0{i}"}) for i in range(1, 7)])
    assert f.rule == {"from": "since", "transform": [{"format_date": "datetime_ms"}]}


def test_join_separator_learned() -> None:
    rows = [(f"{c}, {r}", {"city": c, "region": r, "n": i}) for i, (c, r) in enumerate([("Redmond", "Washington"), ("Austin", "Texas"), ("Lyon", "Rhone"), ("Kyoto", "Kansai"), ("Perth", "WA")])]
    f = leaf("location", rows)
    assert f.rule == {"from": ["city", "region"], "transform": [{"join": ", "}]}
    assert (f.kind, f.support) == ("join", 5)


def test_coalesce() -> None:
    rows = [(f"v{i}", {"a": f"v{i}"} if i % 2 else {"b": f"v{i}"}) for i in range(6)]
    f = leaf("value", rows)
    assert f.rule in ({"coalesce": ["a", "b"]}, {"coalesce": ["b", "a"]})
    assert (f.kind, f.support) == ("coalesce", 6)


def test_lookup_learned() -> None:
    enum = [("LIKE", "liked"), ("LOVE", "loved"), ("LIKE", "liked"), ("HATE", "hated"), ("LOVE", "loved")]
    f = leaf("reaction", [(t, {"kind": s}) for s, t in enum])
    assert f.kind == "lookup"
    assert f.rule == {"from": "kind", "transform": [{"lookup": {"table": {"HATE": "hated", "LIKE": "liked", "LOVE": "loved"}, "else": "null"}}]}
    assert f.support == 5
    assert f.holdout < 1  # HATE is seen once
    assert leaf("reaction", [(t, {"kind": s}) for s, t in enum], lookups=False).kind == "none"


def test_lookup_conflict_rejected() -> None:
    enum = [("LIKE", "liked"), ("LIKE", "fond"), ("LOVE", "loved"), ("LOVE", "loved"), ("HATE", "hated")]
    f = leaf("reaction", [(t, {"kind": s}) for s, t in enum])
    assert (f.kind, f.rule, f.support) == ("none", None, 0)


def test_holdout_catches_overfit() -> None:
    rows = [("person" if i else "company", {"n": i}) for i in range(6)]
    f = leaf("type", rows)
    assert (f.rule, f.kind, f.support, f.total) == ({"const": "person"}, "constant", 5, 6)
    assert f.holdout == 5 / 6


def test_name_tiebreak() -> None:
    rows = [(f"e{i}@acme.com", {"contact": f"e{i}@acme.com", "work_email": f"e{i}@acme.com"}) for i in range(6)]
    assert leaf("email", rows).rule == {"from": "work_email"}
    rows = [(f"e{i}@acme.com", {"work_email": f"e{i}@acme.com", "contact": f"e{i}@acme.com"}) for i in range(6)]
    assert leaf("email", rows).rule == {"from": "work_email"}


def test_root_array_body() -> None:
    f = leaf("name", [(f"n{i}", [{"full": f"n{i}"}]) for i in range(5)])
    assert f.rule == {"from": "[0].full"}


def test_skips_null_targets() -> None:
    rows = [(None if i == 0 else f"t{i}", {"t": f"t{i}"}) for i in range(6)]
    f = leaf("t", rows)
    assert (f.rule, f.support, f.total) == ({"from": "t"}, 5, 5)


def test_deterministic() -> None:
    rows = [(f"{i}", {"a": f"{i}", "b": f"{i}", "c": i, "d": f"urn:{i}"}) for i in range(6)]
    a, b = leaf("x", rows), leaf("x", rows)
    assert a == b
    assert a.alternatives == b.alternatives and a.alternatives
