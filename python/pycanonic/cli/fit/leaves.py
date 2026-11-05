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
from .chains import ORDER, SAFE, Chain, apply, apply_rule, candidates, text
from .compare import Tolerance, match, same, used
from .index import Occurrence
from .pairs import Pair

# Transforms whose argument fit learns from one value's text, not from a fixed list.
_LEARNED = {"strip_prefix", "strip_suffix", "regex", "replace", "split"}
_KIND_RANK = {"path": 0, "join": 1, "coalesce": 2, "lookup": 3, "const": 4}
_CASE = {"lower", "upper", "title"}
_TOKEN = re.compile(r"[^\W_]{2,}")
_SEG = re.compile(r'\[(\d+)\]|\[("(?:[^"\\]|\\.)*")\]|\.?([^.\[\]]+)')


@dataclass
class LeafFit:
    field: str
    rule: dict[str, Any] | None
    kind: str  # path, a transform name, join, coalesce, lookup, const or none
    support: int
    total: int
    holdout: float
    examples: list[dict[str, Any]]
    alternatives: list[dict[str, Any]]
    mixed_empty: bool = False  # the default's samples disagree on the empty form: never auto
    one_value: bool = False  # the default sent one value or none in every pair: never auto
    tolerated: str | None = None  # the flags some pair matched only within: never auto


@dataclass(frozen=True)
class _Cand:
    kind: str
    paths: tuple[str, ...] = ()
    steps: tuple[tuple[str, str], ...] = ()  # (name, argument as JSON text): hashable
    const: str = "null"

    def label(self) -> str:
        """The kind: `path` with no steps, else the step off the safe list, else the last step."""
        if self.kind != "path" or not self.steps:
            return self.kind
        names = [n for n, _ in self.steps]
        return next((n for n in names if n not in SAFE), names[-1])

    def chain(self) -> Chain:
        return Chain([(n, json.loads(a)) for n, a in self.steps])

    def rule(self) -> dict[str, Any]:
        if self.kind == "const":
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


def _related(v: Any, t: Any, tol: Tolerance = Tolerance()) -> bool:
    """A cheap test that some chain could turn `v` into `t`, so the chain search skips unrelated values."""
    if match(v, t, tol) != "miss" or isinstance(t, bool):
        return True
    tv, tt = text(v).lower(), text(t).lower()
    if not tv or not tt:
        return False
    dv, dt = _digits(tv), _digits(tt)
    numeric = isinstance(v, (int, float)) or (isinstance(v, str) and v.strip().isdigit())
    if isinstance(t, (int, float)):
        if isinstance(v, bool) or (isinstance(v, str) and dv):
            return True  # casts, number parsing, date text to epoch
        return bool(dv) and bool(dt) and (dv in dt or dt in dv)
    if numeric:
        return bool(dv) and bool(dt)  # epoch to date
    if tt in tv or tv in tt or set(_TOKEN.findall(tv)) & set(_TOKEN.findall(tt)):
        return True
    return len(dv) >= 4 and len(dt) >= 4 and (dv in dt or dt in dv)


def _proposals(target: Any, occs: list[Occurrence], tol: Tolerance = Tolerance()) -> list[_Cand]:
    """Path, chain and join rules that reproduce `target` from this pair's provider body."""
    out: list[_Cand] = []
    related = [o for o in occs if _related(o.value, target, tol)]
    for o in related:
        out += [_cand("path", (o.path,), c) for c in candidates(target, o.value, tol)]
    if isinstance(target, str):
        parts = [o for o in related if not isinstance(o.value, bool) and text(o.value) and text(o.value) in target]
        for a in parts:
            for b in parts:
                if a.path != b.path and target.startswith(text(a.value)) and target.endswith(text(b.value)):
                    out += [_cand("join", (a.path, b.path), c) for c in candidates(target, [a.value, b.value]) if c.steps]
    return out


class _Search:
    def __init__(self, field: str, pairs: list[Pair], targets: list[Any], sources: list[list[Occurrence]], lookups: bool, floor: float, owners: list[int] | None = None, tol: Tolerance = Tolerance()) -> None:
        self.tol = tol
        self.targets, self.lookups, self.floor = targets, lookups, floor
        self.owner = owners or list(range(len(pairs)))  # the sample pair behind each (element) pair
        self.bodies = [p.provider for p in pairs]
        self.values = [{o.path: o.value for o in occs} for occs in sources]
        self.proposed = [_proposals(t, occs, tol) if t is not None else [] for t, occs in zip(targets, sources)]
        self.live = [j for j, t in enumerate(targets) if t is not None]
        self.memo: dict[_Cand, frozenset[int]] = {}
        keys = [k for k in _segs(field.replace("[]", "")) if isinstance(k, str)]
        self.leaf = Leaf(field, [keys[-1] if keys else field], keys[-2] if len(keys) > 1 else "", None, False, None)
        self.ctx = ScoreCtx(syn=SEED, known=None)
        self.names: dict[str, float] = {}
        self.seen: dict[_Cand, int] = {}  # first-seen order, the last tie-break

    def out(self, c: _Cand, j: int) -> Any:
        """What the rule maps for pair `j`."""
        if c.kind == "const":
            return json.loads(c.const)
        if c.kind in ("join", "coalesce"):  # the runtime reads objects and nulls the scalar index leaves out
            return apply_rule(c.rule(), self.bodies[j])
        x = self.values[j].get(c.paths[0])
        return None if x is None else apply(c.chain(), x)

    def hits(self, c: _Cand) -> frozenset[int]:
        """The pairs with a value that the rule reproduces, over all pairs, so holdout runs reuse it."""
        if c not in self.memo:
            self.memo[c] = frozenset(j for j in self.live if match(self.out(c, j), self.targets[j], self.tol) != "miss")
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
        cands = [c for c in dict.fromkeys(c for j in live for c in self.proposed[j]) if not self._one_off(c)]
        scored = {c: support(c) for c in cands}
        for c in self._coalesces(scored, support):
            scored[c] = support(c)
        best = max(scored.values(), default=0)
        if self.lookups and live and best < self.floor * len(live):
            for c in self._lookups(live):
                scored[c] = support(c)
        consts = {json.dumps(self.targets[j], sort_keys=True) for j in train}
        if live and len(live) == len(train) and len(consts) == 1:  # a constant holds in every pair, null included
            scored[_Cand("const", const=consts.pop())] = len(live)
        return sorted(scored.items(), key=lambda cs: self.rank(*cs))

    def _one_off(self, c: _Cand) -> bool:
        """A chain whose learned text argument (a prefix, a suffix, a pattern) changes the value in
        fewer than two sample pairs. Its argument is one pair's text, such as a school's town."""
        if c.kind != "path" or not any(n in _LEARNED for n, _ in c.steps):
            return False
        changed = {self.owner[j] for j in self.hits(c) if not same(self.values[j].get(c.paths[0]), self.targets[j])}
        return len(changed) < 2

    def _coalesces(self, scored: dict[_Cand, int], support: Callable[[_Cand], int]) -> list[_Cand]:
        """Per chain, add the path that adds the most pairs to the best path, up to three paths. The
        first step also tries the new path in front, so two paths are tried in both orders (a tie
        keeps the best path first)."""
        groups: dict[tuple[tuple[str, str], ...], list[_Cand]] = {}
        for c in sorted(scored, key=lambda c: self.rank(c, scored[c])):
            if c.kind == "path":
                groups.setdefault(c.steps, []).append(c)
        out: list[_Cand] = []
        for steps, group in groups.items():
            paths, best = group[0].paths, scored[group[0]]
            while len(paths) < 3:
                orders = [(*paths, c.paths[0]) for c in group if c.paths[0] not in paths]
                if len(paths) == 1:
                    orders = [o for ab in orders for o in (ab, ab[::-1])]
                tries = [(support(t), t) for t in (_Cand("coalesce", o, steps) for o in orders)]
                top = max(tries, key=lambda st: st[0], default=None)
                if top is None or top[0] <= best:
                    break
                best, paths = top[0], top[1].paths
                out.append(top[1])
        return out

    def _lookups(self, live: list[int]) -> list[_Cand]:
        """A table per path from its value to the target most pairs give it, scored by support like
        any rule, so one noisy pair does not hide it. A value whose targets tie has no
        majority, so the path gets no table. Only for scalar targets, when some value repeats (a table with one entry per pair is an ID,
        not an enum), and the targets differ (one target is a constant). An entry needs its value and
        target in two sample pairs: an entry seen in one pair memorizes that pair (a company's
        domain, a title's employment type), so it is left out, and a table left with no entry is none."""
        out: list[_Cand] = []
        for path in dict.fromkeys(p for j in live for p in self.values[j]):
            votes: dict[str, Counter[str]] = {}
            where: dict[tuple[str, str], set[int]] = {}
            first: dict[str, Any] = {}
            seen = 0
            for j in live:
                v, t = self.values[j].get(path), self.targets[j]
                if v is None:
                    continue
                if isinstance(t, (dict, list)):
                    break
                key = json.dumps(t, sort_keys=True)
                first.setdefault(key, t)
                votes.setdefault(text(v), Counter())[key] += 1
                where.setdefault((text(v), key), set()).add(self.owner[j])
                seen += 1
            else:
                tops = {k: c.most_common(2) for k, c in votes.items()}
                if any(len(t) > 1 and t[0][1] == t[1][1] for t in tops.values()):
                    continue  # a tie is no majority: the pairs disagree, so the path is not an enum
                table = {k: first[t[0][0]] for k, t in tops.items() if len(where[(k, t[0][0])]) >= 2}
                if table and len(table) < seen and len({json.dumps(t, sort_keys=True) for t in table.values()}) > 1:
                    arg = {"table": dict(sorted(table.items())), "else": "null"}
                    out.append(_cand("lookup", (path,), Chain([("lookup", arg)])))
        return out


def fit_leaf(
    field: str,
    pairs: list[Pair],
    targets: list[Any],
    sources_by_pair: list[list[Occurrence]],
    lookups_allowed: bool,
    floor: float = 0.8,
    owners: list[int] | None = None,
    tol: Tolerance = Tolerance(),
) -> LeafFit:
    """The best rule for `field` over the pairs, or kind `none` when no rule reaches `floor`, the
    share of pairs with a value. A learned lookup is tried only below it.

    Build `sources_by_pair[j]` with `scalars(pairs[j].provider)`: join and coalesce are scored on
    `pairs[j].provider`, so both must read the same body. Item-level fits build element pairs the
    same way, with the provider element as `provider` and `scalars(element)` as its occurrences.

    The holdout leaves out one sample pair at a time. For element pairs, `owners[j]` names the
    sample pair that element `j` came from, so all elements of one response leave together, and the pair holds only if every one of its elements is reproduced."""
    s = _Search(field, pairs, targets, sources_by_pair, lookups_allowed, floor, owners, tol)
    every = list(range(len(pairs)))
    live = [j for j in every if targets[j] is not None]
    ranked = s.select(every)
    chosen = ranked[0] if ranked and ranked[0][1] >= floor * len(live) else None
    rest_ranked = [(c, n) for c, n in ranked[1 if chosen else 0 :] if not (chosen and _case_variant(c, chosen[0]))]
    alternatives = [{"rule": c.rule(), "kind": c.label(), "support": n} for c, n in rest_ranked[:3]]
    if chosen is None:
        examples = [{"pair": pairs[j].name, "want": targets[j], "got": None} for j in live[:3]]
        return LeafFit(field, None, "none", 0, len(live), 0.0, examples, alternatives)
    c, support = chosen
    owner = owners or every
    units = list(dict.fromkeys(owner[j] for j in live))
    held = 0
    for u in units:  # leave each pair out: does the rule chosen without it reproduce it?
        out = [j for j in live if owner[j] == u]
        best = s.select([j for j in every if owner[j] != u])
        held += bool(best and best[0][1] >= floor * (len(live) - len(out)) and s.hits(best[0][0]).issuperset(out))
    order = sorted(live, key=lambda j: j in s.hits(c))  # failures first, stable in pair order
    examples = [{"pair": pairs[j].name, "want": targets[j], "got": s.out(c, j)} for j in order[:3]]
    flags = dict.fromkeys(f for j in s.hits(c) if match(s.out(c, j), targets[j], tol) == "tolerated" for f in used(s.out(c, j), targets[j], tol))
    return LeafFit(field, c.rule(), c.label(), support, len(live), held / len(units), examples, alternatives, tolerated=", ".join(flags) or None)


def _case_variant(c: _Cand, chosen: _Cand) -> bool:
    """`c` is the chosen rule with extra case-only steps, which tells a reviewer nothing new."""
    return c != chosen and (c.kind, c.paths) == (chosen.kind, chosen.paths) and [s for s in c.steps if s[0] not in _CASE] == list(chosen.steps)
