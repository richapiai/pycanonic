"""`--align-items`: pairs the elements of a list in both bodies by id, so fit and verify see one
pair per item instead of one per response."""

from __future__ import annotations

import argparse
import re
from dataclasses import dataclass
from typing import Any

from .arrays import MISSING, get
from .leaves import _segs
from .pairs import Pair


@dataclass
class AlignCount:
    pair: str
    matched: int
    default_only: int
    provider_only: int


def add_flags(p: argparse.ArgumentParser) -> None:
    p.add_argument("--align-items", nargs=2, metavar=("DEFAULT_LIST", "PROVIDER_LIST"))
    p.add_argument("--by", nargs=2, metavar=("DEFAULT_ID", "PROVIDER_ID"))
    p.add_argument("--id-pattern")


def _list(path: str) -> list[str | int]:
    return _segs(path.removesuffix("[]"))


def _key(item: Any, segs: list[str | int], pattern: re.Pattern[str] | None) -> str | None:
    v = get(item, segs)
    if v is MISSING or isinstance(v, bool) or not isinstance(v, (str, int, float)):
        return None
    text = str(v)
    if pattern is None:
        return re.sub(r"\D", "", text) or None
    m = pattern.search(text)
    got = (m.group(1) if pattern.groups else m.group(0)) if m else None
    return got or None


def _index(body: Any, lst: list[str | int], idp: list[str | int], pattern: re.Pattern[str] | None) -> tuple[list[Any], dict[str, Any]]:
    """The list's length-bearing elements, and the first element for each normalized id."""
    items = get(body, lst)
    if items is MISSING or not isinstance(items, list):
        return [], {}
    first: dict[str, Any] = {}
    for it in items:
        k = _key(it, idp, pattern)
        if k is not None:
            first.setdefault(k, it)
    return items, first


def align(pairs: list[Pair], default_list: str, provider_list: str, default_id: str, provider_id: str, pattern: str | None) -> tuple[list[Pair], list[AlignCount]]:
    """One pair `<pair>#<n>` per id found on both sides, in the default list's order, with the
    parent's inputs. A repeated id pairs its first element only; the others count as unmatched."""
    try:
        rx = re.compile(pattern) if pattern is not None else None
    except re.error as e:
        raise ValueError(f"--id-pattern {pattern!r}: {e}") from None
    dl, pl, di, pi = _list(default_list), _list(provider_list), _segs(default_id), _segs(provider_id)
    out: list[Pair] = []
    counts: list[AlignCount] = []
    for p in pairs:
        d_items, d_first = _index(p.default, dl, di, rx)
        p_items, p_first = _index(p.provider, pl, pi, rx)
        n = 0
        for k, d in d_first.items():
            if k in p_first:
                out.append(Pair(f"{p.name}#{n}", d, p_first[k], p.inputs))
                n += 1
        counts.append(AlignCount(p.name, n, len(d_items) - n, len(p_items) - n))
    return out, counts


def item_schema(schema: dict[str, Any], default_list: str) -> dict[str, Any]:
    """The schema node of the list's items. Exits 2 (ValueError) unless the path names an array of objects."""
    node: Any = schema
    for s in _list(default_list):
        if isinstance(s, int):
            node = node.get("items") if isinstance(node, dict) and node.get("type") == "array" else None
        else:
            node = (node.get("properties") or {}).get(s) if isinstance(node, dict) else None
    items = node.get("items") if isinstance(node, dict) and node.get("type") == "array" else None
    if not (isinstance(items, dict) and items.get("type") == "object"):
        raise ValueError(f"--align-items {default_list}: expected a path to an array of objects in the schema, such as data.items")
    return {**items, "x-shape": schema["x-shape"]} if "x-shape" in schema else items


def from_args(a: argparse.Namespace, pairs: list[Pair], schema: dict[str, Any]) -> tuple[list[Pair], dict[str, Any], list[AlignCount]]:
    """The pairs and schema as given when `--align-items` is off, else the item pairs and item schema."""
    if not a.align_items:
        if a.by or a.id_pattern:
            raise ValueError("--by and --id-pattern need --align-items")
        return pairs, schema, []
    if not a.by:
        raise ValueError("--align-items needs --by DEFAULT_ID PROVIDER_ID, such as --by id id")
    sch = item_schema(schema, a.align_items[0])
    items, counts = align(pairs, a.align_items[0], a.align_items[1], a.by[0], a.by[1], a.id_pattern)
    return items, sch, counts


def lines(counts: list[AlignCount]) -> list[str]:
    return [f"aligned  {c.pair}  {c.matched} matched, {c.default_only} default only, {c.provider_only} provider only" for c in counts]


def to_json(counts: list[AlignCount]) -> list[dict[str, Any]]:
    return [{"pair": c.pair, "matched": c.matched, "default_only": c.default_only, "provider_only": c.provider_only} for c in counts]
