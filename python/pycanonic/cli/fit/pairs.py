"""Sample pairs: the default provider's body and the new provider's body for one request, by file name."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class Pair:
    name: str
    default: Any
    provider: Any
    inputs: dict[str, Any] | None


def _files(d: Path) -> dict[str, Path]:
    return {p.name: p for p in d.iterdir() if p.is_file() and p.name.endswith(".json")}


def _body(path: Path) -> tuple[Any, str | None]:
    """The parsed body, or the reason it is not usable. A bare array is a body (the `$` root)."""
    try:
        v = json.loads(path.read_bytes())
    except (ValueError, RecursionError):
        return None, "not JSON"
    return (v, None) if isinstance(v, (dict, list)) else (None, "not an object or array")


def _inputs(path: Path | None) -> tuple[dict[str, Any] | None, str | None]:
    if path is None:
        return None, None
    v, why = _body(path)
    if why == "not JSON":
        return None, "inputs not JSON"
    return (v, None) if isinstance(v, dict) else (None, "inputs not an object")


def load_pairs(default_dir: Path, provider_dir: Path, inputs_dir: Path | None) -> tuple[list[Pair], list[tuple[str, str]]]:
    """The usable pairs and the skipped `(name, reason)` list, both sorted by name. A pair with no
    inputs file has `inputs` None; an inputs file that is not a JSON object skips the pair."""
    dflt, prov = _files(default_dir), _files(provider_dir)
    inputs = _files(inputs_dir) if inputs_dir else {}
    pairs: list[Pair] = []
    skipped: list[tuple[str, str]] = []
    for name in sorted(dflt.keys() | prov.keys()):
        if name not in prov:
            skipped.append((name, "only in default"))
            continue
        if name not in dflt:
            skipped.append((name, "only in provider"))
            continue
        (d, why_d), (p, why_p) = _body(dflt[name]), _body(prov[name])
        ctx, why_i = _inputs(inputs.get(name))
        if why := why_d or why_p or why_i:
            skipped.append((name, why))
            continue
        pairs.append(Pair(name, d, p, ctx))
    return pairs, skipped
