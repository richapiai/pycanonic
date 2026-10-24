"""The rule for one canonical array (the design, step 4): the provider array it comes from, and
whether it is a plain copy, a `pluck`, an `item` map, `wrap`, `flatten`, `group_by` or `filter`.

The runtime does every mapping here. A candidate's elements come from a compiled rule, and the
finished rule is scored by mapping each pair, so `fit` and `map` agree.

Element pairs follow the leaf convention of `fit_leaf`: an element pair's `provider` is the provider
element, and its occurrences come from `scalars(element)`, as a top-level leaf's come from
`scalars(pair.provider)`, because join and coalesce read `Pair.provider`."""

from __future__ import annotations

import json
from dataclasses import dataclass, replace
from typing import Any

from ..infer.score import Leaf, ScoreCtx, name_score
from ..infer.sources import Seg, Source, format_path
from ..infer.tokens import SEED
from .chains import apply_rule, text
from .compare import same
from .index import scalars
from .leaves import LeafFit, _segs, fit_leaf
from .pairs import Pair

FLOOR = 0.8
KINDS = ["path", "wrap", "flatten", "group", "filter"]  # the order the brief tries them in, also the tie-break
_ANY: dict[str, Any] = {"type": "array"}  # elements are copied as they are
_AGG = [(op, nulls) for op in ("first", "last", "min", "max") for nulls in ("skip", "propagate")]
_G = "g"  # element pairs of a group hold its elements under this key; the rule reads them as $group
MISSING: Any = object()


def get(doc: Any, segs: list[Seg]) -> Any:
    """The value at `segs`, or MISSING."""
    for s in segs:
        if isinstance(s, int):
            if not isinstance(doc, list) or not 0 <= s < len(doc):
                return MISSING
            doc = doc[s]
        elif isinstance(doc, dict) and s in doc:
            doc = doc[s]
        else:
            return MISSING
    return doc


def at(doc: Any, field: str) -> Any:
    """The default's value for a canonical field key (`jobs[]`, `hq.city`, `[]` for the root), None if missing."""
    v = get(doc, _segs(field.removesuffix("[]")))
    return None if v is MISSING else v


def name(field: str, path: str) -> float:
    """How well the provider `path` matches the canonical `field` by name (the infer token score)."""
    keys = [k for k in _segs(field.replace("[]", "")) if isinstance(k, str)]
    leaf = Leaf(field, [keys[-1] if keys else field], keys[-2] if len(keys) > 1 else "", None, False, None)
    return name_score(leaf, Source(path, _segs(path), "scalar", []), ScoreCtx(syn=SEED, known=None))


def rel_paths(elems: list[Any]) -> list[str]:
    """Scalar paths inside the elements, without array positions, in first-seen order."""
    return list(dict.fromkeys(o.path for x in elems for o in scalars(x) if o.path != "$" and not any(isinstance(s, int) for s in _segs(o.path))))


def _flat(chunks: list[list[Seg]]) -> str:
    """A path that collects across arrays: `jobs[].skills[]`, or `[].x[]` from a bare-array body."""
    out = format_path(chunks[0]) if chunks[0] else ""
    for c in chunks[1:]:
        p = format_path(c)
        out += "[]" + ("" if p.startswith("[") else ".") + p
    return out + "[]"


def _sources(bodies: list[Any], wrap: bool) -> list[tuple[str, dict[str, Any]]]:
    """Every (kind, base rule) that can feed the array, in first-seen order: arrays outside arrays
    (`path`), arrays inside array elements (`flatten`) and, with `wrap`, other values outside arrays."""
    found: dict[str, tuple[str, dict[str, Any]]] = {}

    def add(kind: str, base: dict[str, Any]) -> None:
        found.setdefault(repr(base), (kind, base))

    def walk(v: Any, chunks: list[list[Seg]]) -> None:
        if isinstance(v, list):
            if len(chunks) == 1:
                add("path", {"from": format_path(chunks[0])})
            else:
                add("flatten", {"from": _flat(chunks)})
            for x in v:
                if isinstance(x, dict):
                    walk(x, [*chunks, []])
            return
        if wrap and len(chunks) == 1 and chunks[0] and v is not None:
            add("wrap", {"from": format_path(chunks[0]), "wrap": True})
        if isinstance(v, dict):
            for k, x in v.items():
                walk(x, [*chunks[:-1], [*chunks[-1], k]])

    for b in bodies:
        walk(b, [[]])
    return list(found.values())


def _texts(v: Any) -> set[str]:
    return {text(o.value).lower() for o in scalars(v)}


def _overlap(d: Any, e: Any) -> int:
    """How many scalar values a default element shares with a provider element, ignoring case."""
    return len(_texts(d) & _texts(e))


def _runs(elems: list[Any], segs: list[Seg]) -> list[list[Any]]:
    """Consecutive groups by the key at `segs`, compared as JSON text. A null or missing key is its own group."""
    runs: list[list[Any]] = []
    last: str | None = None
    for x in elems:
        v = get(x, segs)
        key = None if v is MISSING or v is None else json.dumps(v, sort_keys=True)
        if runs and key is not None and key == last:
            runs[-1].append(x)
        else:
            runs.append([x])
        last = key
    return runs


def _match(d: list[Any], e: list[Any]) -> list[int] | None:
    """The positions in `e` of a subsequence that matches every element of `d` with the most shared
    values, or None when no such subsequence exists."""
    n, m = len(d), len(e)
    ov = [[_overlap(x, y) for y in e] for x in d]
    best = [[0] * (m + 1)] + [[-1] * (m + 1) for _ in range(n)]
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            best[i][j] = best[i][j - 1]
            if ov[i - 1][j - 1] > 0 and best[i - 1][j - 1] >= 0:
                best[i][j] = max(best[i][j], best[i - 1][j - 1] + ov[i - 1][j - 1])
    if best[n][m] < 0:
        return None
    out: list[int] = []
    i, j = n, m
    while i:
        if best[i][j] == best[i][j - 1]:
            j -= 1
        else:
            out.append(j - 1)
            i, j = i - 1, j - 1
    return out[::-1]


def _group_key(field: str, props: dict[str, Any], wants: list[Any], elems: list[list[Any]], live: list[int]) -> tuple[str, list[list[list[Any]]]] | None:
    """The element path that groups the provider array into the default's elements, with the groups
    per live pair. The group count must equal the default's length in every pair, and one nested
    array of the default must have the group sizes."""
    nested = [k for k, sub in props.items() if isinstance(sub, dict) and sub.get("type") == "array"]
    if not nested or any(len(elems[j]) < len(wants[j]) for j in live) or not any(len(elems[j]) > len(wants[j]) for j in live):
        return None
    ok: list[tuple[str, list[list[list[Any]]]]] = []
    for k in rel_paths([x for j in live for x in elems[j]]):
        groups = [_runs(elems[j], _segs(k)) for j in live]
        if any(len(g) != len(wants[j]) for g, j in zip(groups, live)):
            continue
        sizes = any(
            all(isinstance(want, dict) and isinstance(want.get(q), list) and len(want[q]) == len(run) for g, j in zip(groups, live) for want, run in zip(wants[j], g))
            for q in nested
        )
        if sizes:
            ok.append((k, groups))
    if not ok:
        return None
    return min(ok, key=lambda kg: (-name(field, kg[0]), len(_segs(kg[0])), ok.index(kg)))


def _separator(kept: list[list[Any]], dropped: list[list[Any]]) -> tuple[str, Any] | None:
    """An element path whose value is the same scalar in every kept element and never that value in
    a dropped one, in every pair."""
    keep = [x for xs in kept for x in xs]
    drop = [x for xs in dropped for x in xs]
    if not drop:
        return None
    for p in rel_paths(keep):
        segs = _segs(p)
        vals = [get(x, segs) for x in keep]
        v = vals[0]
        if all(x is not MISSING and same(x, v) for x in vals) and not any(same(get(x, segs), v) for x in drop):
            return p, v
    return None


def _regroup(rule: dict[str, Any] | None) -> dict[str, Any] | None:
    """A nested rule learned from the `g` key of a group's element pairs, reading `$group` instead."""
    src = rule.get("from") if rule else None
    if rule is None or not isinstance(src, str) or not (src == _G or src.startswith(_G + "[]")):
        return None
    return {**rule, "from": "$group" + src[len(_G) :]}


def _under(rel: str) -> str:
    return f"{_G}[]{'' if rel.startswith('[') else '.'}{rel}"


def _aggregate(f: LeafFit, key: str, eps: list[Pair], targets: list[Any], groups: list[list[Any]]) -> LeafFit:
    """The leaf rule of a grouped item: the plain rule (it reads the group's first element) or an
    aggregate over `$group`, whichever reproduces more of the default. Null targets break ties, so
    `nulls: propagate` wins where the default is null for an open group."""
    full = [j for j, t in enumerate(targets) if t is not None]
    nulls = [j for j, t in enumerate(targets) if t is None]
    cands: list[tuple[dict[str, Any], list[Any]]] = []
    if f.rule is not None:
        cands.append((f.rule, [apply_rule(f.rule, e.provider) for e in eps]))
    for p in rel_paths([x for g in groups for x in g]):
        for op, how in _AGG:
            agg = {"aggregate": op, "from": _under(p), **({"nulls": how} if how == "propagate" else {})}
            cands.append(({**agg, "from": "$group" + agg["from"][len(_G) :]}, [apply_rule(agg, {_G: g}) for g in groups]))
    hits = [frozenset(j for j, (o, t) in enumerate(zip(outs, targets)) if same(o, t)) for _, outs in cands]

    def best(skip: int | None = None) -> int | None:
        score = [(len(h & set(full) - {skip}), len(h & set(nulls))) for h in hits]
        return max(range(len(cands)), key=lambda i: score[i], default=None)

    top = best()
    if top is None or (f.rule is not None and top == 0) or len(hits[top] & set(full)) < max(1, FLOOR * len(full)):
        return f
    rule, outs = cands[top]
    held = sum(1 for j in full if (b := best(j)) is not None and j in hits[b])
    order = sorted(full, key=lambda j: j in hits[top])
    examples = [{"pair": eps[j].name, "want": targets[j], "got": outs[j]} for j in order[:3]]
    rest = sorted((i for i in range(len(cands)) if i != top), key=lambda i: -len(hits[i] & set(full)))
    alternatives = [{"rule": cands[i][0], "kind": "group", "support": len(hits[i] & set(full))} for i in rest[:3]]
    return LeafFit(key, rule, "group", len(hits[top] & set(full)), len(full), held / len(full), examples, alternatives)


@dataclass
class _Items:
    part: dict[str, Any]  # `item`, or `pluck` and `transform`, to add to the base rule
    fits: list[LeafFit]  # item leaves and nested arrays, field keys relative to the element
    kind: str  # the element rule's kind for scalar items, else path
    holdout: float


def _object(props: dict[str, Any], eps: list[Pair], groups: list[list[Any]] | None, prefix: list[Seg]) -> tuple[dict[str, Any], list[LeafFit]]:
    item: dict[str, Any] = {}
    fits: list[LeafFit] = []
    for k, sub in props.items():
        segs = [*prefix, k]
        key = format_path(segs)
        sub = sub if isinstance(sub, dict) else {}
        if sub.get("type") == "array":
            sub_pairs = eps if groups is None else [Pair(e.name, e.default, {_G: g}, e.inputs) for e, g in zip(eps, groups)]
            rule, sub_fits = fit_array(key + "[]", sub, sub_pairs)
            # a nested array in a group reads $group only, not the first element; add the
            # first element as a second source if a provider needs it.
            rule = rule if groups is None else _regroup(rule)
            if rule is not None:
                item[key + "[]"] = rule
            fits += sub_fits
        elif "properties" in sub:
            inner, inner_fits = _object(sub["properties"], eps, groups, segs)
            item.update(inner)
            fits += inner_fits
        else:
            targets = [None if (v := get(e.default, segs)) is MISSING else v for e in eps]
            f = fit_leaf(key, eps, targets, [scalars(e.provider) for e in eps], True)
            if groups is not None:
                f = _aggregate(f, key, eps, targets, groups)
            if f.rule is not None:
                item[key] = f.rule
            fits.append(f)
    return item, fits


def _items(field: str, items: dict[str, Any], eps: list[Pair], groups: list[list[Any]] | None) -> _Items | None:
    """The per-element part of the rule, fitted over element pairs (default element, provider element)."""
    if not eps:
        return None
    if "properties" in items:
        item, fits = _object(items["properties"], eps, groups, [])
        held = [f.holdout for f in fits if f.rule is not None]
        return _Items({"item": item}, fits, "path", min(held, default=1.0)) if item else None
    f = fit_leaf(field, eps, [e.default for e in eps], [scalars(e.provider) for e in eps], True)
    src = f.rule.get("from") if f.rule else None
    if f.rule is None or not isinstance(src, str):
        return None  # join, coalesce and const have no per-element form
    part = {} if src == "$" else {"pluck": src}
    if "transform" in f.rule:
        part["transform"] = f.rule["transform"]
    return _Items(part, [], f.kind, f.holdout)


@dataclass
class _Tried:
    kind: str
    rule: dict[str, Any]
    items: _Items
    hits: frozenset[int]
    outs: list[Any]


def fit_array(field: str, node: dict[str, Any], pairs: list[Pair]) -> tuple[dict[str, Any] | None, list[LeafFit]]:
    """The array rule for `field` and the fits under it. The first fit is the array itself, with kind
    path (or the element transform for scalar items), wrap, flatten, group, filter or none; the rest
    are the item leaves and nested arrays, keyed `field.leaf`. Pairs where the default has no list do
    not count. The holdout of the array is the lowest holdout of its item rules."""
    wants = [at(p.default, field) for p in pairs]
    live = [j for j, w in enumerate(wants) if isinstance(w, list)]
    if not live:
        return None, []
    items: dict[str, Any] = node["items"] if isinstance(node.get("items"), dict) else {}
    objects = "properties" in items
    srcs = _sources([p.provider for p in pairs], all(len(wants[j]) <= 1 for j in live))
    elems: list[list[list[Any]]] = []
    for _, base in srcs:
        got = [apply_rule(base, p.provider, _ANY) for p in pairs]
        elems.append([g if isinstance(g, list) else [] for g in got])

    def element_pairs(j: int, d: list[Any], e: list[Any]) -> list[Pair]:
        return [Pair(f"{pairs[j].name}[{i}]", x, y, pairs[j].inputs) for i, (x, y) in enumerate(zip(d, e))]

    tried: list[_Tried] = []

    def score(kind: str, rule: dict[str, Any], it: _Items | None) -> None:
        if it is None:
            return
        full = {**rule, **it.part}
        outs = [apply_rule(full, p.provider, node) for p in pairs]
        tried.append(_Tried(kind, full, it, frozenset(j for j in live if same(outs[j], wants[j])), outs))

    for kind in KINDS:
        # only the best source by shared values gets the full item fit; try the next ones
        # too if a provider has two near-identical arrays.
        if kind in ("path", "wrap", "flatten"):
            aligned = [
                (sum(_overlap(x, y) for j in live for x, y in zip(wants[j], es[j])), i)
                for i, ((k, _), es) in enumerate(zip(srcs, elems))
                if k == kind and sum(len(es[j]) == len(wants[j]) for j in live) >= FLOOR * len(live)
            ]
            top = max(aligned, key=lambda oi: (oi[0], -oi[1]), default=(0, -1))
            if top[0] > 0:
                es = elems[top[1]]
                eps = [ep for j in live if len(es[j]) == len(wants[j]) for ep in element_pairs(j, wants[j], es[j])]
                score(kind, srcs[top[1]][1], _items(field, items, eps, None))
        elif kind == "group" and objects:
            for (k, base), es in zip(srcs, elems):
                found = _group_key(field, items["properties"], wants, es, live) if k in ("path", "flatten") else None
                if found:
                    key, groups = found
                    flat = [(ep, g) for j, gs in zip(live, groups) for ep, g in zip(element_pairs(j, wants[j], [g[0] for g in gs]), gs)]
                    it = _items(field, items, [ep for ep, _ in flat], [g for _, g in flat])
                    score(kind, {**base, "group_by": key, "consecutive": True}, it)
                    break
        elif kind == "filter":
            for (k, base), es in zip(srcs, elems):
                if k not in ("path", "flatten") or not any(len(es[j]) > len(wants[j]) for j in live):
                    continue
                matches = [_match(wants[j], es[j]) for j in live]
                if any(m is None for m in matches):
                    continue
                kept = [[es[j][i] for i in m or []] for j, m in zip(live, matches)]
                dropped = [[x for i, x in enumerate(es[j]) if i not in (m or [])] for j, m in zip(live, matches)]
                sep = _separator(kept, dropped)
                if sep:
                    eps = [ep for j, ks in zip(live, kept) for ep in element_pairs(j, wants[j], ks)]
                    score(kind, {**base, "filter": {"path": sep[0], "eq": sep[1]}}, _items(field, items, eps, None))
                    break
        if any(len(t.hits) == len(live) for t in tried):
            break

    ranked = sorted(tried, key=lambda t: (-len(t.hits), KINDS.index(t.kind)))
    alternatives = [{"rule": t.rule, "kind": t.kind, "support": len(t.hits)} for t in ranked[1:4]]
    if not ranked or len(ranked[0].hits) < FLOOR * len(live):
        examples = [{"pair": pairs[j].name, "want": wants[j], "got": None} for j in live[:3]]
        return None, [LeafFit(field, None, "none", 0, len(live), 0.0, examples, [{"rule": t.rule, "kind": t.kind, "support": len(t.hits)} for t in ranked[:3]])]
    t = ranked[0]
    order = sorted(live, key=lambda j: j in t.hits)
    examples = [{"pair": pairs[j].name, "want": wants[j], "got": t.outs[j]} for j in order[:3]]
    kind = t.items.kind if t.kind == "path" else t.kind
    head = LeafFit(field, t.rule, kind, len(t.hits), len(live), t.items.holdout, examples, alternatives)
    return t.rule, [head, *(replace(f, field=f"{field}.{f.field}") for f in t.items.fits)]
