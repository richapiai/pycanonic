"""`canonic fit`: the leaf search (index, transform chains, rule choice, holdout)."""

from __future__ import annotations

from typing import Any

from pycanonic.cli.fit.chains import Chain, apply, candidates
from pycanonic.cli.fit.index import Occurrence, scalars
from pycanonic.cli.fit.leaves import LeafFit, _related, fit_leaf
from pycanonic.cli.fit.pairs import Pair

# --- leaf section ---


def leaf(field: str, rows: list[tuple[Any, Any]], lookups: bool = True, **kw: Any) -> LeafFit:
    """fit_leaf over pairs built from (target, provider body) rows."""
    pairs = [Pair(f"p{i}.json", {field: t}, body, None) for i, (t, body) in enumerate(rows)]
    return fit_leaf(field, pairs, [t for t, _ in rows], [scalars(p.provider) for p in pairs], lookups, **kw)


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
    assert (f.rule, f.kind, f.support, f.total, f.holdout) == ({"from": "job.title"}, "path", 6, 6, 1.0)


def test_cast_integer() -> None:
    f = leaf("size", [(42 + i, {"size": str(42 + i)}) for i in range(6)])
    assert f.rule == {"from": "size", "transform": [{"cast": "integer"}]}
    assert (f.kind, f.support) == ("cast", 6)


def test_strip_prefix_learned() -> None:
    f = leaf("id", [(f"{i}a{i}", {"urn": f"urn:x:{i}a{i}"}) for i in range(6)])
    assert f.rule == {"from": "urn", "transform": [{"strip_prefix": "urn:x:"}]}
    assert (f.kind, f.holdout) == ("strip_prefix", 1.0)


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
    assert (f.rule, f.kind, f.support, f.total) == ({"const": "person"}, "const", 5, 6)
    assert f.holdout == 5 / 6
    assert leaf("type", rows, floor=0.9).kind == "none"


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


def test_date_text_to_epoch() -> None:
    f = leaf("at", [(1614556800 + 86400 * i, {"day": f"2021-03-0{i + 1}"}) for i in range(6)])
    assert (f.rule, f.kind) == ({"from": "day", "transform": [{"format_date": "epoch_s"}]}, "format_date")
    f = leaf("at", [(1614556800000 + 86400000 * i, {"day": f"2021-03-0{i + 1}T00:00:00Z"}) for i in range(6)])
    assert (f.rule, f.kind) == ({"from": "day", "transform": [{"format_date": "epoch_ms"}]}, "format_date")


def test_epoch_string_to_date() -> None:
    f = leaf("at", [(f"2021-03-0{i + 1}T00:00:00.000Z", {"ts": str(1614556800 + 86400 * i)}) for i in range(6)])
    assert f.rule == {"from": "ts", "transform": [{"cast": "integer"}, {"format_date": "datetime_ms"}]}
    assert f.kind == "format_date"  # two safe steps: the second names the kind


def test_boolean_to_number() -> None:
    """The filter lets a boolean through for a number target. The runtime's cast gives null for a
    boolean, so no chain exists, and the learned lookup is what remains."""
    assert _related(True, 1) and _related(False, 0)
    assert candidates(1, True) == []
    f = leaf("n", [(i % 2, {"flag": i % 2 == 1}) for i in range(6)])
    assert (f.rule, f.kind) == ({"from": "flag", "transform": [{"lookup": {"table": {"false": 0, "true": 1}, "else": "null"}}]}, "lookup")


def test_two_step_kind_names_the_review_step() -> None:
    f = leaf("first", [(f"N{i}", {"name": f"n{i} l{i}"}) for i in range(6)])
    assert f.rule is not None and len(f.rule["transform"]) == 2
    assert f.kind == "split"  # the step off the safe list names the kind


def test_coalesce_scored_by_runtime() -> None:
    """The runtime's coalesce takes an object at the first path, so that pair does not count."""
    rows: list[tuple[Any, Any]] = [("v0", {"a": {"k": 1}, "b": "v0"})]
    rows += [(f"v{i}", {"a": f"v{i}"}) for i in range(1, 5)] + [("v5", {"b": "v5"})]
    f = leaf("value", rows)
    assert (f.rule, f.kind, f.support, f.total) == ({"coalesce": ["a", "b"]}, "coalesce", 5, 6)


def test_alternatives_skip_case_only_steps() -> None:
    f = leaf("ok", [(i % 2 == 0, {"ok": "yes" if i % 2 == 0 else "no"}) for i in range(6)])
    assert f.rule == {"from": "ok", "transform": [{"cast": "boolean"}]}
    for alt in f.alternatives:
        steps = alt["rule"].get("transform", [])
        assert not ({"cast": "boolean"} in steps and ({"upper", "lower", "title"} & {s for s in steps if isinstance(s, str)})), alt
