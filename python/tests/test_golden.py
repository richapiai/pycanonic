"""0.1.1 plans and outputs (fixtures/golden-0.1.1, written by the 0.1.1 build): a loaded plan and a
fresh compile of the case spec must both map to the same bytes. See conformance/cases*/README.md."""

import json
import sys
from pathlib import Path

import pytest

import pycanonic

CONFORMANCE = Path(__file__).resolve().parents[2] / "conformance"
GOLDEN = Path(__file__).parent / "fixtures" / "golden-0.1.1"
sys.path.insert(0, str(CONFORMANCE))
from compare import fffd  # noqa: E402

PLANS = sorted(GOLDEN.glob("v*/*.plan.json"))


def _inputs(corpus, case):
    if corpus == "v1":
        return [(fffd(s).encode(), None) for s in case["inputs"]]
    return [((i["raw"] if "raw" in i else json.dumps(i["body"])).encode(), i.get("context")) for i in case["inputs"]]


def _run(a, raw, ctx):
    try:
        return a.map_json(raw, context=ctx).decode()
    except pycanonic.Rejected as e:
        return {"$rejected": e.reason}


def test_there_are_fixtures():
    assert PLANS


@pytest.mark.parametrize("plan", PLANS, ids=lambda p: f"{p.parent.name}/{p.name[:-len('.plan.json')]}")
def test_golden_0_1_1(plan):
    corpus, name = plan.parent.name, plan.name[: -len(".plan.json")]
    src = CONFORMANCE / ("cases" if corpus == "v1" else "cases-v2") / (name.replace("__", "/") + ".json")
    case = json.loads(src.read_text(encoding="utf-8"))
    want = json.loads(plan.with_name(name + ".out.json").read_text(encoding="utf-8"))
    ins = _inputs(corpus, case)
    compiled = pycanonic.compile_spec(case["spec"], case["canonical" if corpus == "v1" else "schema"])
    for a in (pycanonic.load(plan), compiled):
        assert [_run(a, raw, ctx) for raw, ctx in ins] == want
