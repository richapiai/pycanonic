"""Leave-one-provider-out, ported from the TypeScript CLI's test/eval/eval.test.ts: each provider is
inferred with a synonym dictionary learned only from the other providers' approved specs, then scored on
40 responses it never saw. A field is correct when the draft spec and the reviewed spec give the same
value on every one of them."""

from __future__ import annotations

import json
from dataclasses import dataclass
from functools import cache
from pathlib import Path
from typing import Any

import pytest
from providers import PROVIDERS, Provider, person
from unseen import UNSEEN

import pycanonic
from pycanonic.cli.infer.infer import infer_draft
from pycanonic.cli.infer.learn import learn_synonyms
from pycanonic.cli.io import js_json

FIXTURES = Path(__file__).resolve().parents[3] / "crates/canonic-core/tests/fixtures"
# bench/person.js `canonical`
CANONICAL: dict[str, Any] = json.loads((FIXTURES / "specs/canonical/bench.json").read_text())

ROUTING = ["email", "phone", "company.domain"]
FIELDS = ["first_name", "last_name", "full_name", "email", "phone", "title", "seniority", "linkedin_url", "location.city", "location.country",
          "company.name", "company.domain", "company.headcount", "company.industry", "company.founded", "experiences", "skills", "updated_at"]
CONFIDENT = 0.6


def get(o: Any, path: str) -> Any:
    for k in path.split("."):
        o = o.get(k) if isinstance(o, dict) else None
    return o


def spec(fields: dict[str, Any]) -> dict[str, Any]:
    return {"provider": "eval", "endpoint": "person.enrich", "version": 1, "status": "draft", "fields": fields}


@dataclass
class Row:
    provider: str
    fields: int
    correct: int
    routing: str
    confident_wrong: list[str]
    wrong: list[str]


def evaluate(p: Provider, others: list[Provider]) -> Row:
    """Scores provider p with synonyms learned from the others."""
    synonyms = learn_synonyms([o.truth for o in others], CANONICAL)
    train = [p.payload(person(n), n) for n in range(12)]
    held = [p.payload(person(100 + n), 100 + n) for n in range(40)]
    r = infer_draft(CANONICAL, train, provider="eval", endpoint="person.enrich", synonyms=synonyms, raws=[js_json(t) for t in train])
    truth = pycanonic.compile_spec(spec(p.truth), CANONICAL)
    draft = pycanonic.compile_spec(r.spec, CANONICAL) if r.spec["fields"] else None
    confidence = {f.field.removesuffix("[]"): f.confidence for f in r.fields}
    want = [truth.map(x) for x in held]
    got = [draft.map(x) if draft else None for x in held]
    scored = [f for f in FIELDS if any(get(t, f) is not None for t in want) or f in confidence]
    wrong = [f for f in scored if any(js_json(get(t, f)) != js_json(get(d, f)) for t, d in zip(want, got))]
    routing_ok = len([f for f in ROUTING if f not in wrong])
    return Row(
        provider=p.name,
        fields=len(scored),
        correct=len(scored) - len(wrong),
        routing=f"{routing_ok}/{len(ROUTING)}",
        confident_wrong=[f for f in wrong if (confidence.get(f) or 0) >= CONFIDENT],
        wrong=wrong,
    )


def table(rows: list[Row]) -> str:
    head = "provider               accuracy  routing  wrong"
    return "\n".join([head, *(f"{r.provider:<23}{f'{r.correct}/{r.fields}':<10}{r.routing:<9}{', '.join(r.wrong)}" for r in rows)])


@cache
def rows() -> tuple[Row, ...]:
    return tuple(evaluate(p, [o for o in PROVIDERS if o is not p]) for p in PROVIDERS)


def test_unseen_providers_reported_only() -> None:
    every = [*PROVIDERS, *UNSEEN]
    print(table([evaluate(p, [o for o in every if o is not p]) for p in UNSEEN]))


@pytest.mark.parametrize("i", range(len(PROVIDERS)), ids=[p.name for p in PROVIDERS])
def test_leave_one_provider_out(i: int) -> None:
    """Routing 100%, no confident-wrong fields, 95% overall per provider."""
    r = rows()[i]
    assert r.routing == "3/3", r
    assert r.confident_wrong == [], r
    assert r.correct / r.fields >= 0.95, r


def test_table() -> None:
    print(table(list(rows())))
    assert sum(r.correct for r in rows()) / sum(r.fields for r in rows()) >= 0.95
