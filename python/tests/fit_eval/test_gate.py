"""The release gate for `canonic fit`: zero wrong auto fields on fresh pairs."""

from __future__ import annotations

import pytest

from .providers import PROVIDERS
from .report import all_outcomes, evaluate, table
from .world import entities


def test_world_has_the_traps() -> None:
    """The fresh pairs show what the training pairs may not: rare enum values, a currency other
    than USD, a repeat employer after another job."""
    train, fresh = entities(1, 15), entities(2, 60)
    rare = lambda es, k: {e[k] for e in es}
    assert rare(fresh, "seniority") - rare(train, "seniority")
    assert rare(fresh, "currency") - {"USD"}
    ids = [[j["company_id"] for j in e["jobs"]] for e in fresh]
    assert any(any(a == c != b for a, b, c in zip(x, x[1:], x[2:])) for x in ids)
    assert len(PROVIDERS) >= 12


@pytest.mark.parametrize("name", [p.name for p in PROVIDERS])
def test_zero_wrong_auto(name: str) -> None:
    o = evaluate(name)
    lines = [f"{o.provider} {w.field} ({w.kind}, {w.rule}): {w.pair} default {w.want!r}, mapped {w.got!r}" for w in o.wrong]
    assert not lines, "wrong auto fields:\n" + "\n".join(lines)


def test_auto_share_reported(capsys: pytest.CaptureFixture[str]) -> None:
    text = table(all_outcomes())
    with capsys.disabled():
        print("\n" + text)
