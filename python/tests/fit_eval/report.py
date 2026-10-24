"""Fits each provider on 15 pairs (seed 1) and maps 60 fresh pairs (seed 2) through the spec it
wrote. An auto field is wrong when its mapped value differs from the default's on any fresh pair.

Run as a script, it writes the auto share table to RESULTS.md next to this file."""

from __future__ import annotations

import sys
import time
from dataclasses import dataclass, field
from functools import cache
from pathlib import Path
from typing import Any

if __package__ in (None, ""):  # run as a script: make `fit_eval` importable
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    __package__ = "fit_eval"

import pycanonic
from pycanonic.cli.fit.arrays import MISSING, get
from pycanonic.cli.fit.derive import derive_schema
from pycanonic.cli.fit.fit import FitOptions, _eq, fit_pairs
from pycanonic.cli.fit.leaves import _segs
from pycanonic.cli.fit.pairs import Pair
from pycanonic.cli.fit.report import FitResult

from .providers import PROVIDERS, Provider
from .world import entities

TRAIN, FRESH = 15, 60
RESULTS = Path(__file__).with_name("RESULTS.md")


@dataclass
class Wrong:
    field: str
    kind: str
    rule: Any
    pair: str
    want: Any
    got: Any


@dataclass
class Outcome:
    provider: str
    result: FitResult
    rejected: int  # fresh pairs the spec rejects: a failover, not a wrong value
    seconds: float
    wrong: list[Wrong] = field(default_factory=list)

    def tiers(self) -> dict[str, int]:
        return self.result.summary()


def pairs(p: Provider, seed: int, n: int) -> list[Pair]:
    return [Pair(f"s{seed}-{i:02d}", p.render_default(e), p.render(e), None) for i, e in enumerate(entities(seed, n))]


def _value(doc: Any, path: str) -> Any:
    return get(doc, _segs(path)) if path else doc


def _show(v: Any) -> Any:
    return "<missing>" if v is MISSING else v


def differs(got: Any, want: Any, parts: list[str]) -> Any:
    """For a field key split on `[]` (`jobs[].title` gives `["jobs", ".title"]`), the first (got,
    want) that differ, or None. Item values compare only where both arrays have the same length:
    a length mismatch belongs to the array's own row."""
    head, rest = parts[0].lstrip("."), parts[1:]
    g, w = _value(got, head), _value(want, head)
    if rest in ([], [""]):
        return None if _eq(g, w) else (g, w)
    if not (isinstance(g, list) and isinstance(w, list) and len(g) == len(w)):
        return None
    return next((d for x, y in zip(g, w) if (d := differs(x, y, rest)) is not None), None)


@cache
def evaluate(name: str) -> Outcome:
    p = next(x for x in PROVIDERS if x.name == name)
    start = time.perf_counter()
    train = pairs(p, 1, TRAIN)
    schema = derive_schema([x.default for x in train])
    result = fit_pairs(train, schema, FitOptions(p.name, "person.enrich"))
    out = Outcome(p.name, result, 0, 0.0)
    auto = [r for r in result.fields if r.tier == "auto"]
    if result.spec is not None and auto:
        art = pycanonic.compile_spec(result.spec, schema)
        seen: set[str] = set()
        for x in pairs(p, 2, FRESH):
            try:
                mapped = art.map(x.provider)
            except pycanonic.Rejected:
                out.rejected += 1
                continue
            for r in auto:
                d = differs(mapped, x.default, r.field.split("[]"))
                if d is not None and r.field not in seen:
                    seen.add(r.field)
                    out.wrong.append(Wrong(r.field, r.kind, r.rule, x.name, _show(d[1]), _show(d[0])))
    out.seconds = time.perf_counter() - start
    return out


def table(outcomes: list[Outcome]) -> str:
    lines = ["| provider | fields | auto | review | manual | auto share | wrong auto | fresh pairs rejected |", "|---|---|---|---|---|---|---|---|"]
    tot = {"auto": 0, "review": 0, "manual": 0, "wrong": 0, "rejected": 0}
    for o in outcomes:
        t = o.tiers()
        n = sum(t.values())
        lines.append(f"| {o.provider} | {n} | {t['auto']} | {t['review']} | {t['manual']} | {t['auto'] / n:.0%} | {len(o.wrong)} | {o.rejected} |")
        for k in ("auto", "review", "manual"):
            tot[k] += t[k]
        tot["wrong"] += len(o.wrong)
        tot["rejected"] += o.rejected
    n = tot["auto"] + tot["review"] + tot["manual"]
    lines.append(f"| total | {n} | {tot['auto']} | {tot['review']} | {tot['manual']} | {tot['auto'] / n:.0%} | {tot['wrong']} | {tot['rejected']} |")
    return "\n".join(lines) + "\n"


def all_outcomes() -> list[Outcome]:
    return [evaluate(p.name) for p in PROVIDERS]


def main() -> None:
    start = time.perf_counter()
    outcomes = all_outcomes()
    text = [
        "# fit evaluation results",
        "",
        f"Each provider is fitted on {TRAIN} pairs (seed 1). Then {FRESH} fresh pairs (seed 2) are mapped through the spec that fit wrote.",
        "An auto field is wrong if its value differs from the default's on a fresh pair. The suite fails on any wrong auto field.",
        "The auto share is reported, not gated. The target is 80%.",
        "",
        table(outcomes),
        f"Run time: {time.perf_counter() - start:.0f} seconds.",
        "",
    ]
    RESULTS.write_text("\n".join(text), encoding="utf-8")
    sys.stdout.write("\n".join(text))


if __name__ == "__main__":
    main()
