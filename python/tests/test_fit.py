"""`canonic fit`: the leaf search (index, transform chains, rule choice, holdout), arrays, structure, and the command."""

from __future__ import annotations

import doctest
import importlib.util
import json
from dataclasses import replace
from pathlib import Path
from typing import Any

import pytest

import pycanonic
from pycanonic.cli.__main__ import main

from pycanonic.cli.fit.arrays import fit_array
from pycanonic.cli.fit.chains import Chain, apply, apply_rule, candidates
from pycanonic.cli.fit import fit as fit_mod
from pycanonic.cli.fit import report
from pycanonic.cli.fit.compare import Tolerance, match, same
from pycanonic.cli.fit.decide import tier
from pycanonic.cli.fit.derive import derive_schema
from pycanonic.cli.fit.fit import FitOptions, exit_code, fit_pairs
from pycanonic.cli.fit.index import Occurrence, scalars
from pycanonic.cli.fit.leaves import LeafFit, _related, fit_leaf
from pycanonic.cli.fit.pairs import Pair, load_pairs
from pycanonic.cli.fit.report import FitResult, Row
from pycanonic.cli.fit.structure import fit_const, fit_object_presence, fit_when, propose_reject

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
    assert apply(Chain([("strip_prefix", "id:x:"), ("cast", "integer")]), "id:x:42") == 42
    assert apply(Chain([]), {"a": 1}) == {"a": 1}
    assert Chain([]) in candidates("Ada", "Ada")
    got = candidates(42, "id:x:42")
    assert Chain([("strip_prefix", "id:x:"), ("cast", "integer")]) in got
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
    f = leaf("id", [(f"{i}a{i}", {"ref": f"id:x:{i}a{i}"}) for i in range(6)])
    assert f.rule == {"from": "ref", "transform": [{"strip_prefix": "id:x:"}]}
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
    enum = [("UP", "liked"), ("STAR", "starred"), ("UP", "liked"), ("DOWN", "disliked"), ("STAR", "starred")]
    f = leaf("reaction", [(t, {"kind": s}) for s, t in enum])
    assert f.kind == "lookup"
    # DOWN is seen in one pair only, so the table leaves it out and that pair misses.
    assert f.rule == {"from": "kind", "transform": [{"lookup": {"table": {"STAR": "starred", "UP": "liked"}, "else": "null"}}]}
    assert (f.support, f.total) == (4, 5)
    assert leaf("reaction", [(t, {"kind": s}) for s, t in enum], lookups=False).kind == "none"


def test_lookup_by_majority_lands_in_review() -> None:
    """R44: one pair that disagrees does not hide the table. The majority wins per source value,
    the rule scores 11 of 12, and the failing pair is the first example."""
    enum = [("UP", "liked"), ("STAR", "starred"), ("DOWN", "disliked")] * 4
    rows = [(t, {"kind": s, "n": i}) for i, (s, t) in enumerate(enum)]
    rows[4] = ("fond", {"kind": "STAR", "n": 4})
    f = leaf("reaction", rows)
    assert f.kind == "lookup" and (f.support, f.total) == (11, 12)
    assert f.rule == {"from": "kind", "transform": [{"lookup": {"table": {"DOWN": "disliked", "STAR": "starred", "UP": "liked"}, "else": "null"}}]}
    assert f.examples[0] == {"pair": "p4.json", "want": "fond", "got": "starred"}
    assert tier(f) == "review"


def test_lookup_with_tied_votes_gets_no_table() -> None:
    """A value whose targets tie has no majority: the path is not an enum (a memorized table)."""
    enum = [("UP", "liked"), ("UP", "fond"), ("STAR", "starred"), ("STAR", "starred"), ("DOWN", "disliked")]
    f = leaf("reaction", [(t, {"kind": s}) for s, t in enum])
    assert (f.kind, f.rule, f.support) == ("none", None, 0)


def test_lookup_keeps_only_values_seen_in_two_sample_pairs() -> None:
    """A table entry seen in one sample pair memorizes that pair. Elements of one response count
    as one pair (`owners`), so a value repeated inside one response is still seen once."""
    rows = [("liked", {"kind": "UP"})] * 4 + [("starred", {"kind": "STAR"})] * 4 + [("disliked", {"kind": "DOWN"})] * 2
    pairs = [Pair(f"e{i}.json", {"reaction": t}, b, None) for i, (t, b) in enumerate(rows)]
    targets = [t for t, _ in rows]
    owners = [0, 1, 2, 3, 4, 5, 6, 7, 8, 8]  # both DOWN elements come from sample pair 8
    f = fit_leaf("reaction", pairs, targets, [scalars(p.provider) for p in pairs], True, owners=owners)
    assert f.kind == "lookup"
    assert f.rule == {"from": "kind", "transform": [{"lookup": {"table": {"STAR": "starred", "UP": "liked"}, "else": "null"}}]}
    assert (f.support, f.total) == (8, 10)


def test_lookup_of_one_off_values_is_no_table() -> None:
    """Company name to domain over five profiles: every value is seen in one pair, so no table."""
    rows = [(f"c{i}.example", {"company": f"Co {i}"}) for i in range(5)] + [("c0.example", {"company": "Co 0"})]
    pairs = [Pair(f"e{i}.json", {"domain": t}, b, None) for i, (t, b) in enumerate(rows)]
    f = fit_leaf("domain", pairs, [t for t, _ in rows], [scalars(p.provider) for p in pairs], True, owners=[0, 1, 2, 3, 4, 0])
    assert (f.kind, f.rule) == ("none", None)


def test_argument_learned_from_one_pair_is_not_kept() -> None:
    """A suffix that only one sample pair carries is that pair's text, not a rule: the plain path wins."""
    rows = [(f"School {i}", {"school": f"School {i}"}) for i in range(5)] + [("Big School", {"school": "Big School, Oslo"})]
    f = leaf("name", rows)
    assert (f.rule, f.kind, f.support, f.total) == ({"from": "school"}, "path", 5, 6)
    rows = [(f"School {i}", {"school": f"School {i}, Campus"}) for i in range(6)]  # every pair carries it
    assert leaf("name", rows).rule == {"from": "school", "transform": [{"strip_suffix": ", Campus"}]}


def test_holdout_catches_overfit() -> None:
    rows = [(f"v{i}", {"a": f"v{i}" if i else "x", "n": i}) for i in range(6)]
    f = leaf("type", rows)
    assert (f.rule, f.kind, f.support, f.total) == ({"from": "a"}, "path", 5, 6)
    assert f.holdout == 5 / 6
    assert leaf("type", rows, floor=0.9).kind == "none"
    assert leaf("type", [("person" if i else "company", {"n": i}) for i in range(6)]).kind == "none"  # 5 of 6 is no constant


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
    rows = [(f"{i}", {"a": f"{i}", "b": f"{i}", "c": i, "d": f"id:{i}"}) for i in range(6)]
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
    """The runtime's coalesce takes an object at the first path, so `a` first misses that pair and
    `b` first wins."""
    rows: list[tuple[Any, Any]] = [("v0", {"a": {"k": 1}, "b": "v0"})]
    rows += [(f"v{i}", {"a": f"v{i}"}) for i in range(1, 5)] + [("v5", {"b": "v5"})]
    f = leaf("value", rows)
    assert (f.rule, f.kind, f.support, f.total) == ({"coalesce": ["b", "a"]}, "coalesce", 6, 6)
    assert apply_rule({"coalesce": ["a", "b"]}, rows[0][1]) == {"k": 1}  # a first: the object, a miss


def test_alternatives_skip_case_only_steps() -> None:
    f = leaf("ok", [(i % 2 == 0, {"ok": "yes" if i % 2 == 0 else "no"}) for i in range(6)])
    assert f.rule == {"from": "ok", "transform": [{"cast": "boolean"}]}
    for alt in f.alternatives:
        steps = alt["rule"].get("transform", [])
        assert not ({"cast": "boolean"} in steps and ({"upper", "lower", "title"} & {s for s in steps if isinstance(s, str)})), alt


def test_coalesce_tries_both_orders() -> None:
    """`a` alone hits 5 of 6, `b` first then `a` hits all 6, `a` first then `b` stays at 5."""
    rows: list[tuple[Any, Any]] = [(f"v{i}", {"a": f"v{i}"}) for i in range(3)]
    rows += [("v3", {"a": "wrong", "b": "v3"})] + [(f"v{i}", {"a": f"v{i}", "b": f"v{i}"}) for i in (4, 5)]
    f = leaf("value", rows)
    assert (f.rule, f.kind, f.support) == ({"coalesce": ["b", "a"]}, "coalesce", 6)


# --- array section ---

S = {"type": "string"}


def objs(**props: Any) -> dict[str, Any]:
    return {"type": "array", "items": {"type": "object", "properties": props}}


def pairs_of(rows: list[tuple[Any, Any]], inputs: list[Any] | None = None) -> list[Pair]:
    return [Pair(f"p{i}.json", d, p, inputs[i] if inputs else None) for i, (d, p) in enumerate(rows)]


def test_item_alignment() -> None:
    rows = [
        ({"experiences": [{"title": f"t{i}{k}", "company": f"c{i}{k}"} for k in range(i % 3 + 1)]}, {"workHistory": [{"role": f"t{i}{k}", "orgName": f"c{i}{k}", "n": k} for k in range(i % 3 + 1)], "tags": ["x"]})
        for i in range(6)
    ]
    rule, fits = fit_array("experiences[]", objs(title=S, company=S), pairs_of(rows))
    assert rule == {"from": "workHistory", "item": {"title": {"from": "role"}, "company": {"from": "orgName"}}}
    assert [(f.field, f.kind, f.support, f.total) for f in fits] == [("experiences[]", "path", 6, 6), ("experiences[].title", "path", 12, 12), ("experiences[].company", "path", 12, 12)]


def test_pluck() -> None:
    rows = [({"skills": [f"s{i}{k}" for k in range(i % 2 + 1)]}, {"tags": [{"label": f"S{i}{k}", "w": k} for k in range(i % 2 + 1)]}) for i in range(6)]
    rule, fits = fit_array("skills[]", {"type": "array", "items": S}, pairs_of(rows))
    assert rule == {"from": "tags", "pluck": "label", "transform": ["lower"]}
    assert (fits[0].kind, fits[0].support) == ("lower", 6)


def test_wrap_detected() -> None:
    rows = [({"industries": [f"i{i}"]}, {"industry": f"i{i}", "n": i}) for i in range(6)]
    rule, fits = fit_array("industries[]", {"type": "array", "items": S}, pairs_of(rows))
    assert rule == {"from": "industry", "wrap": True}
    assert (fits[0].kind, fits[0].support) == ("wrap", 6)


def test_flatten_detected() -> None:
    rows = [({"skills": [f"s{i}a", f"s{i}b", f"s{i}c"]}, {"jobs": [{"skills": [f"s{i}a", f"s{i}b"]}, {"title": "x"}, {"skills": [f"s{i}c"]}]}) for i in range(6)]
    rule, fits = fit_array("skills[]", {"type": "array", "items": S}, pairs_of(rows))
    assert rule == {"from": "jobs[].skills[]"}
    assert (fits[0].kind, fits[0].support) == ("flatten", 6)


def test_root_array() -> None:
    node = objs(name=S)
    rows = [([{"name": f"n{i}{k}"} for k in range(i % 3 + 1)], {"results": [{"n": f"n{i}{k}"} for k in range(i % 3 + 1)]}) for i in range(5)]
    assert fit_array("[]", node, pairs_of(rows))[0] == {"from": "results", "item": {"name": {"from": "n"}}}
    rule, fits = fit_array("[]", node, pairs_of([(d, p["results"]) for d, p in rows]))
    assert rule == {"from": "$", "item": {"name": {"from": "n"}}}
    assert [f.field for f in fits] == ["[]", "[].name"]


GROUPS = [["A", "A", "B", "A"], ["A", "B"], ["C", "C", "C"], ["A", "B", "A"], ["B", "B", "A"], ["C", "A", "C", "C"]]


def grouped(i: int, keys: list[str]) -> tuple[Any, Any]:
    """Positions with company keys `keys`; the default groups them consecutively."""
    pos = [{"company_id": k, "title": f"t{i}{n}", "start": f"2020-0{(n * 7 + i) % 9 + 1}", "end": None if (n * 3 + i) % 7 == 0 else f"2024-0{9 - abs(n - 1) * 3}"} for n, k in enumerate(keys)]
    runs: list[list[dict[str, Any]]] = []
    for p in pos:
        if runs and runs[-1][0]["company_id"] == p["company_id"]:
            runs[-1].append(p)
        else:
            runs.append([p])
    ends = [[p["end"] for p in r] for r in runs]
    jobs = [
        {"company_id": r[0]["company_id"], "start": min(p["start"] for p in r), "end": None if None in e else max(x for x in e if x), "roles": [{"title": p["title"]} for p in r]}
        for r, e in zip(runs, ends)
    ]
    return {"jobs": jobs}, {"positions": pos}


JOBS = objs(company_id=S, start=S, end=S, roles=objs(title=S))


def test_group_detected_with_gaps() -> None:
    rows = [grouped(i, k) for i, k in enumerate(GROUPS)]
    assert len(rows[3][0]["jobs"]) == 3  # A, B, A is three groups
    rule, fits = fit_array("jobs[]", JOBS, pairs_of(rows))
    assert rule is not None
    assert {k: rule[k] for k in ("from", "group_by", "consecutive")} == {"from": "positions", "group_by": "company_id", "consecutive": True}
    assert rule["item"]["company_id"] == {"from": "company_id"}
    assert rule["item"]["roles[]"] == {"from": "$group", "item": {"title": {"from": "title"}}}
    assert (fits[0].kind, fits[0].support, fits[0].total) == ("group", 6, 6)
    assert "jobs[].roles[].title" in [f.field for f in fits]
    roles = {f.field: f for f in fits}["jobs[].roles[]"]
    assert (roles.rule, roles.kind) == ({"from": "$group", "item": {"title": {"from": "title"}}}, "path")  # the spec's rule, not the internal key
    assert all(a["rule"]["from"].startswith("$group") for a in roles.alternatives)


def test_group_aggregates_min_max_propagate() -> None:
    rows = [grouped(i, k) for i, k in enumerate(GROUPS)]
    rule, fits = fit_array("jobs[]", JOBS, pairs_of(rows))
    assert rule is not None
    assert rule["item"]["start"] == {"aggregate": "min", "from": "$group[].start"}
    assert rule["item"]["end"] == {"aggregate": "max", "from": "$group[].end", "nulls": "propagate"}
    by = {f.field: f for f in fits}
    assert by["jobs[].end"].kind == "group"


def test_filter_detected() -> None:
    def row(i: int) -> tuple[Any, Any]:
        people = [{"name": f"n{i}{k}", "status": "active" if (k + i) % 3 else "gone", "role": "x"} for k in range(4)]
        return {"people": [{"name": p["name"]} for p in people if p["status"] == "active"]}, {"people": people}

    rule, fits = fit_array("people[]", objs(name=S), pairs_of([row(i) for i in range(6)]))
    assert rule == {"from": "people", "filter": {"path": "status", "eq": "active"}, "item": {"name": {"from": "name"}}}
    assert (fits[0].kind, fits[0].support) == ("filter", 6)


def test_item_holdout_leaves_out_whole_pairs() -> None:
    """Only p0's elements read `b`. Leaving one of them out, the other two still teach the coalesce;
    leaving all of p0 out gives `a` alone, which misses p0."""
    rows = [({"people": [{"title": f"v0{k}"} for k in range(3)]}, {"people": [{"b": f"v0{k}"} for k in range(3)]})]
    rows += [({"people": [{"title": f"v{i}{k}"} for k in range(2)]}, {"people": [{"a": f"v{i}{k}"} for k in range(2)]}) for i in range(1, 6)]
    rule, fits = fit_array("people[]", objs(title=S), pairs_of(rows))
    assert rule == {"from": "people", "item": {"title": {"coalesce": ["a", "b"]}}}
    by = {f.field: f for f in fits}
    assert by["people[].title"].holdout == 5 / 6
    assert by["people[]"].holdout == 5 / 6
    eps = [Pair(f"e{n}", d, e, None) for d0, p0 in rows for n, (d, e) in enumerate(zip(d0["people"], p0["people"]))]
    per_element = fit_leaf("title", eps, [e.default["title"] for e in eps], [scalars(e.provider) for e in eps], True)
    assert per_element.holdout == 1.0  # what the element-by-element holdout said


def test_fitted_rules_map_on_every_path() -> None:
    """The fitted group and filter rules, compiled as full specs, reproduce the defaults through
    map(bytes), map(dict) and map_json."""

    def people(i: int) -> tuple[Any, Any]:
        ps = [{"name": f"n{i}{k}", "status": "active" if (k + i) % 3 else "gone"} for k in range(4)]
        return {"people": [{"name": p["name"]} for p in ps if p["status"] == "active"]}, {"people": ps}

    cases = [("jobs", JOBS, [grouped(i, k) for i, k in enumerate(GROUPS)]), ("people", objs(name=S), [people(i) for i in range(6)])]
    for key, node, rows in cases:
        rule, _ = fit_array(f"{key}[]", node, pairs_of(rows))
        assert rule is not None and ("group_by" in rule or "filter" in rule)
        spec = {"language": 2, "provider": "acme", "endpoint": "e", "version": 1, "status": "draft", "fields": {f"{key}[]": rule}}
        art = pycanonic.compile_spec(spec, {"type": "object", "properties": {key: node}})
        for want, body in rows:
            raw = json.dumps(body).encode()
            for got in (art.map(raw), art.map(body), json.loads(art.map_json(raw))):
                assert same(got, want), (key, got, want)


def test_array_not_found() -> None:
    rows = [({"skills": [f"s{i}"]}, {"other": [1, 2], "n": i}) for i in range(4)]
    rule, fits = fit_array("skills[]", {"type": "array", "items": S}, pairs_of(rows))
    assert rule is None and [(f.field, f.kind, f.total) for f in fits] == [("skills[]", "none", 4)]
    assert fit_array("skills[]", {"type": "array", "items": S}, pairs_of([({}, {"a": [1]})])) == (None, [])


def test_filter_picks_best_source() -> None:
    """The decoy `aaa` comes first and passes the filter check, but its cities are wrong."""

    def row(i: int) -> tuple[Any, Any]:
        ps = [{"name": f"n{i}{k}", "city": f"c{i}{k}", "status": "active" if (k + i) % 3 else "gone"} for k in range(4)]
        return {"people": [{"name": p["name"], "city": p["city"]} for p in ps if p["status"] == "active"]}, {"aaa": [{**p, "city": "x"} for p in ps], "people": ps}

    rule, fits = fit_array("people[]", objs(name=S, city=S), pairs_of([row(i) for i in range(6)]))
    assert rule is not None and "filter" in rule and rule["from"] == "people"
    assert (fits[0].kind, fits[0].support) == ("filter", 6)


def _code(i: int, k: int) -> str:
    """A value no provider body in these tests shares a token, a digit or a substring with."""
    return "zz" + "uvwxy"[i % 5] + "uvwxy"[k % 5]


def _fit_rows(key: str, node: dict[str, Any], rows: list[tuple[Any, Any]]) -> dict[str, Row]:
    res = fit_pairs(pairs_of(rows), {"type": "object", "properties": {key: node}}, FitOptions("acme", "e"))
    return {r.field: r for r in res.fields}


def test_item_leaves_the_provider_lacks_do_not_block_the_array() -> None:
    """The default's elements carry `ref` and `page`, which the provider never sends. The three
    leaves it does send fit, so the array has a rule, and the other two are absent rows."""

    def row(i: int) -> tuple[Any, Any]:
        n = range(i % 3 + 1)
        d = {"jobs": [{"title": f"t{i}{k}", "org": f"o{i}{k}", "start": f"2020-0{k + i % 2 + 1}", "ref": _code(i, k), "page": _code(k, i)} for k in n]}
        return d, {"work": [{"role": f"t{i}{k}", "orgName": f"o{i}{k}", "from": f"2020-0{k + i % 2 + 1}"} for k in n]}

    rows = [row(i) for i in range(6)]
    node = objs(title=S, org=S, start=S, ref=S, page=S)
    rule, fits = fit_array("jobs[]", node, pairs_of(rows))
    assert rule == {"from": "work", "item": {"title": {"from": "role"}, "org": {"from": "orgName"}, "start": {"from": "from"}}}
    kinds = {f.field: (f.kind, f.support) for f in fits}
    assert kinds["jobs[]"] == ("path", 6)
    assert [kinds[f"jobs[].{k}"][0] for k in ("title", "org", "start", "ref", "page")] == ["path", "path", "path", "absent", "absent"]
    by = _fit_rows("jobs", node, rows)  # the runtime check keeps the array
    assert (by["jobs[]"].kind, by["jobs[]"].support, by["jobs[]"].total) == ("path", 6, 6)
    assert (by["jobs[].ref"].kind, by["jobs[].page"].kind) == ("absent", "absent")


def test_nested_item_object_with_a_leaf_the_provider_lacks() -> None:
    def row(i: int) -> tuple[Any, Any]:
        n = range(i % 2 + 1)
        d = {"edu": [{"degree": f"d{i}{k}", "school": {"name": f"s{i}{k}", "id": _code(i, k)}} for k in n]}
        return d, {"studies": [{"title": f"d{i}{k}", "schoolName": f"s{i}{k}"} for k in n]}

    rows = [row(i) for i in range(6)]
    node = objs(degree=S, school={"type": "object", "properties": {"name": S, "id": S}})
    rule, fits = fit_array("edu[]", node, pairs_of(rows))
    assert rule == {"from": "studies", "item": {"degree": {"from": "title"}, "school.name": {"from": "schoolName"}}}
    kinds = {f.field: f.kind for f in fits}
    assert (kinds["edu[]"], kinds["edu[].school.name"], kinds["edu[].school.id"]) == ("path", "path", "absent")
    by = _fit_rows("edu", node, rows)
    assert (by["edu[]"].kind, by["edu[]"].support) == ("path", 6)


def test_grouped_array_with_a_group_leaf_the_provider_lacks() -> None:
    rows = [grouped(i, k) for i, k in enumerate(GROUPS)]
    for i, (d, _) in enumerate(rows):
        for n, job in enumerate(d["jobs"]):
            job["slug"] = _code(i, n)
    node = objs(company_id=S, start=S, end=S, slug=S, roles=objs(title=S))
    rule, fits = fit_array("jobs[]", node, pairs_of(rows))
    assert rule is not None and rule["group_by"] == "company_id" and "slug" not in rule["item"]
    kinds = {f.field: (f.kind, f.support) for f in fits}
    assert kinds["jobs[]"] == ("group", 6)
    assert kinds["jobs[].slug"][0] == "absent"
    by = _fit_rows("jobs", node, rows)
    assert (by["jobs[]"].kind, by["jobs[]"].support) == ("group", 6)


def test_item_leaf_right_on_most_elements_does_not_reject_the_array() -> None:
    """`org` is right on 9 of 10 elements: the array keeps its rule, and `org` is its own row."""

    def row(i: int) -> tuple[Any, Any]:
        d = {"jobs": [{"title": f"t{i}{k}", "org": f"o{i}{k}"} for k in range(2)]}
        return d, {"work": [{"role": f"t{i}{k}", "orgName": "zzq" if (i, k) == (0, 0) else f"o{i}{k}"} for k in range(2)]}

    rows = [row(i) for i in range(5)]
    node = objs(title=S, org=S)
    rule, fits = fit_array("jobs[]", node, pairs_of(rows))
    assert rule == {"from": "work", "item": {"title": {"from": "role"}, "org": {"from": "orgName"}}}
    by = _fit_rows("jobs", node, rows)
    assert (by["jobs[]"].kind, by["jobs[]"].support, by["jobs[]"].total) == ("path", 5, 5)
    org = by["jobs[].org"]
    assert (org.support, org.total) == (9, 10) and org.tier in ("review", "manual")


def test_decoy_array_of_the_same_length_is_still_rejected() -> None:
    rows = [({"jobs": [{"title": f"t{i}{k}"} for k in range(2)]}, {"other": [{"x": _code(i, k)} for k in range(2)]}) for i in range(5)]
    rule, fits = fit_array("jobs[]", objs(title=S), pairs_of(rows))
    assert rule is None and fits[0].kind == "none"


def test_grouped_array_with_a_group_leaf_right_on_most_groups() -> None:
    rows = [grouped(i, k) for i, k in enumerate(GROUPS)]
    rows[0][0]["jobs"][0]["start"] = "2019-01"  # one of 14 groups disagrees
    rule, fits = fit_array("jobs[]", JOBS, pairs_of(rows))
    assert rule is not None and rule["item"]["start"] == {"aggregate": "min", "from": "$group[].start"}
    by = {f.field: f for f in fits}
    assert (by["jobs[]"].kind, by["jobs[]"].support) == ("group", 6)
    assert (by["jobs[].start"].support, by["jobs[].start"].total) == (13, 14)
    assert _fit_rows("jobs", JOBS, rows)["jobs[]"].support == 6


# --- structure section ---


def test_presence_detected() -> None:
    rows = [({"name": f"n{i}", "hq": {"city": f"c{i}"} if i % 2 else None}, {"n": f"n{i}", "data": {"hq": {"city": f"c{i}", "zip": i}} if i % 2 else {"x": 1}}) for i in range(6)]
    assert fit_object_presence("hq", pairs_of(rows)) == {"present": "data.hq"}
    rows = [({"hq": {"city": "c"}}, {"data": {"hq": {"city": "c"}}}) for _ in range(4)]
    assert fit_object_presence("hq", pairs_of(rows)) is None  # never null: nothing to learn


def test_const_detected_only_without_source() -> None:
    assert fit_const("kind", ["person"] * 6) == {"const": "person"}
    assert fit_const("tags", [[]] * 4) == {"const": []}
    assert fit_const("kind", ["person"] * 5 + ["org"]) is None
    assert fit_const("kind", [None] * 4) is None
    with_source = leaf("kind", [("person", {"type": "person", "n": i}) for i in range(6)])
    assert (with_source.rule, with_source.kind) == ({"from": "type"}, "path")
    without = leaf("kind", [("person", {"n": i}) for i in range(6)])
    assert without.kind == "const"


def test_const_must_hold_in_every_pair_null_included() -> None:
    """A value the default sends in one pair of eight and leaves null in the rest is no constant."""
    rows: list[tuple[Any, Any]] = [(None, {"n": i}) for i in range(7)] + [("1.8", {"n": 7})]
    f = leaf("ver", rows)
    assert f.kind != "const" and f.rule is None
    assert fit_const("ver", [r[0] for r in rows]) is None
    assert fit_const("ver", ["1.8"] + [None] * 7) is None
    assert leaf("ver", [("1.8", {"n": i}) for i in range(8)]).rule == {"const": "1.8"}

    def item(i: int) -> tuple[Any, Any]:
        n = range(2)
        return {"tools": [{"name": f"n{i}{k}", "ver": "1.8" if (i, k) == (0, 0) else None} for k in n]}, {"kit": [{"label": f"n{i}{k}"} for k in n]}

    rule, fits = fit_array("tools[]", objs(name=S, ver=S), pairs_of([item(i) for i in range(4)]))
    assert rule == {"from": "kit", "item": {"name": {"from": "label"}}}
    assert {f.field: f.kind for f in fits}["tools[].ver"] != "const"


def test_when_from_inputs() -> None:
    inputs: list[Any] = [{"full": i % 2 == 0, "page": i} for i in range(6)]
    rows = [({"name": f"n{i}", **({"bio": f"b{i}"} if i % 2 == 0 else {})}, {"bio": f"b{i}"}) for i in range(6)]
    assert fit_when("bio", pairs_of(rows, inputs)) == {"when": {"path": "$input.full", "eq": True}, "omit": True}
    rows = [({"bio": f"b{i}" if i % 2 == 0 else None}, {"bio": f"b{i}"}) for i in range(6)]
    assert fit_when("bio", pairs_of(rows, inputs)) == {"when": {"path": "$input.full", "eq": True}}
    assert fit_when("name", pairs_of([({"name": "a"}, {})] * 3, [{"full": True}] * 3)) is None


def test_reject_proposal() -> None:
    schema = {"type": "object", "properties": {"name": S, "title": S, "jobs": objs(title=S)}}
    rows = [({"name": f"n{i}", "title": f"t{i}" if i % 3 else None, "jobs": []}, {"title": f"t{i}", **({"full_name": f"n{i}"} if i % 3 else {})}) for i in range(6)]
    assert propose_reject(schema, pairs_of(rows)) == [{"missing": "full_name"}]
    full = [(d, {**p, "full_name": d["name"]}) for d, p in rows]
    assert propose_reject(schema, pairs_of(full)) == []


def test_reject_needs_a_mostly_empty_body() -> None:
    """A provider that leaves out one field the default always had is not a failover; a body that
    lacks most fitted sources (an error body) is, and its conditions collapse to the first one."""
    schema = {"type": "object", "properties": {"name": S, "title": S, "bio": S, "city": S}}
    def row(i: int, body: dict[str, Any]) -> tuple[Any, Any]:
        return ({"name": f"n{i}", "title": f"t{i}", "bio": f"b{i}", "city": f"c{i}"}, body)
    full = lambda i: {"full_name": f"n{i}", "role": f"t{i}", "about": f"b{i}", "town": f"c{i}"}
    rows = [row(i, full(i)) for i in range(5)]
    rows.append(row(5, {k: v for k, v in full(5).items() if k != "about"}))  # a real profile without a bio
    assert propose_reject(schema, pairs_of(rows)) == []
    rows.append(row(6, {"error": {"type": "not_found"}}))
    assert propose_reject(schema, pairs_of(rows)) == [{"missing": "full_name"}]


# --- command section ---

FIXTURE = Path(__file__).parent / "fixtures" / "fit" / "acme-to-globex"
NEVER_AUTO = ["lookup", "const", "regex", "group", "omit", "when", "reject", "null_if", "keys", "absent"]


def lf(kind: str, support: int, total: int, holdout: float = 1.0) -> LeafFit:
    return LeafFit("f", {"from": "f"}, kind, support, total, holdout, [], [])


def fixture_pairs() -> list[Pair]:
    pairs, skipped = load_pairs(FIXTURE / "default", FIXTURE / "provider", None)
    assert skipped == [] and len(pairs) == 10
    return pairs


def fit_fixture(**kw: Any) -> FitResult:
    pairs = fixture_pairs()
    return fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("globex", "person.enrich", **kw))


def cli(tmp: Path, *extra: str) -> int:
    args = ["fit", "--default", str(FIXTURE / "default"), "--provider", str(FIXTURE / "provider"), "--provider-name", "globex", "--endpoint", "person.enrich"]
    return main([*args, "--out", str(tmp / "spec.json"), "--report", str(tmp / "report"), *extra])


def handle_pairs(n: int = 6) -> list[Pair]:
    """`handle` is the last and first name, lower case, joined by a dot: no learned chain gives it."""
    names = [("Ada", "Lane"), ("Bo", "Marsh"), ("Cy", "North"), ("Di", "Oak"), ("Ed", "Pike"), ("Flo", "Quill")][:n]
    return [Pair(f"p{i}.json", {"first": f, "handle": f"{l.lower()}.{f.lower()}"}, {"person": {"first": f, "last": l}}, None) for i, (f, l) in enumerate(names)]


def test_tier_auto_needs_5_pairs() -> None:
    assert tier(lf("path", 5, 5)) == "auto"
    assert tier(lf("path", 4, 4)) == "review"
    assert tier(lf("path", 4, 4), min_pairs=4) == "auto"
    assert tier(lf("path", 9, 10)) == "review"  # 90% support
    assert tier(lf("path", 10, 10, 0.9)) == "review"  # holdout below 95%
    assert tier(lf("path", 7, 10)) == "manual"  # below the floor
    assert tier(lf("none", 0, 10)) == tier(lf("disagree", 0, 10)) == "manual"


@pytest.mark.parametrize("kind", NEVER_AUTO)
def test_tier_never_auto_kinds(kind: str) -> None:
    assert tier(lf(kind, 20, 20)) == "review"
    if kind in ("absent", "reject"):  # support is not a match share for these: never manual either
        assert tier(lf(kind, 1, 30)) == "review"


def test_tier_mixed_empty_never_auto() -> None:
    assert tier(replace(lf("empty", 10, 10), mixed_empty=True)) == "review"
    assert tier(replace(lf("path", 10, 10), mixed_empty=True)) == "review"
    pairs = [Pair(f"p{i}.json", {"name": f"n{i}", "tags": None if i % 2 else []}, {"n": f"n{i}"}, None) for i in range(10)]
    rows = {r.field: r for r in fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e")).fields}
    assert (rows["tags[]"].kind, rows["tags[]"].tier, rows["name"].tier) == ("empty", "review", "auto")


def test_exit_codes() -> None:
    def result(*tiers: str, skipped: list[tuple[str, str]] | None = None) -> FitResult:
        rows = [Row(f"f{i}", t, "path" if t != "review" else "lookup", None, 1, 1, 1.0, []) for i, t in enumerate(tiers)]
        return FitResult("a", "e", 5, rows, None, "", skipped or [])

    assert exit_code(result("auto", "auto")) == 0
    assert exit_code(result("auto", "review")) == 1
    assert exit_code(result("auto", "review", "manual")) == 2
    assert exit_code(result("auto", "review"), strict=True) == 2
    assert exit_code(result("auto"), strict=True) == 0
    assert exit_code(result("auto", skipped=[("x.json", "not JSON")])) == 2


def test_report_json_layout(tmp_path: Path) -> None:
    result = fit_fixture()
    report.write_json(result, tmp_path / "r.json")
    got = json.loads((tmp_path / "r.json").read_text())
    assert list(got) == ["provider", "endpoint", "pairs", "skipped", "fields", "summary"]
    assert (got["provider"], got["endpoint"], got["pairs"], got["skipped"]) == ("globex", "person.enrich", 10, [])
    assert all(list(f) == ["field", "tier", "kind", "rule", "support", "total", "holdout", "examples", "risk"] for f in got["fields"])
    assert list(got["summary"]) == ["auto", "review", "manual"]
    assert sum(got["summary"].values()) == len(got["fields"])


def test_report_md_lists_review_with_risk(tmp_path: Path) -> None:
    result = fit_fixture()
    report.write_md(result, tmp_path / "r.md")
    md = (tmp_path / "r.md").read_text()
    assert "10 pairs. " in md.splitlines()[2]
    assert "| email | path | `\"contact.email\"` | 10 of 10 | 100% |" in md
    section = md.split("## jobs[]: review, group")[1].split("\n## ")[0]
    assert "- evidence: 10 of 10 pairs, holdout 100%" in section
    assert report.RISKS["group"] in section
    assert "## status: manual, disagree" in md and report.RISKS["disagree"] in md


def test_keep_preserves_rule(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    kept = {"from": "state", "transform": [{"lookup": {"table": {"active": "active", "inactive": "inactive"}, "else": "null"}}]}
    (tmp_path / "keep.json").write_text(json.dumps({"status": kept}))
    code = cli(tmp_path, "--keep", str(tmp_path / "keep.json"))
    spec = json.loads((tmp_path / "spec.json").read_text())
    assert spec["fields"]["status"] == kept
    assert list(spec["fields"])[:5] == ["id", "name", "email", "joined", "status"]  # schema order
    row = next(f for f in json.loads((tmp_path / "report.json").read_text())["fields"] if f["field"] == "status")
    assert (row["tier"], row["kind"], row["rule"], row["support"], row["total"]) == ("review", "kept", kept, 7, 10)
    assert "## status: review, kept by reviewer" in (tmp_path / "report.md").read_text()
    assert code == 1  # hq and jobs[] still need review; the kept field does not raise the code


def test_changed_warning(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert cli(tmp_path) == 2
    spec = json.loads((tmp_path / "spec.json").read_text())
    spec["fields"]["email"] = "contact.other"
    (tmp_path / "spec.json").write_text(json.dumps(spec))
    capsys.readouterr()
    cli(tmp_path)
    err = capsys.readouterr().err
    assert "fit: email changed since the last spec; add it to --keep to hold it" in err
    assert "fit: name changed" not in err
    (tmp_path / "keep.json").write_text(json.dumps({"email": "contact.other"}))
    (tmp_path / "spec.json").write_text(json.dumps(spec))
    cli(tmp_path, "--keep", str(tmp_path / "keep.json"))
    assert "email changed" not in capsys.readouterr().err


def test_too_few_pairs(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    for side in ("d", "p"):
        (tmp_path / side).mkdir()
        for i in range(2):
            (tmp_path / side / f"{i}.json").write_text(json.dumps({"a": i}))
    (tmp_path / "d" / "bad.json").write_text("{")
    (tmp_path / "p" / "bad.json").write_text("{}")
    args = ["fit", "--default", str(tmp_path / "d"), "--provider", str(tmp_path / "p"), "--provider-name", "a", "--endpoint", "e", "--out", str(tmp_path / "s.json")]
    assert main(args) == 2
    assert "need at least 3 pairs, got 2" in capsys.readouterr().err
    assert not (tmp_path / "s.json").exists()
    for side in ("d", "p"):
        (tmp_path / side / "2.json").write_text(json.dumps({"a": 2}))
    main(args)
    assert "fit: only 3 pairs" in capsys.readouterr().err


def test_transforms_stub_has_doctests(tmp_path: Path) -> None:
    pairs = handle_pairs()
    result = fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e"))
    row = {r.field: r for r in result.fields}["handle"]
    assert (row.tier, row.kind) == ("manual", "none")
    assert result.spec is not None and result.spec["fields"]["handle"] == {"const": None}  # placeholder (R66)
    assert "def handle(body):" in result.transforms
    (tmp_path / "stubs.py").write_text(result.transforms)
    module = importlib.util.module_from_spec(spec := importlib.util.spec_from_file_location("stubs", tmp_path / "stubs.py"))  # type: ignore[arg-type]
    assert spec is not None and spec.loader is not None
    spec.loader.exec_module(module)
    tests = doctest.DocTestFinder().find(module.handle)
    runner = doctest.DocTestRunner(optionflags=doctest.REPORT_ONLY_FIRST_FAILURE)
    for t in tests:
        runner.run(t, out=lambda _: None)
    assert (runner.failures, runner.tries) == (3, 3)  # the stub returns None until someone writes it
    module.handle = lambda body: f"{body['person']['last'].lower()}.{body['person']['first'].lower()}"
    assert all(module.handle(p.provider) == p.default["handle"] for p in pairs)


def test_ask_answers_are_review() -> None:
    pairs = handle_pairs()
    asked: list[Any] = []

    def ask(q: dict[str, Any]) -> str:
        asked.append(q["field"])
        return "person.last"

    result = fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e", ask=ask))
    row = {r.field: r for r in result.fields}["handle"]
    assert asked == ["handle"]  # manual leaves only, not `first`
    assert (row.tier, row.kind, row.rule) == ("review", "asked", "person.last")
    assert result.spec is not None and result.spec["fields"]["handle"] == "person.last"


def test_absent_field_stays_unmapped() -> None:
    pairs = [Pair(f"p{i}.json", {"name": f"n{i}", "nickname": f"zz{i}q"}, {"name": f"n{i}"}, None) for i in range(6)]
    result = fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e"))
    row = {r.field: r for r in result.fields}["nickname"]
    assert (row.tier, row.kind, row.risk()) == ("review", "absent", report.RISKS["absent"])
    assert result.spec is not None and list(result.spec["fields"]) == ["name"]


def test_runtime_check_downgrades(monkeypatch: pytest.MonkeyPatch) -> None:
    """A rule that fit reports as matching but the runtime does not reproduce is not written."""
    real = fit_mod.fit_leaf

    def wrong(field: str, *a: Any, **kw: Any) -> LeafFit:
        f = real(field, *a, **kw)
        return replace(f, rule={"from": "other"}) if field == "email" else f

    monkeypatch.setattr(fit_mod, "fit_leaf", wrong)
    result = fit_fixture()
    row = {r.field: r for r in result.fields}["email"]
    assert (row.tier, row.kind, row.rule) == ("manual", "none", None)
    assert row.note == 'Runtime check: the rule "other" maps 0 pairs, fit found 10. placeholder: the draft writes null, the schema\'s empty form, until a rule replaces it.'
    assert row.examples[0] == {"pair": "p01.json", "want": "ada@mail0.example.test", "got": None}
    assert result.spec is not None and result.spec["fields"]["email"] == {"const": None}


def test_end_to_end_fixture(tmp_path: Path) -> None:
    assert cli(tmp_path, "--transforms-out", str(tmp_path / "t.py")) == 2  # status is manual
    spec = json.loads((tmp_path / "spec.json").read_text())
    assert (spec["language"], spec["status"], spec["version"]) == (2, "draft", 1)
    f = spec["fields"]
    assert list(f) == ["id", "name", "email", "joined", "status", "hq", "hq.city", "tags[]", "jobs[]"]
    assert f["status"] == {"const": None}  # the manual field's placeholder (R66)
    assert f["email"] == "contact.email" and f["hq.city"] == "office.town"
    assert f["jobs[]"]["group_by"] == "org" and f["jobs[]"]["item"]["org"] == "org"
    assert f["jobs[]"]["item"]["roles[]"] == {"from": "$group", "item": {"title": "title"}}
    pairs = fixture_pairs()
    art = pycanonic.compile_spec(spec, derive_schema([p.default for p in pairs]))
    for p in pairs:
        raw = json.dumps(p.provider).encode()
        want = {**p.default, "status": None}
        for got in (art.map(raw), art.map(p.provider), json.loads(art.map_json(raw))):
            assert same(got, want), (p.name, got)
    rows = {r["field"]: r for r in json.loads((tmp_path / "report.json").read_text())["fields"]}
    assert (rows["status"]["tier"], rows["status"]["kind"]) == ("manual", "disagree")
    assert {k: rows[k]["tier"] for k in ("id", "name", "email", "joined", "hq.city", "tags[]")} == dict.fromkeys(("id", "name", "email", "joined", "hq.city", "tags[]"), "auto")
    assert {k: rows[k]["kind"] for k in ("id", "joined", "name", "hq", "jobs[]")} == {"id": "cast", "joined": "format_date", "name": "join", "hq": "presence", "jobs[]": "group"}
    assert "def " not in (tmp_path / "t.py").read_text()  # disagree is a decision, not code


def test_array_root_field_key() -> None:
    """an array root is the field key `[]`."""
    pairs = [Pair(f"p{i}.json", [{"name": f"n{i}{k}"} for k in range(i % 3 + 1)], {"results": [{"n": f"n{i}{k}"} for k in range(i % 3 + 1)]}, None) for i in range(6)]
    result = fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e"))
    assert result.spec is not None and result.spec["fields"] == {"[]": {"from": "results", "item": {"name": "n"}}}
    assert [(r.field, r.tier) for r in result.fields] == [("[]", "auto"), ("[].name", "auto")]


def test_reject_failover() -> None:
    """p0 lacks full_name, the reject fails it over, and no field counts it."""
    pairs = [Pair(f"p{i}.json", {"name": f"n{i}", "email": f"e{i}@x.test"}, {**({"full_name": f"n{i}"} if i else {}), "mail": f"e{i}@x.test"}, None) for i in range(30)]
    result = fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e"))
    assert result.spec is not None and result.spec["reject"] == [{"missing": "full_name"}]
    rows = {r.field: r for r in result.fields}
    assert (rows["reject[0]"].tier, rows["reject[0]"].support, rows["reject[0]"].total) == ("review", 1, 30)
    assert "rejects p0.json, where the default answered" in rows["reject[0]"].note
    assert [(rows[k].tier, rows[k].support, rows[k].total) for k in ("name", "email")] == [("auto", 29, 29)] * 2
    art = pycanonic.compile_spec(result.spec, derive_schema([p.default for p in pairs]))
    with pytest.raises(pycanonic.Rejected):
        art.map(pairs[0].provider)
    assert all(same(art.map(p.provider), p.default) for p in pairs[1:])


def test_open_map_fitted_as_path() -> None:
    """an x-open node maps to the provider object that equals it."""
    attrs = [{f"k{i}{j}": j for j in range(3)} for i in range(10)]
    pairs = [Pair(f"p{i}.json", {"name": f"n{i}", "attrs": a}, {"n": f"n{i}", "meta": {"extra": a, "id": i}}, None) for i, a in enumerate(attrs)]
    schema = derive_schema([p.default for p in pairs])
    assert schema["properties"]["attrs"] == {"type": "object", "x-open": True}
    result = fit_pairs(pairs, schema, FitOptions("acme", "e"))
    row = {r.field: r for r in result.fields}["attrs"]
    assert (row.kind, row.tier, row.rule, row.support, row.holdout) == ("path", "auto", "meta.extra", 10, 1.0)
    assert result.spec is not None and result.spec["fields"]["attrs"] == "meta.extra"
    none = fit_pairs([replace(p, provider={"n": p.provider["n"]}) for p in pairs], schema, FitOptions("acme", "e"))
    assert [(r.kind, r.tier) for r in none.fields if r.field == "attrs"] == [("none", "manual")]


def test_stub_escapes_quotes_and_backslashes(tmp_path: Path) -> None:
    """provider text with triple quotes and backslashes still gives a module that imports."""
    pairs = [replace(p, provider={**p.provider, "note": 'say """hi""" \\ back\\slash \n "end"'}) for p in handle_pairs()]
    result = fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e"))
    (tmp_path / "stubs2.py").write_text(result.transforms)
    module = importlib.util.module_from_spec(spec := importlib.util.spec_from_file_location("stubs2", tmp_path / "stubs2.py"))  # type: ignore[arg-type]
    assert spec is not None and spec.loader is not None
    spec.loader.exec_module(module)
    tests = doctest.DocTestFinder().find(module.handle)
    assert len(tests[0].examples) == 3 and tests[0].examples[0].source.startswith("handle({")
    runner = doctest.DocTestRunner()
    for t in tests:
        t.globs["handle"] = lambda body: f"{body['person']['last'].lower()}.{body['person']['first'].lower()}"
        runner.run(t, out=lambda _: None)
    assert (runner.failures, runner.tries) == (0, 3)  # the examples read back exactly


def test_report_suffix_appended(tmp_path: Path) -> None:
    cli(tmp_path, "--report", str(tmp_path / "report.v2"))
    assert (tmp_path / "report.v2.md").exists() and (tmp_path / "report.v2.json").exists()


# --- fixes from the fit_eval gate ---


def test_one_default_value_never_auto() -> None:
    """A default that sends one value in every pair, or none, proves no rule: a decoy path that is
    constant in the samples (billing.region) reproduces it as well as the true one (fit_eval found
    region and nickname wrong on fresh pairs)."""
    pairs = [
        Pair(f"p{i}.json", {"name": f"n{i}", "region": "na", "nick": None, "jobs": [{"t": f"t{i}", "type": "full"}]},
             {"n": f"n{i}", "billing": {"region": "na"}, "regionCode": "na", "nickName": None, "jobs": [{"t": f"t{i}", "type": "full"}]}, None)
        for i in range(10)
    ]
    rows = {r.field: r for r in fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e")).fields}
    assert (rows["region"].kind, rows["region"].tier) == ("path", "review")
    assert "one value" in rows["region"].note
    assert (rows["nick"].kind, rows["nick"].tier) == ("empty", "review")
    assert (rows["jobs[].type"].kind, rows["jobs[].type"].tier) == ("path", "review")
    assert rows["name"].tier == rows["jobs[].t"].tier == rows["jobs[]"].tier == "auto"
    assert tier(replace(lf("path", 10, 10), one_value=True)) == "review"


def test_object_never_empty_gets_if_empty_for_review() -> None:
    """An object that the default sent in every pair has no empty form in the samples: fit writes
    `if_empty: null` for review, so a provider with no company gives null, not an object of
    nulls (fit_eval found every company leaf wrong on a fresh pair)."""
    pairs = [Pair(f"p{i}.json", {"name": f"n{i}", "org": {"name": f"o{i}", "size": i}}, {"n": f"n{i}", "employer": {"orgName": f"o{i}", "count": i}}, None) for i in range(10)]
    schema = derive_schema([p.default for p in pairs])
    result = fit_pairs(pairs, schema, FitOptions("acme", "e"))
    assert result.spec is not None and result.spec["fields"]["org"] == {"if_empty": "null"}
    rows = {r.field: r for r in result.fields}
    assert (rows["org"].kind, rows["org"].tier, rows["org.name"].tier) == ("if_empty", "review", "auto")
    assert rows["org"].note == "The default sent this object in all 10 pairs, never empty, so its empty form is a guess."
    art = pycanonic.compile_spec(result.spec, schema)
    assert art.map({"n": "x", "employer": None}) == {"name": "x", "org": None}


def reject_pairs() -> list[Pair]:
    return [Pair(f"p{i}.json", {"name": f"n{i}", "email": f"e{i}@x.test"}, {**({"full_name": f"n{i}"} if i else {}), "mail": f"e{i}@x.test"}, None) for i in range(30)]


def test_reject_runtime_rejects_more_goes_manual(monkeypatch: pytest.MonkeyPatch) -> None:
    """fit's own check says no pair is rejected, the runtime rejects p0: the reject is manual, not an
    uncaught Rejected."""
    monkeypatch.setattr(fit_mod, "_rejects", lambda cond, body: False)
    result = fit_pairs(reject_pairs(), derive_schema([p.default for p in reject_pairs()]), FitOptions("acme", "e"))
    assert result.spec is not None and "reject" not in result.spec
    row = {r.field: r for r in result.fields}["reject[0]"]
    assert (row.tier, row.kind, row.rule) == ("manual", "reject", {"missing": "full_name"})
    assert "p0.json" in row.note


def test_reject_runtime_rejects_less_goes_manual(monkeypatch: pytest.MonkeyPatch) -> None:
    """fit's own check says p1 is rejected too, the runtime does not reject it: manual, not a RuntimeError."""
    monkeypatch.setattr(fit_mod, "_rejects", lambda cond, body: "full_name" not in body or body["mail"] == "e1@x.test")
    result = fit_pairs(reject_pairs(), derive_schema([p.default for p in reject_pairs()]), FitOptions("acme", "e"))
    assert result.spec is not None and "reject" not in result.spec
    row = {r.field: r for r in result.fields}["reject[0]"]
    assert (row.tier, row.kind) == ("manual", "reject")
    assert "p1.json" in row.note
    rows = {r.field: r for r in result.fields}
    # the failover pairs come back once the reject is dropped (R32): p0 has no full_name, so name misses it
    assert [(rows[k].support, rows[k].total) for k in ("name", "email")] == [(29, 30), (30, 30)]


def test_case_drift_sets_case_insensitive_keys() -> None:
    """Keys that drift in case across the samples set `"keys": "case_insensitive"`, a review row,
    so a casing that no sample showed still maps (fit_eval found drovo fields null on fresh pairs)."""
    style = lambda k, i: k if i % 3 else k[0].upper() + k[1:]
    pairs = [Pair(f"p{i}.json", {"name": f"n{i}", "score": i}, {style("fullName", i): f"n{i}", "rating": i}, None) for i in range(10)]
    schema = derive_schema([p.default for p in pairs])
    result = fit_pairs(pairs, schema, FitOptions("acme", "e"))
    assert result.spec is not None and result.spec["keys"] == "case_insensitive"
    rows = {r.field: r for r in result.fields}
    assert (rows["spec.keys"].kind, rows["spec.keys"].tier, rows["spec.keys"].rule) == ("keys", "review", "case_insensitive")
    art = pycanonic.compile_spec(result.spec, schema)
    assert art.map({"FULLNAME": "x", "Rating": 7}) == {"name": "x", "score": 7}
    plain = fit_pairs([replace(p, provider={"fullName": p.default["name"], "rating": p.default["score"]}) for p in pairs], schema, FitOptions("acme", "e"))
    assert plain.spec is not None and "keys" not in plain.spec and "spec.keys" not in {r.field for r in plain.fields}


def test_fit_survives_nan_in_a_pair(tmp_path: Path) -> None:
    """R42: a provider body with NaN (json.loads accepts it) and a kept sort_by over it never panics."""
    import math
    import shutil

    shutil.copytree(FIXTURE, tmp_path / "fx")
    for f in (tmp_path / "fx" / "provider").iterdir():
        body = json.loads(f.read_text())
        body["scores"] = [{"v": math.nan if i % 3 == 0 else (i * 7) % 13} for i in range(60)]
        f.write_text(json.dumps(body))
    (tmp_path / "keep.json").write_text(json.dumps({"tags[]": {"from": "scores", "sort_by": "v", "pluck": "v"}}))
    args = ["fit", "--default", str(tmp_path / "fx" / "default"), "--provider", str(tmp_path / "fx" / "provider"), "--provider-name", "globex", "--endpoint", "person.enrich"]
    code = main([*args, "--out", str(tmp_path / "spec.json"), "--report", str(tmp_path / "report"), "--keep", str(tmp_path / "keep.json")])
    assert code in (1, 2)
    assert json.loads((tmp_path / "spec.json").read_text())["fields"]["tags[]"] == {"from": "scores", "sort_by": "v", "pluck": "v"}


def test_report_names_mapped_and_raw_provider_values() -> None:
    """R44: with a rule the example shows the mapped output; without one, the provider's raw value."""
    md = report.to_md(fit_fixture())
    jobs = md.split("## jobs[]: review, group")[1].split("\n## ")[0]
    assert ", mapped `" in jobs and ", provider `" not in jobs
    assert '- p03.json: default `"retired"`, provider `"inactive"`' in md
    codes = [f"Q{i}X" for i in range(10)]
    pairs = [Pair(f"p{i}.json", {"name": f"n{i}", "code": c.lower() if i < 7 else f"z{i}"}, {"n": f"n{i}", "c": c}, None) for i, c in enumerate(codes)]
    result = fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e"))
    row = next(r for r in result.fields if r.field == "code")
    assert (row.kind, row.rule) == ("disagree", None)
    assert row.examples[0] == {"pair": "p7.json", "want": "z7", "got": "Q7X"}  # raw, not lower-cased


def test_keep_key_that_matches_no_field_warns(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    (tmp_path / "keep.json").write_text(json.dumps({"status": "state", "x": "y", "hq.town": "office.town"}))
    cli(tmp_path, "--keep", str(tmp_path / "keep.json"))
    err = capsys.readouterr().err
    assert 'fit: --keep key "x" matches no field' in err and 'fit: --keep key "hq.town" matches no field' in err
    assert 'key "status"' not in err


def test_unmeasured_holdout_prints_n_a(tmp_path: Path) -> None:
    (tmp_path / "keep.json").write_text(json.dumps({"status": "state"}))
    cli(tmp_path, "--keep", str(tmp_path / "keep.json"))
    md = (tmp_path / "report.md").read_text()
    for head in ("## status: review, kept", "## hq: review, presence"):
        assert "- evidence: " in md.split(head)[1] and "holdout n/a" in md.split(head)[1].split("\n## ")[0], head
    rows = {r["field"]: r for r in json.loads((tmp_path / "report.json").read_text())["fields"]}
    assert rows["status"]["holdout"] is None and rows["hq"]["holdout"] is None
    assert rows["email"]["holdout"] == 1.0
    pairs = reject_pairs()
    reject = [r for r in fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e")).fields if r.kind in ("reject", "if_empty")]
    assert reject and all(r.holdout is None for r in reject)


@pytest.mark.parametrize("old", ["[1, 2]", '"text"', "null"])
def test_old_out_not_an_object_warns(tmp_path: Path, capsys: pytest.CaptureFixture[str], old: str) -> None:
    (tmp_path / "spec.json").write_text(old)
    assert cli(tmp_path) == 2
    err = capsys.readouterr().err
    assert "is not a JSON object, so changed rules are not compared" in err
    assert json.loads((tmp_path / "spec.json").read_text())["language"] == 2


# --- tolerance (R64) ---


def test_match() -> None:
    assert match(1804869, 1804870, Tolerance(num=0.01)) == "tolerated"
    assert match(1804869, 1804870, Tolerance()) == "miss"
    assert match(5, 5, Tolerance()) == "exact"
    assert match(True, 1, Tolerance(num=5)) == "miss"
    assert match(100, 150, Tolerance(num=0.01)) == "miss"
    a, b = "https://x.example/p.png?sig=1#top", "https://x.example/p.png?sig=2#top"
    assert match(a, b, Tolerance(url_query=True)) == "tolerated"
    assert match(a, b, Tolerance()) == "miss"
    assert match(a, a.replace("#top", "#end"), Tolerance(url_query=True)) == "miss"  # the fragment still counts
    assert match("a?x", "a?y", Tolerance(url_query=True)) == "miss"  # not a scheme:// URL
    assert match("a  b ", "a b", Tolerance(space=True)) == "tolerated"
    assert match("a  b ", "a b", Tolerance()) == "miss"
    t = Tolerance(num=0.01, space=True)
    assert match({"n": [1000, "a  b"]}, {"n": [1001, "a b"]}, t) == "tolerated"
    assert match({"n": [1000, "a"]}, {"n": [1001, "b"]}, t) == "miss"
    assert match({"n": 1}, {"m": 1}, t) == "miss"


def drifting(tol: Tolerance) -> Any:
    rows = [({"followers": 1804870 + i}, {"stats": {"fans": 1804869 + i}}) for i in range(6)]
    pairs = [Pair(f"p{i}.json", d, p, None) for i, (d, p) in enumerate(rows)]
    return fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("globex", "e", tolerance=tol))


def test_fit_tolerated_field_is_review() -> None:
    row = drifting(Tolerance(num=0.01)).fields[0]
    assert (row.tier, row.rule, row.support, row.total) == ("review", "stats.fans", 6, 6)
    assert "matches within --num-tolerance 0.01" in row.risk()
    assert drifting(Tolerance()).fields[0].rule is None  # strict: no rule reproduces a drifting count


def test_fit_tolerated_item_leaf_is_review() -> None:
    def run(tol: Tolerance) -> Any:
        rows = [
            ({"xs": [{"id": f"i{i}{k}", "n": 1000 * (i + 1) + k} for k in range(2)]}, {"ys": [{"key": f"i{i}{k}", "c": 1000 * (i + 1) + k + 1} for k in range(2)]})
            for i in range(6)
        ]
        pairs = [Pair(f"p{i}.json", d, p, None) for i, (d, p) in enumerate(rows)]
        return fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("globex", "e", tolerance=tol))

    row = next(r for r in run(Tolerance(num=0.01)).fields if r.field == "xs[].n")
    assert row.tier == "review" and row.support == row.total == 12
    assert "matches within --num-tolerance 0.01" in row.risk()
    assert next(r for r in run(Tolerance()).fields if r.field == "xs[].n").tier == "manual"


# --- format, split_name modes, chance lookups, near matches (R53, R58, R67, R68) ---


def _fit_leaf_rows(key: str, rows: list[tuple[Any, Any]]) -> dict[str, Row]:
    return _fit_rows(key, S, [({key: t}, b) for t, b in rows])


def test_fit_proposes_format() -> None:
    rows = [({"id": f"urn:example:item:{100 + i}"}["id"], {"ref": 100 + i}) for i in range(6)]
    f = leaf("id", rows)
    assert f.kind == "format" and (f.support, f.total) == (6, 6)
    assert f.rule == {"from": "ref", "transform": [{"format": "urn:example:item:{}"}]}
    assert _fit_leaf_rows("id", rows)["id"].tier == "auto"
    assert leaf("id", rows[:1]).kind != "format"  # one pair: the prefix is that pair's text


def test_fit_split_first_word() -> None:
    names = [("Mary", "Ann Smith"), ("Jo", "Lee Park"), ("Ada", "May Brown"), ("Sam", "Rae Stone"), ("Ivy", "Kay Reed"), ("Tom", "Joy Hale")]
    rows = [(rest, {"full": f"{a} {rest}"}) for a, rest in names]
    f = leaf("last_name", rows)
    assert f.rule == {"from": "full", "transform": [{"split_name": {"part": "last", "mode": "first_word"}}]}
    assert f.kind == "split_name"
    assert _fit_leaf_rows("last_name", rows)["last_name"].tier == "review"


def test_no_number_to_category_lookup() -> None:
    sizes = [(1, "small"), (2, "small"), (3, "large"), (4, "large")] * 3
    rows = [(t, {"size_min": n}) for n, t in sizes]
    assert leaf("size", rows).kind == "none"
    assert leaf("size", [(t, {"size_min": str(n)}) for n, t in sizes]).kind == "lookup"  # a text source still can


def test_weak_lookup_label() -> None:
    weak = [(t, {"kind": s}) for s, t in [("STAR", "starred"), ("UP", "liked")] * 4]
    assert "weak, possibly by chance" in _fit_leaf_rows("reaction", weak)["reaction"].risk()
    pairs = [("STAR", "starred"), ("FAV", "starred"), ("UP", "liked"), ("LIKE", "liked")] * 2
    strong = _fit_leaf_rows("reaction", [(t, {"kind": s}) for s, t in pairs])["reaction"]
    assert strong.kind == "lookup" and "weak" not in strong.risk()


def test_trailing_space_is_review() -> None:
    long = "A description that runs on for a while"
    rows = [(f"Item name number {i}", {"name": f"Item name number {i}"}) for i in range(8)]
    rows += [("Item name number 8 ", {"name": "Item name number 8"}), ("Item name number 9\t", {"name": "Item name number 9"}), (long + " and then more", {"name": long})]
    row = _fit_leaf_rows("name", rows)["name"]
    assert (row.tier, row.rule, row.support, row.total) == ("review", "name", 11, 11)
    assert "near match: white space or text cut short in 3 pairs" in row.risk()
    short = [("Item name number %d" % i, {"name": "Item name number %d" % i}) for i in range(8)] + [(f"short text {i} and more", {"name": f"short text {i}"}) for i in range(3)]
    assert _fit_leaf_rows("name", short)["name"].rule is None  # a cut under 20 characters is no near match


def _crew_pairs() -> list[Pair]:
    """Crew lists whose item leaf `vessel.ref` the provider cannot give (its `hull` code has other
    digits), and one item leaves out: the schema says `x-empty: "missing"` for it."""
    pairs = []
    for i in range(8):
        crew = [{"vessel": {"ref": 100 + i * 10 + k, "label": f"v{i}{k}"}, "role": f"r{i}{k}"} for k in range(2)]
        if i == 0:
            del crew[1]["vessel"]["ref"]
        hands = [{"boat": f"v{i}{k}", "job": f"r{i}{k}", "hull": f"H-{900 - i * 10 - k}"} for k in range(2)]
        pairs.append(Pair(f"p{i}.json", {"name": f"n{i}", "crew": crew, "flags": [f"f{i}"] if i % 2 else []}, {"n": f"n{i}", "hands": hands}, None))
    return pairs


def test_fresh_draft_has_full_shape() -> None:
    """A manual item leaf gets its schema empty form in the draft and reads "placeholder", and verify
    on the fresh draft reports no missing key."""
    from pycanonic.cli.fit import verify

    pairs = _crew_pairs()
    schema = derive_schema([p.default for p in pairs])
    assert schema["properties"]["crew"]["items"]["properties"]["vessel"]["properties"]["ref"]["x-empty"] == "missing"
    result = fit_pairs(pairs, schema, FitOptions("acme", "e"))
    rows = {r.field: r for r in result.fields}
    assert rows["crew[].vessel.ref"].tier == "manual" and "placeholder" in rows["crew[].vessel.ref"].risk()
    assert result.spec is not None and result.spec["fields"]["crew[]"]["item"]["vessel.ref"] == {"const": None}
    v = verify.run(result.spec, schema, pairs, 0.95, None)
    assert result.spec["fields"]["flags[]"] == {"const": []}  # x-empty "[]": an empty list, not null
    assert v.shape_failures == []
    assert "crew[].vessel.ref" not in v.fields and len(v.unmapped["crew[].vessel.ref"]) == 8  # listed, as with no rule (R43)


def test_if_empty_note_once() -> None:
    pairs = [Pair(f"p{i}.json", {"name": f"n{i}", "org": {"name": f"o{i}"}}, {"n": f"n{i}", "employer": {"orgName": f"o{i}"}}, None) for i in range(10)]
    rows = {r.field: r for r in fit_pairs(pairs, derive_schema([p.default for p in pairs]), FitOptions("acme", "e")).fields}
    assert rows["org"].risk().count("its empty form is a guess") == 1


def test_fit_union_manual() -> None:
    """A union node is one manual row and a placeholder, no leaf rows below it."""
    from pycanonic.cli.fit import verify

    authors = [{"kind": "person", "name": f"p{i}"} for i in range(3)] + [{"kind": "org", "name": f"o{i}", "size": i} for i in range(3)]
    pairs = [Pair(f"p{i}.json", {"id": f"x{i}", "author": a}, {"ref": f"x{i}", "by": a}, None) for i, a in enumerate(authors)]
    schema = derive_schema([p.default for p in pairs])
    assert "oneOf" in schema["properties"]["author"]
    result = fit_pairs(pairs, schema, FitOptions("acme", "e"))
    rows = {r.field: r for r in result.fields}
    assert rows["author"].tier == "manual" and "one_of" in rows["author"].risk() and "placeholder" in rows["author"].risk()
    assert not any(k.startswith("author.") for k in rows)
    assert result.spec is not None and result.spec["fields"]["author"] == {"const": None}
    pycanonic.compile_spec(result.spec, schema)
    failures = verify.run(result.spec, schema, pairs, 0.95, None).shape_failures
    assert failures and {m for _, _, m in failures} == {"null against object"}  # no missing key; the null stays until the branches are written
