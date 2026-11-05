"""`canonic verify`: maps every pair through a spec and checks shape, values and rejects, so CI can
gate a provider. Rejects follow , not the design."""

from __future__ import annotations

import re
from collections.abc import Callable, Iterator
from dataclasses import dataclass, field
from typing import Any

from ... import Rejected, _api
from .compare import same
from .arrays import MISSING
from .fit import _empty, _eq, _nodes, _val
from .pairs import Pair


@dataclass
class VerifyResult:
    shape_failures: list[tuple[str, str, str]]  # (pair, path, message)
    fields: dict[str, tuple[int, int]]  # field key to (pairs that match, pairs mapped)
    rejected: list[str]
    ok: bool
    rejected_wrongly: list[str]  # rejects that throw away an answer equal to the default's
    misses: dict[str, list[str]] = field(default_factory=dict)  # field key to the first 5 pairs that miss
    unmapped: dict[str, int] = field(default_factory=dict)  # leaf the spec leaves out to pairs where the default fills it

    def to_json(self, pairs: int, skipped: list[tuple[str, str]]) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "pairs": pairs,
            "skipped": [{"pair": n, "reason": why} for n, why in skipped],
            "shape_failures": [{"pair": p, "path": at, "message": m} for p, at, m in self.shape_failures],
            "fields": {k: {"match": m, "total": t, "share": m / t if t else 1.0, "misses": self.misses.get(k, [])} for k, (m, t) in self.fields.items()},
            "unmapped": {k: {"pairs": n} for k, n in self.unmapped.items()},
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
    props = (_branch(node, want) or ("", {}))[1] if "oneOf" in node else node.get("properties") or {}
    return out + [f for k in common for f in shape(got[k], want[k], props.get(k) or {}, _at(path, k))]


def _branch(node: dict[str, Any], obj: Any) -> tuple[str, dict[str, Any]] | None:
    """(title, properties) of the union branch whose keys equal the object's, else the first branch."""
    bs = [b for b in node.get("oneOf") or [] if isinstance(b, dict)]
    hit = next((b for b in bs if isinstance(obj, dict) and set(b.get("properties") or {}) == set(obj)), bs[0] if bs else None)
    return None if hit is None else (str(hit.get("title", "")), hit.get("properties") or {})


def _leaf_rows(node: dict[str, Any], prefix: str) -> Iterator[str]:
    """Row keys for the leaves of an object node, nested lists as `a[].b[] (count)` and their leaves."""
    for name, sub in (node.get("properties") or {}).items():
        sub = sub if isinstance(sub, dict) else {}
        p = f"{prefix}.{name}"
        if sub.get("type") == "array":
            yield p + "[] (count)"
            items = sub.get("items") or {}
            if "properties" in items and not items.get("x-open"):
                yield from _leaf_rows(items, p + "[]")
            else:
                yield p + "[]"
        elif "properties" in sub and not sub.get("x-open"):
            yield from _leaf_rows(sub, p)
        else:
            yield p


def _covers(imap: Any, rel: str, defs: dict[str, Any]) -> bool:
    """Whether an item map maps the leaf `rel`, itself or through an ancestor that copies a whole value."""
    while isinstance(imap, dict) and isinstance(imap.get("$ref"), str):
        imap = defs.get(imap["$ref"])
    if not isinstance(imap, dict):
        return False
    parts = rel.split(".")
    for i in range(1, len(parts) + 1):
        r = imap.get(".".join(parts[:i]))
        if r is None:
            continue
        if i == len(parts) or isinstance(r, str) or "item" not in r:
            return True
        if parts[i - 1].endswith("[]"):
            return _covers(r["item"], ".".join(parts[i:]), defs)
    return False


def _item_rows(spec_fields: dict[str, Any], schema: dict[str, Any], defs: dict[str, Any]) -> tuple[dict[str, list[str]], list[str]]:
    """For each list the spec maps, the count row and the leaf rows of its items, and the
    item leaves the spec's item map leaves out, which are `unmapped` as at the top level."""
    nodes = dict(_nodes(schema))
    out, left = {}, []
    for k in spec_fields:
        if k.endswith("[]") and k != "[]" and k in nodes:
            items = nodes[k].get("items") or {}
            rule = spec_fields[k]
            imap = rule.get("item") if isinstance(rule, dict) else None
            rows = list(_leaf_rows(items, k)) if "properties" in items and not items.get("x-open") else []
            keep = [k + " (count)"]
            for r in rows:
                rel = r.removesuffix(" (count)")[len(k) + 1 :]
                if imap is None or _covers(imap, rel, defs):
                    keep.append(r)
                elif not r.endswith(" (count)"):
                    left.append(r)
            out[k] = keep
    return out, left


def _walk(g: Any, w: Any, key: str, idx: str = "") -> Iterator[tuple[Any, Any, str]]:
    """The (default-side, mapped-side, item index) values at `key`, pairing list items by position."""
    head, sep, rest = key.partition("[]")
    if not sep or not rest:
        yield _val(g, key), _val(w, key), idx
        return
    gl, wl = _val(g, head), _val(w, head)
    if isinstance(gl, list) and isinstance(wl, list):
        for i, (a, b) in enumerate(zip(gl, wl)):
            yield from _walk(a, b, rest.lstrip("."), f"{idx}.{i}" if idx else str(i))


def _under(key: str, ignored: set[str]) -> bool:
    return any(key == i or (key.startswith(i) and key[len(i)] in ".[ (") for i in ignored)


def _sorted(v: Any) -> Any:
    """`v` with object keys sorted at every depth: key order counts only in the shape check."""
    if isinstance(v, dict):
        return {k: _sorted(v[k]) for k in sorted(v)}
    return [_sorted(x) for x in v] if isinstance(v, list) else v


def _unmapped(spec_fields: dict[str, Any], schema: dict[str, Any]) -> list[str]:
    """The schema's leaves (arrays count as one) that no field key maps, by itself or through an
    ancestor that copies a whole value (a path or a `from` rule; `present` and `if_empty` do not)."""

    def copied(k: str) -> bool:
        r = spec_fields.get(k)
        return isinstance(r, str) or (isinstance(r, dict) and "from" in r)

    out = []
    for k, node in _nodes(schema):
        if ("properties" in node and not node.get("x-open")) or k in spec_fields:
            continue
        parts = k.split(".")
        if not any(copied(".".join(parts[:i])) for i in range(1, len(parts))):
            out.append(k)
    return out


def run(
    spec: dict[str, Any],
    schema: dict[str, Any],
    pairs: list[Pair],
    min_match: float,
    transforms: dict[str, Callable[..., Any]] | None,
    max_reject: float = 0.10,
    skipped: list[tuple[str, str]] | None = None,
    fail_unmapped: bool = False,
    ignore: list[str] | None = None,
) -> VerifyResult:
    """Maps every pair. A rejected pair counts in no field. It fails only if the spec without its
    reject rules maps it to exactly the default's output, or if over `max_reject` of the pairs are
    rejected. A field counts only pairs where either side has a value, and any
    `skipped` pair fails the run. A leaf the spec does not map but the default fills is
    listed, and fails the run only with `fail_unmapped`. A list also scores each leaf of its
    items by position and a count row; `ignore` drops a field and all below it."""
    art = _api.compile_spec(spec, schema, transforms=transforms)
    loose = _api.compile_spec({k: v for k, v in spec.items() if k != "reject"}, schema, transforms=transforms) if spec.get("reject") else None
    sf = spec.get("fields") or {}
    ignored = set(ignore or ())
    kept = {k: v for k, v in sf.items() if not _under(k, ignored)}
    keys = list(kept)
    item_rows, left = _item_rows(kept, schema, spec.get("definitions") or {})
    rows = {k: [r for r in rs if not _under(r, ignored)] for k, rs in item_rows.items()}

    def dropped(row: str) -> bool:  # a row whose descendant is ignored cannot compare its whole value
        return any(i != row and _under(i, {row}) for i in ignored)

    order = [r for k in keys for r in [k, *rows.get(k, [])] if not dropped(r)]
    unions = {k: n for k, n in _nodes(schema) if k in kept and "oneOf" in n}
    counts = {k: [0, 0] for k in order}
    misses: dict[str, list[str]] = {k: [] for k in order}
    unmapped = dict.fromkeys((k for k in [*_unmapped(sf, schema), *left] if not _under(k, ignored)), 0)
    failures: list[tuple[str, str, str]] = []
    rejected: list[str] = []
    wrongly: list[str] = []

    def score(row: str, got: Any, want: Any, tag: str) -> None:
        if dropped(row):
            return
        if row.endswith(" (count)"):
            if not (isinstance(got, list) or isinstance(want, list)):
                return
            hit = len(got if isinstance(got, list) else []) == len(want if isinstance(want, list) else [])
        elif (got is None or got is MISSING) and (want is None or want is MISSING):
            return  # both null: neither a match nor a miss
        else:
            hit = _eq(_sorted(got), _sorted(want))
        c = counts.setdefault(row, [0, 0])
        c[0] += hit
        c[1] += 1
        if not hit and len(misses.setdefault(row, [])) < 5:
            misses[row].append(tag)

    for p in pairs:
        try:
            out = art.map(p.provider, context=p.inputs)
        except Rejected:
            rejected.append(p.name)
            if loose is not None and same(loose.map(p.provider, context=p.inputs), p.default):
                wrongly.append(p.name)
            continue
        failures += [(p.name, at, msg) for at, msg in shape(out, p.default, schema) if not _under(re.sub(r"\[\d+\]", "[]", at), ignored)]
        for k in unmapped:
            unmapped[k] += not all(_empty(w) for _, w, _ in _walk(p.default, p.default, k))
        for k in keys:
            score(k, _val(out, k), _val(p.default, k), p.name)
            for row in rows.get(k, []):
                base = row.removesuffix(" (count)")
                for got, want, idx in _walk(out, p.default, base):
                    score(row, got, want, f"{p.name}#{idx}" if idx else p.name)
        for k, node in unions.items():
            b = _branch(node, _val(p.default, k))
            if b is not None and isinstance(_val(p.default, k), dict):
                for leaf in _leaf_rows({"properties": b[1]}, f"{k}({b[0]})"):
                    if not leaf.endswith(" (count)") and not _under(leaf, ignored):
                        src = leaf.replace(f"({b[0]})", "", 1)
                        score(leaf, _val(out, src), _val(p.default, src), p.name)
    fields = {k: (m, t) for k, (m, t) in counts.items()}
    misses = {k: v for k, v in misses.items() if v}
    unmapped = {k: n for k, n in unmapped.items() if n}
    share = len(rejected) / len(pairs) if pairs else 0.0
    ok = not skipped and not failures and not wrongly and share <= max_reject and all(t == 0 or m / t >= min_match for m, t in fields.values())
    ok = ok and not (fail_unmapped and unmapped)
    return VerifyResult(failures, fields, rejected, ok, wrongly, misses, unmapped)
