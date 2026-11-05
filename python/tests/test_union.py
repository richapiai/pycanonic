"""`oneOf` union nodes and `one_of` (R55). The conformance cases in conformance/cases-v2/union hold
the outputs. These tests pin what a case cannot: the plan round trip, stats names and the view."""

import json
from pathlib import Path

import pycanonic

UNION = Path(__file__).resolve().parents[2] / "conformance" / "cases-v2" / "union"


def _case(name):
    return json.loads((UNION / f"{name}.json").read_text())


def _ways(a, body):
    raw = json.dumps(body).encode()
    return [lambda: a.map(raw), lambda: a.map(body), lambda: json.loads(a.map_json(raw))]


def test_union_plan_saves_loads_and_maps(tmp_path):
    for name in ["person-or-org", "item", "x-empty-object"]:
        case = _case(name)
        a = pycanonic.compile_spec(case["spec"], case["schema"])
        assert json.loads(a.plan_json())["format"] == 2
        saved = tmp_path / f"{name}.plan.json"
        saved.write_text(a.plan_json())
        b = pycanonic.load(saved)
        assert b.plan_json() == a.plan_json()
        for inp in case["inputs"]:
            for way in _ways(b, inp["body"]):
                assert way() == inp["expected"]


def test_union_stats_are_named_per_branch():
    case = _case("person-or-org")
    a = pycanonic.compile_spec(case["spec"], case["schema"])
    person = case["inputs"][0]["body"]
    for way in _ways(a, person):
        way()
    fields = a.stats_snapshot()["fields"]
    assert list(fields) == [
        "title",
        "author(person).kind",
        "author(person).last",
        "author(person).first",
        "author(person).id",
        "author(org).id",
        "author(org).name",
        "author(org).followers",
        "author(org).kind",
    ]
    assert fields["author(person).first"]["fill"] == 3
    # A branch that is not written counts a miss for each of its fields.
    assert fields["author(org).name"]["fill"] == 0


def test_union_stats_inside_items_and_source():
    case = _case("item")
    a = pycanonic.compile_spec(case["spec"], case["schema"])
    a.map(case["inputs"][0]["body"])
    fields = a.stats_snapshot()["fields"]
    assert "posts[].author(person).first" in fields
    assert fields["posts[].author(org).followers"]["fill"] == 1
    assert "posts[].author(org): when" in a.source



def test_validate_canonical_reports_union_errors():
    bad = _case("error-schema-one-branch")["schema"]
    assert pycanonic.validate_canonical(bad) == [
        pycanonic.Issue("canonical schema", "author: oneOf needs two or more branches")
    ]
    # validate_canonical checks as language 1, which has no unions.
    good = _case("person-or-org")["schema"]
    assert pycanonic.validate_canonical(good) == [
        pycanonic.Issue("canonical schema", "author: oneOf needs language 2")
    ]
