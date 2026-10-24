"""`canonic fit`: the leaf search (index, transform chains, rule choice, holdout), arrays and structure."""

from __future__ import annotations

import json
from typing import Any

import pycanonic

from pycanonic.cli.fit.arrays import fit_array
from pycanonic.cli.fit.chains import Chain, apply, apply_rule, candidates
from pycanonic.cli.fit.compare import same
from pycanonic.cli.fit.index import Occurrence, scalars
from pycanonic.cli.fit.leaves import LeafFit, _related, fit_leaf
from pycanonic.cli.fit.pairs import Pair
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
