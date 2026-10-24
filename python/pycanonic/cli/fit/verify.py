"""`canonic verify`: maps every pair through a spec and checks shape, values and rejects, so CI can
gate a provider. Rejects follow , not the design."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from ... import Rejected, _api
from .compare import same
from .arrays import MISSING
from .fit import _eq, _val
from .pairs import Pair


@dataclass
class VerifyResult:
    shape_failures: list[tuple[str, str, str]]  # (pair, path, message)
    fields: dict[str, tuple[int, int]]  # field key to (pairs that match, pairs mapped)
    rejected: list[str]
    ok: bool
    rejected_wrongly: list[str]  # rejects that throw away an answer equal to the default's

    def to_json(self, pairs: int, skipped: list[tuple[str, str]]) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "pairs": pairs,
            "skipped": [{"pair": n, "reason": why} for n, why in skipped],
            "shape_failures": [{"pair": p, "path": at, "message": m} for p, at, m in self.shape_failures],
            "fields": {k: {"match": m, "total": t, "share": m / t if t else 1.0} for k, (m, t) in self.fields.items()},
            "rejected": self.rejected,
            "rejected_wrongly": self.rejected_wrongly,
            "reject_share": len(self.rejected) / pairs if pairs else 0.0,
        }


def _kind(v: Any) -> str:
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, (int, float)):
        return "number"
    return "string" if isinstance(v, str) else "array" if isinstance(v, list) else "object"


def _at(path: str, key: str | int) -> str:
    step = f"[{key}]" if isinstance(key, int) else key if path == "$" else f".{key}"
    return step if path == "$" else path + step


def shape(got: Any, want: Any, node: dict[str, Any], path: str = "$") -> list[tuple[str, str]]:
    """Where the mapped output `got` has another shape than the default's `want`: keys and their
    order, JSON types, and null against an object or array, in either direction. Null
    against a scalar is a value miss, which the value check counts (R35). An `x-open` node may
    differ in keys."""
    kg, kw = _kind(got), _kind(want)
    if kg != kw:
        if "null" in (kg, kw):
            other = kw if kg == "null" else kg
            return [(path, f"null against {other}")] if other in ("object", "array") else []
        return [(path, f"type {kg} against {kw}")]
    if isinstance(got, list):
        item = node.get("items") or {}
        return [f for i, (g, w) in enumerate(zip(got, want)) for f in shape(g, w, item, _at(path, i))]
    if not isinstance(got, dict) or node.get("x-open"):
        return []
    out = [(_at(path, k), "missing key") for k in want if k not in got] + [(_at(path, k), "extra key") for k in got if k not in want]
    common = [k for k in want if k in got]
    if [k for k in got if k in want] != common:
        out.append((path, "key order"))
    props = node.get("properties") or {}
    return out + [f for k in common for f in shape(got[k], want[k], props.get(k) or {}, _at(path, k))]


def _sorted(v: Any) -> Any:
    """`v` with object keys sorted at every depth: key order counts only in the shape check."""
    if isinstance(v, dict):
        return {k: _sorted(v[k]) for k in sorted(v)}
    return [_sorted(x) for x in v] if isinstance(v, list) else v


def run(
    spec: dict[str, Any],
    schema: dict[str, Any],
    pairs: list[Pair],
    min_match: float,
    transforms: dict[str, Callable[..., Any]] | None,
    max_reject: float = 0.10,
    skipped: list[tuple[str, str]] | None = None,
) -> VerifyResult:
    """Maps every pair. A rejected pair counts in no field. It fails only if the spec without its
    reject rules maps it to exactly the default's output, or if over `max_reject` of the pairs are
    rejected. A field counts only pairs where either side has a value, and any
    `skipped` pair fails the run."""
    art = _api.compile_spec(spec, schema, transforms=transforms)
    loose = _api.compile_spec({k: v for k, v in spec.items() if k != "reject"}, schema, transforms=transforms) if spec.get("reject") else None
    keys = list(spec.get("fields") or {})
    counts = {k: [0, 0] for k in keys}
    failures: list[tuple[str, str, str]] = []
    rejected: list[str] = []
    wrongly: list[str] = []
    for p in pairs:
        try:
            out = art.map(p.provider, context=p.inputs)
        except Rejected:
            rejected.append(p.name)
            if loose is not None and same(loose.map(p.provider, context=p.inputs), p.default):
                wrongly.append(p.name)
            continue
        failures += [(p.name, at, msg) for at, msg in shape(out, p.default, schema)]
        for k in keys:
            got, want = _val(out, k), _val(p.default, k)
            if (got is None or got is MISSING) and (want is None or want is MISSING):
                continue  # both null: neither a match nor a miss
            counts[k][0] += _eq(_sorted(got), _sorted(want))
            counts[k][1] += 1
    fields = {k: (m, t) for k, (m, t) in counts.items()}
    share = len(rejected) / len(pairs) if pairs else 0.0
    ok = not skipped and not failures and not wrongly and share <= max_reject and all(t == 0 or m / t >= min_match for m, t in fields.values())
    return VerifyResult(failures, fields, rejected, ok, wrongly)
