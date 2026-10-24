"""ShadowDiff and merge_gate through Python: shadow.test.ts on dicts, and a soak of the bench bodies."""

from __future__ import annotations

import dataclasses
import json
from collections.abc import Mapping
from typing import Any

import pytest

from pycanonic import (
    Artifact,
    FieldAlert,
    FillMonitor,
    GateResult,
    ShadowDiff,
    compile_spec,
    merge_gate,
)

CANONICAL = {
    "type": "object",
    "properties": {
        "email": {"type": "string", "semantic": "email", "routing": True},
        "title": {"type": "string"},
        "patent_count": {"type": "integer"},
        "company": {"type": "object", "properties": {"name": {"type": "string"}}},
        "skills": {"type": "array", "items": {"type": "string"}},
    },
}
BASE: dict[str, Any] = {
    "email": "workEmail",
    "title": "role",
    "patent_count": {"from": "patents", "transform": {"cast": "integer"}},
    "company.name": "org.name",
    "skills[]": {"from": "tags", "pluck": "label"},
}


def compile_fields(fields: dict[str, Any], version: int) -> Artifact:
    status = "promoted" if version == 1 else "shadow"
    spec = {"provider": "acme", "endpoint": "person.enrich", "version": version, "status": status, "fields": fields}
    return compile_spec(spec, CANONICAL)


PROMOTED = compile_fields(BASE, 1)


def bodies(shape: Any = lambda i: {}) -> list[dict[str, Any]]:
    """1,000 responses. One in 200 has a patent count, so that field is rarely filled."""
    return [
        {
            "role": f"Role {i}",
            "dept": f"Dept {i % 7}",
            "org": {"name": "Acme"},
            "tags": [{"label": "sql"}],
            "patents": None if i % 200 else 3,
            **shape(i),
        }
        for i in range(1000)
    ]


def soak(candidate: Artifact, responses: list[dict[str, Any]]) -> Mapping[str, Any]:
    diff = ShadowDiff(PROMOTED.routing)
    for r in responses:
        diff.add(PROMOTED.map(r), candidate.map(r))
    return diff.snapshot()


def counts(compared: int, same: int, improvements: int, regressions: int, conflicts: int) -> dict[str, int]:
    return {
        "compared": compared,
        "same": same,
        "improvements": improvements,
        "regressions": regressions,
        "conflicts": conflicts,
    }


def test_classifies_each_field_and_skips_responses_where_both_sides_are_null() -> None:
    d = ShadowDiff(["email"])

    def row(email: str | None, title: str | None, skills: list[str] | None) -> dict[str, Any]:
        return {"email": email, "title": title, "company": {"name": "Acme"}, "skills": skills}

    d.add(row(None, "CTO", ["a"]), row("a@b.co", "CTO", ["a"]))
    d.add(row("a@b.co", "CTO", ["a"]), row(None, "VP", ["a", "b"]))
    d.add(row(None, None, None), row(None, None, None))
    snap = d.snapshot()
    assert snap == {
        "responses": 3,
        "routing": ["email"],
        "fields": {
            "email": counts(2, 0, 1, 1, 0),
            "title": counts(2, 1, 0, 0, 1),
            "company.name": counts(3, 3, 0, 0, 0),
            "skills": counts(2, 1, 0, 0, 1),
        },
    }
    assert list(snap["fields"]) == ["email", "title", "company.name", "skills"]


def test_inside_arrays_key_order_doesnt_count_and_a_null_key_equals_a_missing_one() -> None:
    d = ShadowDiff([])
    # The left side is what hand-written code returned. The right side is map() output for the same body.
    d.add(
        {"jobs": [{"title": "CTO", "org": {"name": "Acme"}}]},
        {"jobs": [{"org": {"name": "Acme", "domain": None}, "title": "CTO", "start": None}]},
    )
    d.add({"jobs": [{"title": "CTO"}]}, {"jobs": [{"title": "CTO", "org": {"name": None}}]})
    d.add({"jobs": [{"title": "CTO"}]}, {"jobs": [{"title": "CEO"}]})
    d.add({"jobs": [{"title": "CTO"}]}, {"jobs": [{"title": "CTO"}, {"title": "CTO"}]})
    assert d.snapshot()["fields"]["jobs"] == counts(4, 2, 0, 0, 2)


def test_a_list_whose_elements_only_gained_or_only_lost_values_is_an_improvement_or_a_regression() -> None:
    d = ShadowDiff([])
    before = {"jobs": [{"title": None, "org": "Acme"}, {"title": None, "org": "Globex"}]}
    after = {"jobs": [{"title": "CTO", "org": "Acme"}, {"title": "VP", "org": "Globex"}]}
    d.add(before, after)  # a renamed item key, fixed by the candidate
    d.add(after, before)  # the candidate lost it
    d.add(before, {"jobs": [{"title": "CTO", "org": "Initech"}, {"title": "VP", "org": "Globex"}]})  # gained one, changed one
    d.add(before, {"jobs": [{"title": "CTO", "org": "Acme"}]})  # a different length is never an improvement
    assert d.snapshot()["fields"]["jobs"] == counts(4, 0, 1, 1, 2)


def test_an_int_equals_the_same_float_and_a_tuple_is_a_list() -> None:
    d = ShadowDiff([])
    d.add({"n": 1, "xs": [1, 2]}, {"n": 1.0, "xs": (1, 2.0)})
    assert d.snapshot()["fields"] == {"n": counts(1, 1, 0, 0, 0), "xs": counts(1, 1, 0, 0, 0)}


def test_artifact_routing_lists_canonical_paths() -> None:
    assert PROMOTED.routing == ["email"]


def test_a_routing_field_rename_auto_merges_once_known_entities_confirm_it() -> None:
    # The provider renamed workEmail to email. Promoted maps null, and the candidate reads the new key.
    responses = bodies(lambda i: {"email": f"u{i}@acme.com"})
    candidate = compile_fields({**BASE, "email": "email"}, 2)
    report = soak(candidate, responses)
    assert report["fields"]["email"] == counts(1000, 0, 1000, 0, 0)

    assert merge_gate(report) == GateResult(
        merge=False, reasons=["email: 1000 routing improvements with no known-entity check"]
    )
    pairs = [(candidate.map(responses[i]), {"email": f"u{i}@acme.com"}) for i in range(3)]
    assert merge_gate(report, known=pairs) == GateResult(merge=True, reasons=[])
    # The TypeScript shape works too.
    as_dicts = [{"candidate": c, "expected": e} for c, e in pairs]
    assert merge_gate(report, known=as_dicts) == GateResult(merge=True, reasons=[])

    # A candidate that picks a personal address passes the validator but not the known entities.
    wrong = [({"email": "ada@gmail.com"}, {"email": "u0@acme.com"})]
    assert merge_gate(report, known=wrong) == GateResult(
        merge=False, reasons=['known entity 0: email is "ada@gmail.com", expected "u0@acme.com"']
    )


def test_a_non_routing_field_that_changes_meaning_escalates_to_a_person() -> None:
    # Re-inference maps title to the department key. Every value still fills, and every value differs.
    report = soak(compile_fields({**BASE, "title": "dept"}, 2), bodies())
    assert merge_gate(report) == GateResult(
        merge=False, reasons=["title: 0 regressions and 1000 conflicts in 1000 compared"]
    )


def test_a_candidate_that_drops_a_rarely_filled_field_doesnt_auto_merge() -> None:
    rest = {k: v for k, v in BASE.items() if k != "patent_count"}
    report = soak(compile_fields(rest, 2), bodies())
    # Five responses had a value and the candidate lost all five. Among 1,000 responses that is 0.5%.
    assert report["fields"]["patent_count"] == counts(5, 0, 0, 5, 0)
    assert merge_gate(report).reasons == ["patent_count: 5 regressions and 0 conflicts in 5 compared"]


def test_an_empty_report_never_merges_and_a_known_entity_has_to_name_the_routing_field_it_vouches_for() -> None:
    assert merge_gate(ShadowDiff(PROMOTED.routing).snapshot()) == GateResult(
        merge=False, reasons=["no responses compared"]
    )
    report = soak(compile_fields({**BASE, "email": "email"}, 2), bodies(lambda i: {"email": f"u{i}@acme.com"}))
    silent: list[tuple[dict[str, Any], dict[str, Any]]] = [({}, {}), ({"title": "x"}, {"title": "x"})]
    assert merge_gate(report, known=silent).reasons == [
        "email: 1000 routing improvements with no known-entity check"
    ]


def test_a_clean_candidate_merges_and_a_fill_alert_on_it_blocks_the_merge() -> None:
    report = soak(compile_fields(BASE, 2), bodies(lambda i: {"workEmail": "a@acme.com"}))
    assert merge_gate(report) == GateResult(merge=True, reasons=[])
    alert = FieldAlert(field="title", kind="fill", rate=0.2, baseline=0.9, responses=1000)
    expected = ["title: fill alert on the candidate (0.200 vs 0.900)"]
    assert merge_gate(report, fill_alerts=[alert]).reasons == expected
    assert merge_gate(report, fill_alerts=[dataclasses.asdict(alert)]).reasons == expected


def test_routing_regressions_and_conflicts_block_and_max_change_is_inclusive() -> None:
    report = {
        "responses": 100,
        "routing": ["email"],
        "fields": {
            "email": counts(100, 90, 0, 7, 3),
            "title": counts(100, 98, 0, 1, 1),
            "seniority": counts(100, 97, 0, 2, 1),
        },
    }
    assert merge_gate(report, max_change=0.02) == GateResult(
        merge=False,
        reasons=[
            "email: 7 routing regressions",
            "email: 3 routing conflicts",
            "seniority: 2 regressions and 1 conflicts in 100 compared",
        ],
    )


def test_known_values_are_written_as_json_and_rates_round_like_to_fixed() -> None:
    report = {"responses": 1, "routing": ["email", "company.domain"], "fields": {}}
    known = [
        (
            {"email": None, "company": {"domain": ['a"b']}},
            {"email": "x@y.co", "company": {"domain": ['a"b']}},
        )
    ]
    alert = FieldAlert(field="jobs[].title", kind="errors", rate=0.0625, baseline=1.0625, responses=400)
    assert merge_gate(report, known=known, fill_alerts=[alert]) == GateResult(
        merge=False,
        reasons=[
            'known entity 0: email is null, expected "x@y.co"',
            "jobs[].title: errors alert on the candidate (0.063 vs 1.063)",
        ],
    )


def test_gate_result_is_frozen() -> None:
    with pytest.raises(dataclasses.FrozenInstanceError):
        merge_gate(ShadowDiff([]).snapshot()).merge = True  # type: ignore[misc]


def test_a_cyclic_or_oversized_object_raises_and_records_nothing() -> None:
    cyclic: dict[str, Any] = {}
    cyclic["a"] = cyclic
    d = ShadowDiff([])
    with pytest.raises(ValueError, match="too large or cyclic"):
        d.add(cyclic, {})
    with pytest.raises(ValueError, match="too large or cyclic"):
        d.add({}, {"xs": [[]] * 1_000_001})
    assert d.snapshot() == {"responses": 0, "routing": [], "fields": {}}
    report = {"responses": 1, "routing": ["a"], "fields": {}}
    with pytest.raises(ValueError, match="too large or cyclic"):
        merge_gate(report, known=[(cyclic, {})])


def nested(n: int) -> Any:
    """`n` lists nested in one another around a number."""
    x: Any = 1
    for _ in range(n):
        x = [x]
    return x


def test_add_allows_512_nested_containers_as_the_parser_does() -> None:
    d = ShadowDiff([])
    d.add({"a": nested(511)}, {"a": nested(511)})
    with pytest.raises(ValueError, match="too large or cyclic"):
        d.add({"a": nested(512)}, {})
    assert d.snapshot()["responses"] == 1


def test_nested_keys_take_javascripts_order() -> None:
    d = ShadowDiff([])
    d.add({"a": {"b": 1, "2": 1}}, {"a": {"b": 1, "2": 1}})
    assert list(d.snapshot()["fields"]) == ["a.2", "a.b"]


def test_a_bench_soak_with_real_fill_alerts(bench_spec: tuple[dict[str, Any], dict[str, Any], list[str]]) -> None:
    spec, schema, raws = bench_spec
    promoted = compile_spec(spec, schema)
    # The candidate reads title from a key the bench bodies do not have.
    title = {"from": "profile.title", "transform": "trim"}
    candidate = compile_spec({**spec, "version": 2, "status": "shadow", "fields": {**spec["fields"], "title": title}}, schema)
    monitor = FillMonitor(min_responses=len(raws))
    diff = ShadowDiff(promoted.routing)
    for raw in raws:
        body = json.loads(raw)
        diff.add(promoted.map(body), candidate.map(body))
    # The promoted window is the baseline the candidate's window is judged against.
    assert monitor.check(promoted.stats.snapshot()) == []
    alerts = monitor.check(candidate.stats.snapshot())
    assert alerts == [FieldAlert(field="title", kind="fill", rate=0.0, baseline=1.0, responses=64)]

    report = diff.snapshot()
    assert report["responses"] == 64
    assert report["fields"]["title"] == counts(64, 0, 0, 64, 0)
    assert all(f["compared"] == f["same"] for p, f in report["fields"].items() if p != "title")
    assert merge_gate(report, fill_alerts=alerts) == GateResult(
        merge=False,
        reasons=[
            "title: 64 regressions and 0 conflicts in 64 compared",
            "title: fill alert on the candidate (0.000 vs 1.000)",
        ],
    )


def test_array_root_output_is_one_field() -> None:
    d = ShadowDiff([])
    d.add([{"name": "a"}], [{"name": "ZZZ"}, {"name": "b"}])
    d.add(None, [{"name": "x"}])
    report = d.snapshot()
    assert report["fields"] == {"[]": {"compared": 2, "same": 0, "improvements": 1, "regressions": 0, "conflicts": 1}}
    assert merge_gate(report).merge is False
    same = ShadowDiff([])
    same.add([{"name": "a"}], [{"name": "a"}])
    assert merge_gate(same.snapshot()) == GateResult(merge=True, reasons=[])
