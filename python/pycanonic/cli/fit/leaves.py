"""The rule for one canonical leaf: the source path and transform chain that reproduce the default's
value in the most pairs (the design, steps 1-3), with the leave-one-out holdout of step 8."""

from __future__ import annotations

import json
import re
from collections import Counter
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from ..infer.score import Leaf, ScoreCtx, name_score
from ..infer.sources import Source
from ..infer.tokens import SEED
from .chains import ORDER, Chain, apply, candidates, text
from .compare import same
from .index import Occurrence
from .pairs import Pair

FLOOR = 0.8
"""No rule below this share of the pairs that have a value; a learned lookup is tried only then."""

_KIND_RANK = {"exact": 0, "transform": 0, "join": 1, "coalesce": 2, "lookup": 3, "constant": 4}
_TOKEN = re.compile(r"[^\W_]{2,}")
_SEG = re.compile(r'\[(\d+)\]|\[("(?:[^"\\]|\\.)*")\]|\.?([^.\[\]]+)')


@dataclass
class LeafFit:
    field: str
    rule: dict[str, Any] | None
    kind: str  # exact, transform, join, coalesce, lookup, constant, or none
    support: int
    total: int
    holdout: float
    examples: list[dict[str, Any]]
    alternatives: list[dict[str, Any]]


@dataclass(frozen=True)
class _Cand:
    kind: str
    paths: tuple[str, ...] = ()
    steps: tuple[tuple[str, str], ...] = ()  # (name, argument as JSON text): hashable
    const: str = "null"

    def chain(self) -> Chain:
        return Chain([(n, json.loads(a)) for n, a in self.steps])

    def rule(self) -> dict[str, Any]:
        if self.kind == "constant":
            return {"const": json.loads(self.const)}
        src: dict[str, Any] = {"coalesce": list(self.paths)} if self.kind == "coalesce" else {"from": list(self.paths) if self.kind == "join" else self.paths[0]}
        return {**src, "transform": self.chain().spec()} if self.steps else src


def _cand(kind: str, paths: tuple[str, ...], chain: Chain) -> _Cand:
    return _Cand(kind, paths, tuple((n, json.dumps(a, sort_keys=True)) for n, a in chain.steps))


def _segs(path: str) -> list[str | int]:
    if path == "$":
        return []
    return [int(i) if i else json.loads(q) if q else k for i, q, k in _SEG.findall(path)]


def _digits(s: str) -> str:
    return re.sub(r"\D", "", s)


def _related(v: Any, t: Any) -> bool:
    """A cheap test that some chain could turn `v` into `t`, so the chain search skips unrelated values."""
    if same(v, t) or isinstance(t, bool):
        return True
    tv, tt = text(v).lower(), text(t).lower()
    if not tv or not tt:
        return False
    dv, dt = _digits(tv), _digits(tt)
    if isinstance(t, (int, float)):
        return bool(dv) and bool(dt) and (dv in dt or dt in dv)  # casts and number parsing
    if isinstance(v, (int, float)):
        return bool(dv) and bool(dt)  # epoch to date
    if tt in tv or tv in tt or set(_TOKEN.findall(tv)) & set(_TOKEN.findall(tt)):
        return True
    return len(dv) >= 4 and len(dt) >= 4 and (dv in dt or dt in dv)


def _proposals(target: Any, occs: list[Occurrence]) -> list[_Cand]:
    """Path, chain and join rules that reproduce `target` from this pair's provider body."""
    out: list[_Cand] = []
    related = [o for o in occs if _related(o.value, target)]
    for o in related:
        out += [_cand("transform" if c.steps else "exact", (o.path,), c) for c in candidates(target, o.value)]
    if isinstance(target, str):
        parts = [o for o in related if not isinstance(o.value, bool) and text(o.value) and text(o.value) in target]
        for a in parts:
            for b in parts:
                if a.path != b.path and target.startswith(text(a.value)) and target.endswith(text(b.value)):
                    out += [_cand("join", (a.path, b.path), c) for c in candidates(target, [a.value, b.value]) if c.steps]
    return out


class _Search:
    def __init__(self, field: str, pairs: list[Pair], targets: list[Any], sources: list[list[Occurrence]], lookups: bool) -> None:
        self.targets, self.lookups = targets, lookups
        self.values = [{o.path: o.value for o in occs} for occs in sources]
        self.proposed = [_proposals(t, occs) if t is not None else [] for t, occs in zip(targets, sources)]
        self.live = [j for j, t in enumerate(targets) if t is not None]
        self.memo: dict[_Cand, frozenset[int]] = {}
        keys = [k for k in _segs(field.replace("[]", "")) if isinstance(k, str)]
        self.leaf = Leaf(field, [keys[-1] if keys else field], keys[-2] if len(keys) > 1 else "", None, False, None)
        self.ctx = ScoreCtx(syn=SEED, known=None)
        self.names: dict[str, float] = {}
        self.seen: dict[_Cand, int] = {}  # first-seen order, the last tie-break

    def out(self, c: _Cand, j: int) -> Any:
        """What the rule maps for pair `j`."""
        if c.kind == "constant":
            return json.loads(c.const)
        vals = [self.values[j].get(p) for p in c.paths]
        if c.kind == "join":
            x = None if all(v is None for v in vals) else vals
        else:
            x = next((v for v in vals if v is not None), None)
        return None if x is None else apply(c.chain(), x)

    def hits(self, c: _Cand) -> frozenset[int]:
        """The pairs with a value that the rule reproduces, over all pairs, so holdout runs reuse it."""
        if c not in self.memo:
            self.memo[c] = frozenset(j for j in self.live if same(self.out(c, j), self.targets[j]))
        return self.memo[c]

    def name(self, path: str) -> float:
        if path not in self.names:
            self.names[path] = name_score(self.leaf, Source(path, _segs(path), "scalar", []), self.ctx)
        return self.names[path]

    def rank(self, c: _Cand, support: int) -> tuple[Any, ...]:
        steps = tuple(ORDER.index(n) if n in ORDER else len(ORDER) for n, _ in c.steps)
        first = c.paths[0] if c.paths else ""
        seen = self.seen.setdefault(c, len(self.seen))
        return (-support, _KIND_RANK[c.kind], len(steps), max(steps, default=-1), steps, -self.name(first) if first else 0, len(_segs(first)), seen)

    def select(self, train: list[int]) -> list[tuple[_Cand, int]]:
        """Every candidate learned from the `train` pairs with its support there, best first."""
        live = [j for j in train if self.targets[j] is not None]
        in_train = set(live)
        support: Callable[[_Cand], int] = lambda c: len(self.hits(c) & in_train)
        cands = list(dict.fromkeys(c for j in live for c in self.proposed[j]))
        scored = {c: support(c) for c in cands}
        for c in self._coalesces(scored, support):
            scored[c] = support(c)
        best = max(scored.values(), default=0)
        if self.lookups and live and best < FLOOR * len(live):
            for c in self._lookups(live):
                scored[c] = support(c)
        if live:
            counts = Counter(json.dumps(self.targets[j], sort_keys=True) for j in live)
            const = min(counts, key=lambda k: (-counts[k], k))
            scored[_Cand("constant", const=const)] = counts[const]
        return sorted(scored.items(), key=lambda cs: self.rank(*cs))

    def _coalesces(self, scored: dict[_Cand, int], support: Callable[[_Cand], int]) -> list[_Cand]:
        """Per chain, add the path that adds the most pairs to the best path, up to three paths."""
        groups: dict[tuple[tuple[str, str], ...], list[_Cand]] = {}
        for c in sorted(scored, key=lambda c: self.rank(c, scored[c])):
            if c.kind in ("exact", "transform"):
                groups.setdefault(c.steps, []).append(c)
        out: list[_Cand] = []
        for steps, group in groups.items():
            paths, best = group[0].paths, scored[group[0]]
            while len(paths) < 3:
                tries = [(support(t), t) for t in (_Cand("coalesce", (*paths, c.paths[0]), steps) for c in group if c.paths[0] not in paths)]
                top = max(tries, key=lambda st: st[0], default=None)
                if top is None or top[0] <= best:
                    break
                best, paths = top[0], top[1].paths
                out.append(top[1])
        return out

    def _lookups(self, live: list[int]) -> list[_Cand]:
        """A table per path from its value to the target, when every value maps to one scalar target,
        some value repeats (a table with one entry per pair is an ID, not an enum), and the targets
        differ (one target is a constant)."""
        out: list[_Cand] = []
        for path in dict.fromkeys(p for j in live for p in self.values[j]):
            table: dict[str, Any] = {}
            seen, ok = 0, True
            for j in live:
                v, t = self.values[j].get(path), self.targets[j]
                if v is None:
                    continue
                k = text(v)
                if isinstance(t, (dict, list)) or (k in table and not same(table[k], t)):
                    ok = False
                    break
                table[k] = t
                seen += 1
            if ok and len(table) < seen and len({json.dumps(t, sort_keys=True) for t in table.values()}) > 1:
                arg = {"table": dict(sorted(table.items())), "else": "null"}
                out.append(_cand("lookup", (path,), Chain([("lookup", arg)])))
        return out


def fit_leaf(field: str, pairs: list[Pair], targets: list[Any], sources_by_pair: list[list[Occurrence]], lookups_allowed: bool) -> LeafFit:
    """The best rule for `field` over the pairs, or kind `none` when no rule reaches FLOOR."""
    s = _Search(field, pairs, targets, sources_by_pair, lookups_allowed)
    every = list(range(len(pairs)))
    live = [j for j in every if targets[j] is not None]
    ranked = s.select(every)
    chosen = ranked[0] if ranked and ranked[0][1] >= FLOOR * len(live) else None
    alternatives = [{"rule": c.rule(), "kind": c.kind, "support": n} for c, n in ranked[1 if chosen else 0 :][:3]]
    if chosen is None:
        examples = [{"pair": pairs[j].name, "want": targets[j], "got": None} for j in live[:3]]
        return LeafFit(field, None, "none", 0, len(live), 0.0, examples, alternatives)
    c, support = chosen
    held, rest = 0, len(live) - 1
    for i in live:  # leave each pair out: does the rule chosen without it reproduce it?
        best = s.select([j for j in every if j != i])
        held += bool(best and best[0][1] >= FLOOR * rest and i in s.hits(best[0][0]))
    order = sorted(live, key=lambda j: j in s.hits(c))  # failures first, stable in pair order
    examples = [{"pair": pairs[j].name, "want": targets[j], "got": s.out(c, j)} for j in order[:3]]
    return LeafFit(field, c.rule(), c.kind, support, len(live), held / len(live), examples, alternatives)
