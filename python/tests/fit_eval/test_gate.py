"""The release gate for `canonic fit`: zero wrong auto fields on fresh pairs."""

from __future__ import annotations

import pytest

from .providers import PROVIDERS
from .report import all_outcomes, evaluate, table
from .world import entities


def test_world_has_the_traps() -> None:
    """The fresh pairs show what the training pairs do not: rare enum values, values that only
    looked constant, a value that was never sent, a section that was never empty, and a repeat
    employer after another job."""
    train, fresh = entities(1, 15), entities(2, 60)
    rare = lambda es, k: {e[k] for e in es}
    assert rare(fresh, "seniority") - rare(train, "seniority")
    assert rare(fresh, "status") - rare(train, "status")
    assert (rare(train, "region"), rare(train, "plan")) == ({"na"}, {"free"})
    assert len(rare(fresh, "region")) > 1 and len(rare(fresh, "plan")) > 1
    assert rare(train, "nickname") == {None} != rare(fresh, "nickname")
    assert all(e["company"] for e in train) and not all(e["company"] for e in fresh)
    industry = lambda es: {e["company"]["industry"] for e in es if e["company"]}
    assert industry(fresh) - industry(train)
    assert rare(fresh, "currency") - rare(train, "currency")
    assert any(b["start"] != a["end"] for e in fresh for a, b in zip(e["jobs"], e["jobs"][1:]))
    ids = [[j["company_id"] for j in e["jobs"]] for e in fresh]
    assert any(any(a == c != b for a, b, c in zip(x, x[1:], x[2:])) for x in ids)
    assert len(PROVIDERS) >= 12


@pytest.mark.parametrize("name", [p.name for p in PROVIDERS])
def test_zero_wrong_auto(name: str) -> None:
    o = evaluate(name)
    lines = [f"{o.provider} {w.field} ({w.kind}, {w.rule}): {w.pair} default {w.want!r}, mapped {w.got!r}" for w in o.wrong]
    assert not lines, "wrong auto fields:\n" + "\n".join(lines)


@pytest.mark.parametrize("name", [p.name for p in PROVIDERS])
def test_verify_no_shape_failure_on_auto(name: str) -> None:
    """`canonic verify` over the 60 fresh pairs finds no shape failure at, above or below an auto field."""
    o = evaluate(name)
    assert not o.shape, "shape failures on auto fields:\n" + "\n".join(f"{o.provider} {f}: {p} {at} {m}" for f, p, at, m in o.shape[:20])


def test_auto_share_reported(capsys: pytest.CaptureFixture[str]) -> None:
    text = table(all_outcomes())
    with capsys.disabled():
        print("\n" + text)
