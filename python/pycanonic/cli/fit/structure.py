"""Structure rules: optional objects, constants, conditions on
the request inputs and proposed reject rules. Each candidate is checked through the
runtime before it is returned."""

from __future__ import annotations

from typing import Any

from ... import Rejected, _api
from ..infer.sources import Seg, format_path
from .arrays import MISSING, at, get, name
from .chains import apply_rule
from .compare import same
from .index import scalars
from .leaves import _segs, fit_leaf
from .pairs import Pair

_OBJ: dict[str, Any] = {"type": "object", "properties": {"x": {}}}
# A provider that leaves a leaf out in up to half the pairs still gets its path found; a
# provider that misses it more often gets no proposal.
_REJECT_FLOOR = 0.5


def _paths(body: Any, segs: list[Seg] | None = None) -> list[str]:
    """Every path outside arrays, objects included, in document order."""
    segs = segs or []
    out: list[str] = []
    if isinstance(body, dict):
        for k, v in body.items():
            out.append(format_path([*segs, k]))
            out += _paths(v, [*segs, k])
    return out


def fit_object_presence(node_path: str, pairs: list[Pair]) -> dict[str, Any] | None:
    """`{"present": p}` for a default object that is null or missing in exactly the pairs where the
    provider path `p` is missing or null. None when the object is always or never there."""
    absent = [at(p.default, node_path) is None for p in pairs]
    if all(absent) or not any(absent):
        return None
    ok: list[str] = []
    for path in dict.fromkeys(x for p in pairs for x in _paths(p.provider)):
        segs = _segs(path)
        if all(((v := get(p.provider, segs)) is MISSING or v is None) == a for p, a in zip(pairs, absent)):
            if all((apply_rule({"present": path}, p.provider, _OBJ) is None) == a for p, a in zip(pairs, absent)):
                ok.append(path)
    if not ok:
        return None
    return {"present": min(ok, key=lambda p: (-name(node_path, p), len(_segs(p)), ok.index(p)))}


def fit_const(field: str, targets: list[Any]) -> dict[str, Any] | None:
    """`{"const": v}` when the default's value is the same non-null `v` in every pair. The caller
    tries the source paths first, so a value that a provider path also gives is a path, not a const."""
    if not targets or targets[0] is None or not all(same(t, targets[0]) for t in targets):
        return None
    rule = {"const": targets[0]}
    return rule if same(apply_rule(rule, {}), targets[0]) else None


def fit_when(field: str, pairs: list[Pair]) -> dict[str, Any] | None:
    """`{"when": cond}` for a field that the default has (not null) only in pairs whose inputs meet
    `cond`, plus `"omit": true` when the default leaves the key out in the other pairs. `cond` is an
    `eq` on one input path, else `present: true`. The caller adds the source rule."""
    vals = [get(p.default, _segs(field)) for p in pairs]
    has = [v is not MISSING and v is not None for v in vals]
    if all(has) or not any(has):
        return None
    first = next(p.inputs or {} for p, h in zip(pairs, has) if h)
    paths = list(dict.fromkeys(o.path for p in pairs for o in scalars(p.inputs or {}) if o.path != "$"))
    conds: list[dict[str, Any]] = []
    for path in paths:
        if (v := get(first, _segs(path))) is not MISSING:
            conds.append({"path": "$input" + ("" if path.startswith("[") else ".") + path, "eq": v})
    conds += [{"path": c["path"], "present": True} for c in conds]
    for cond in conds:
        rule = {"const": 1, "when": cond}
        if all((apply_rule(rule, {}, None, p.inputs) is not None) == h for p, h in zip(pairs, has)):
            omit = all(v is MISSING for v, h in zip(vals, has) if not h)
            return {"when": cond, **({"omit": True} if omit else {})}
    return None


def _leaves(node: dict[str, Any], segs: list[Seg]) -> list[str]:
    """Leaf field keys outside arrays."""
    out: list[str] = []
    for k, sub in (node.get("properties") or {}).items():
        sub = sub if isinstance(sub, dict) else {}
        if sub.get("type") == "array":
            continue
        out += _leaves(sub, [*segs, k]) if "properties" in sub else [format_path([*segs, k])]
    return out


def _rejects(cond: dict[str, Any], body: Any) -> bool:
    spec = {"language": 2, "provider": "fit", "endpoint": "reject", "version": 1, "status": "draft", "reject": [cond], "fields": {"v": {"const": 1}}}
    try:
        _api.compile_spec(spec, {"type": "object", "properties": {"v": {}}}).map(body)
    except Rejected:
        return True
    return False


def propose_reject(schema: dict[str, Any], pairs: list[Pair]) -> list[dict[str, Any]]:
    """`{"missing": p}` for each leaf outside arrays that is never null in the default and whose
    source path `p` the provider leaves out in some pairs. Always for review.

    A reject is a failover for a body with no usable data, so a condition stays only if each pair
    it rejects lacks at least half of the fitted sources (an error body, not a profile without a
    bio). Conditions that reject the same pairs are one cause: the first in schema order stays."""
    conds: list[dict[str, Any]] = []
    for key in _leaves(schema, []):
        targets = [at(p.default, key) for p in pairs]
        if any(t is None for t in targets):
            continue
        f = fit_leaf(key, pairs, targets, [scalars(p.provider) for p in pairs], False, _REJECT_FLOOR)
        src = f.rule.get("from") if f.rule else None
        if not isinstance(src, str) or src == "$":
            continue
        cond = {"missing": src}
        if cond not in conds:
            conds.append(cond)

    def lacks_most(body: Any) -> bool:
        return 2 * sum(_rejects(c, body) for c in conds) >= len(conds)

    out: list[dict[str, Any]] = []
    seen: set[frozenset[int]] = set()
    for cond in conds:
        hit = frozenset(i for i, p in enumerate(pairs) if _rejects(cond, p.provider))
        if hit and hit not in seen and all(lacks_most(pairs[i].provider) for i in hit):
            seen.add(hit)
            out.append(cond)
    return out
