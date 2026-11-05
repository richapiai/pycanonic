"""Draft a spec from a canonical schema and sample responses. Offline: nothing here runs per request.
Passes: one-to-one assignment of scalar fields (Hungarian), then coalesce, split and join for what a
one-to-one match cannot express, then arrays (item, recursively, pluck, or plain values), then anchors."""

from __future__ import annotations

import json
import math
from dataclasses import dataclass, replace
from decimal import ROUND_HALF_UP, Decimal
from typing import Any

from ..._api import compile_spec
from .hungarian import assign
from .learn import learn_synonyms
from .score import (
    ArrayField,
    Leaf,
    ScoreCtx,
    Scored,
    Steps,
    backs,
    choose_steps,
    key_tokens,
    looks_full_name,
    path_tokens,
    score,
    share,
    steps_for,
    walk_canonical,
)
from .sources import Source, collect, elements
from .tokens import SEED, Synonyms, similarity, split, synonyms_of, tokens


@dataclass(frozen=True)
class FieldReport:
    """One line of the report. `how` is match, coalesce, split, join, item, pluck, values, ask or
    unmapped. `confidence` is the score (for unmapped, the best candidate's, if any). `check` marks
    the lines the report flags with `<- check`."""

    field: str
    how: str
    rule: Any = None
    confidence: float | None = None
    best: str | None = None
    name: float | None = None
    value: float | None = None
    overlap: float | None = None
    check: bool = False


@dataclass
class Inferred:
    spec: dict[str, Any]
    fields: list[FieldReport]
    unmapped: list[dict[str, Any]]
    """Canonical fields left unmapped, with the best candidate that did not make the threshold."""


def with_steps(steps: Steps) -> dict[str, Any]:
    return {} if not steps else {"transform": steps[0] if len(steps) == 1 else steps}


def rule(frm: str, steps: Steps) -> Any:
    return frm if not steps else {"from": frm, **with_steps(steps)}


FULL = ["full", "name"]
WEAK = 0.6
PLUCK_KEYS = ["name", "label", "value", "title", "text"]


def js_round(x: float) -> float:
    """Math.round(x * 100) / 100: halves round up, not to even."""
    y = x * 100
    f = math.floor(y)
    return (f + (1 if y - f >= 0.5 else 0)) / 100


def to_fixed(x: float) -> str:
    """Number.prototype.toFixed(2): the exact binary value, halves up."""
    return str(Decimal(x).quantize(Decimal("0.01"), ROUND_HALF_UP))


def last_seg(s: Source) -> str:
    """String(s.segs.at(-1)): "undefined" for the response itself."""
    return str(s.segs[-1]) if s.segs else "undefined"


class Pass:
    def __init__(self, leaves: list[Leaf], arrays: list[ArrayField], sources: list[Source], ctx: ScoreCtx) -> None:
        self.leaves = leaves
        self.arrays = arrays
        self.sources = sources
        self.ctx = ctx
        self.fields: dict[str, Any] = {}
        self.report: list[FieldReport] = []
        self.used: set[str] = set()
        self.done: set[str] = set()
        self.scalars = [s for s in sources if s.kind == "scalar"]
        self.scores = [[score(l, s, ctx) for s in self.scalars] for l in leaves]

    def put(self, entry: FieldReport, sc: Scored | None = None) -> None:
        self.fields[entry.field] = entry.rule
        self.done.add(entry.field)
        self.report = [r for r in self.report if r.field != entry.field]
        assert entry.confidence is not None
        signals: dict[str, Any] = {}
        if sc:
            signals = {"name": js_round(sc.name), "value": js_round(sc.value)}
            if sc.overlap is not None:
                signals["overlap"] = js_round(sc.overlap)
        self.report.append(replace(entry, confidence=js_round(entry.confidence), **signals))

    def match(self) -> None:
        matrix = [[sc.score if sc.score >= self.ctx.min else 0 for sc in row] for row in self.scores]
        picks = assign(matrix)
        taken = {j for j in picks if j != -1}
        for i, j in enumerate(picks):
            if j == -1:
                continue
            l = self.leaves[i]
            s = self.scalars[j]
            partners = self.coalesce_with(i, j, taken)
            self.used.add(s.path)
            for p in partners:
                self.used.add(p.path)
            steps = choose_steps(l, s, self.ctx.known)
            sc = self.scores[i][j]
            if not partners:
                self.put(FieldReport(l.path, "match", rule(s.path, steps), sc.score), sc)
            else:
                coalesce = [s.path, *(p.path for p in partners)]
                self.put(FieldReport(l.path, "coalesce", {"coalesce": coalesce, **with_steps(steps)}, sc.score), sc)

    def coalesce_with(self, i: int, j: int, taken: set[int]) -> list[Source]:
        """Other unmatched sources that fill in where the match is empty, best first."""
        main = self.scalars[j]
        l = self.leaves[i]
        found = [
            (s, self.scores[i][k].score)
            for k, s in enumerate(self.scalars)
            if s is not main and k not in taken and backs(l, main, s, self.ctx)
        ]
        return [s for s, _ in sorted(found, key=lambda x: -x[1])]

    def key_tokens(self, s: Source) -> list[str]:
        return key_tokens(last_seg(s), self.ctx)

    def open(self, l: Leaf) -> bool:
        """Not mapped yet, or mapped by a match weaker than a split or join would be."""
        return l.path not in self.done or any(
            r.field == l.path and r.how == "match" and r.confidence is not None and r.confidence < WEAK for r in self.report
        )

    def leaf_tokens(self, l: Leaf) -> list[str]:
        return [t for n in l.names for t in tokens(n, self.ctx.syn)]

    def names(self) -> None:
        """first_name and last_name split from one full-name source, and full_name joined from first and last."""
        fulls = [
            s
            for s in self.scalars
            if "name" in self.key_tokens(s) and share([v for v in s.values if v is not None], looks_full_name) >= 0.8
        ]
        fulls.sort(key=lambda s: -similarity(FULL, self.key_tokens(s)))
        if fulls:
            for l in [x for x in self.leaves if self.open(x)]:
                self.split(l, fulls[0])
        first = self.with_tokens(["first", "name"])
        last = self.with_tokens(["last", "name"])
        if not first or not last:
            return
        for l in [x for x in self.leaves if self.open(x)]:
            t = self.leaf_tokens(l)
            if l.semantic == "full_name" or ("full" in t and "name" in t):
                self.put(FieldReport(l.path, "join", {"from": [first.path, last.path], "transform": "join"}, 0.6))

    def split(self, l: Leaf, full: Source) -> None:
        t = self.leaf_tokens(l)
        part = None if "name" not in t else "first" if "first" in t else "last" if "last" in t else None
        if not part:
            return
        confidence = 0.6 + 0.3 * similarity(FULL, self.key_tokens(full))
        self.put(FieldReport(l.path, "split", {"from": full.path, "transform": {"split_name": part}}, confidence))

    def with_tokens(self, want: list[str]) -> Source | None:
        """The best source for a first or last name: its key reads like want, and it is not a list element."""
        found = [
            (s, sim)
            for s in self.scalars
            if not any(isinstance(x, int) for x in s.segs)
            for sim in [similarity(want, path_tokens(s, self.ctx))]
            if sim >= 0.8
        ]
        found.sort(key=lambda x: -x[1])
        return found[0][0] if found else None

    def arrays_pass(self) -> None:
        objects = [s for s in self.sources if s.kind == "objects"]
        items = [a for a in self.arrays if a.objects]
        subs = [[infer_object(a.item, elements(s), replace(self.ctx, known=None)) for s in objects] for a in items]
        scores = [[self.array_score(a, s, subs[i][k]) for k, s in enumerate(objects)] for i, a in enumerate(items)]
        picks = assign([[x if x >= self.ctx.min else 0 for x in row] for row in scores])
        for i, k in enumerate(picks):
            # A name match with no item field mapped would give "item": {}, which is not a valid rule.
            if k == -1 or not subs[i][k].fields:
                continue
            a = items[i]
            s = objects[k]
            self.used.add(s.path)
            self.put(FieldReport(f"{a.path}[]", "item", {"from": s.path, "item": subs[i][k].fields}, scores[i][k]))
            for r in subs[i][k].report:
                self.report.append(replace(r, field=f"{a.path}[].{r.field}"))
        for a in [x for x in self.arrays if not x.objects]:
            self.pluck(a, objects)

    def array_score(self, a: ArrayField, s: Source, sub: Pass) -> float:
        name = similarity(tokens(a.name, self.ctx.syn), tokens(last_seg(s), self.ctx.syn))
        n = len(walk_canonical(a.item)[0])
        total = sum(r.confidence or 0 for r in sub.report)
        quality = (total / n if n else math.inf) if sub.report else 0
        # The response itself (`$`) has no name to compare, so its elements decide.
        return min(1, quality) if not s.segs else 0.5 * name + 0.5 * min(1, quality)

    def scalar_arrays(self, a: ArrayField, objects: list[Source]) -> list[tuple[Source, Source, float, bool]]:
        """Sources for an array of scalars: a plain-value array as it is, or one key plucked from each object."""
        want = tokens(a.name, self.ctx.syn)

        def name_of(s: Source) -> float:
            return similarity(want, tokens(last_seg(s), self.ctx.syn))

        values = [
            (s, replace(s, kind="scalar", values=[x for v in s.values if isinstance(v, list) for x in v]), name_of(s), True)
            for s in self.sources
            if s.kind == "scalars" and s.path not in self.used
        ]
        plucked = []
        for s in objects:
            if s.path in self.used:
                continue
            for key in collect(elements(s)):
                if key.kind != "scalar" or len(key.segs) != 1:
                    continue
                k = str(key.segs[0])
                key_score = max(similarity(want, tokens(k, self.ctx.syn)), 0.8 if k in PLUCK_KEYS else 0)
                plucked.append((s, key, 0.6 * name_of(s) + 0.4 * key_score, False))
        return [*values, *plucked]

    def pluck(self, a: ArrayField, objects: list[Source]) -> None:
        best = None
        for c in self.scalar_arrays(a, objects):
            if c[2] >= self.ctx.min and (best is None or c[2] > best[2]):
                best = c
        if not best:
            return
        s, key, sc, whole = best
        found = walk_canonical({"properties": {a.name: a.item}})[0]
        item = found[0] if found else Leaf(a.path, [a.name], "", None, False, None)
        leaf = replace(item, path=a.path, names=[a.name])
        self.used.add(s.path)
        steps = steps_for(leaf, key)
        r = {"from": s.path, **with_steps(steps)} if whole else {"from": s.path, "pluck": key.path, **with_steps(steps)}
        self.put(FieldReport(f"{a.path}[]", "values" if whole else "pluck", r, sc))

    def unmapped(self) -> list[dict[str, Any]]:
        out: list[dict[str, Any]] = []
        for i, l in enumerate(self.leaves):
            if l.path in self.done:
                continue
            k = -1
            for j, sc in enumerate(self.scores[i]):
                if sc.score > (self.scores[i][k].score if k != -1 else 0):
                    k = j
            out.append({"field": l.path} if k == -1 else {"field": l.path, "best": self.scalars[k].path, "score": js_round(self.scores[i][k].score)})
        return out + [{"field": f"{a.path}[]"} for a in self.arrays if f"{a.path}[]" not in self.done]


def noise_prefixes(sources: list[Source], leaves: list[Leaf], objects: set[str], syn: Synonyms) -> dict[str, str]:
    """First tokens of three or more keys that are not words: short, not a synonym, not canonical. A
    one-letter prefix that starts exactly one canonical object's name stands for it ("c" -> company)."""
    words = {t for l in leaves for t in [*split(l.parent), *(w for n in l.names for w in split(n))]}
    firsts = [t for s in sources for t in split(last_seg(s))[:1] if len(t) <= 3]
    out: dict[str, str] = {}
    for t in dict.fromkeys(firsts):
        if firsts.count(t) < 3 or synonyms_of(syn, t) is not None or t in words:
            continue
        named = [o for o in objects if len(t) == 1 and o.startswith(t)]
        out[t] = named[0] if len(named) == 1 else ""
    return out


def infer_object(schema: Any, samples: list[Any], base: ScoreCtx) -> Pass:
    leaves, arrays = walk_canonical(schema)
    sources = collect(samples)
    objects = {t for l in leaves if l.parent for t in tokens(l.parent, base.syn)}
    ctx = replace(base, noise=noise_prefixes(sources, leaves, objects, base.syn), objects=objects)
    p = Pass(leaves, arrays, sources, ctx)
    p.match()
    p.names()
    p.arrays_pass()
    return p


def anchors(leaves: list[Leaf], fields: dict[str, Any], sources: list[Source], raws: list[str]) -> dict[str, str]:
    """Anchor tokens for routing fields read from one key that occurs exactly once in every raw sample."""
    found: list[tuple[str, str, int]] = []
    for l in [x for x in leaves if x.routing]:
        r = fields.get(l.path)
        frm = r if isinstance(r, str) else r.get("from") if isinstance(r, dict) else None
        if not isinstance(frm, str):
            continue
        key = next((s.segs[-1] if s.segs else None for s in sources if s.path == frm), None)
        if not isinstance(key, str):
            continue
        token = f"{json.dumps(key, ensure_ascii=False)}:"
        if all(t.count(token) == 1 for t in raws):
            found.append((l.path, token, raws[0].find(token)))
    return {f: t for f, t, _ in sorted(found, key=lambda x: x[2])}


def infer_draft(
    schema: Any,
    samples: list[Any],
    *,
    provider: str,
    endpoint: str,
    synonyms: Synonyms | None = None,
    known: list[Any] | None = None,
    raws: list[str] | None = None,
    min_score: float = 0.4,
) -> Inferred:
    """The TypeScript `infer()`: synonyms merge over the seed, and anchors are only proposed with raws."""
    ctx = ScoreCtx(syn={**SEED, **(synonyms or {})}, known=known, min=min_score)
    p = infer_object(schema, samples, ctx)
    spec: dict[str, Any] = {"provider": provider, "endpoint": endpoint, "version": 1, "status": "draft", "fields": p.fields}
    a = anchors(walk_canonical(schema)[0], p.fields, collect(samples), raws) if raws is not None else {}
    if a:
        spec["anchors"] = a
    # The draft must compile, so a failure here is a bug in inference. An empty spec is not valid, so it is skipped.
    if p.fields:
        compile_spec(spec, schema)
    return Inferred(spec, p.report, p.unmapped())


def infer(
    samples: list[Any],
    schema: dict[str, Any],
    *,
    provider: str,
    endpoint: str,
    known: list[Any] | None = None,
    synonyms: Synonyms | None = None,
    learn: list[dict[str, Any]] | None = None,
    min_score: float = 0.4,
    ask: str | None = None,
    raws: list[str] | None = None,
    ask_timeout: float = 120.0,
) -> tuple[dict[str, Any], list[FieldReport]]:
    """The draft spec (status "draft") and one report line per canonical field.

    `known` is the canonical output per sample (None where unknown). `learn` takes approved specs to
    learn synonyms from (`synonyms` wins on conflicts). `ask` is a shell command asked about each
    doubtful scalar field, and `ask_timeout` the seconds one ask may run. `raws` is each sample's raw text, needed to propose anchors."""
    from .ask import LOW, ask_low_confidence, ask_with  # here, not at the top, because ask.py imports FieldReport from this module

    for i, s in enumerate(learn or []):
        if not (isinstance(s, dict) and isinstance(s.get("fields"), dict)):
            raise ValueError(f"learn[{i}]: an approved spec needs a fields object")
    learned = learn_synonyms([s["fields"] for s in learn], schema) if learn else {}
    merged = {**learned, **(synonyms or {})}
    r = infer_draft(schema, samples, provider=provider, endpoint=endpoint, synonyms=merged, known=known, raws=raws, min_score=min_score)
    if ask:
        r = ask_low_confidence(r, schema, samples, ask_with(ask, ask_timeout))
    report = [replace(f, check=(f.confidence or 0) < LOW or f.how == "ask") for f in r.fields]
    report += [FieldReport(u["field"], "unmapped", best=u.get("best"), confidence=u.get("score")) for u in r.unmapped]
    return r.spec, report
