"""`canonic schema`: a language 2 canonical schema derived from the default provider's samples.

Each node records its types, its key order (first appearance), and the form an empty value takes in
the samples (`x-empty`), because `fit` and `verify` reproduce that form."""

from __future__ import annotations

import json
import re
from collections import Counter
from typing import Any

# Preference when two empty forms are seen equally often.
_EMPTY_ORDER = ["null", "[]", "{}", "missing"]
_ALLOWED = {"object": {"null", "{}", "missing"}, "array": {"null", "[]", "missing"}, "leaf": {"null", "missing"}}
_IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")


def _type(v: Any) -> str:
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, (int, float)):
        return "number"
    if isinstance(v, str):
        return "string"
    return "array" if isinstance(v, list) else "object"


def _empty(v: Any) -> str | None:
    if v is None:
        return "null"
    if isinstance(v, list) and not v:
        return "[]"
    if isinstance(v, dict) and not v:
        return "{}"
    return None


def _is_open(dicts: list[dict[str, Any]], keys: list[str]) -> bool:
    """A map of dynamic keys: over 20 distinct keys, and a sample carries under half of them on average."""
    return len(keys) > 20 and sum(len(d) for d in dicts) / len(dicts) < len(keys) / 2


def _node(values: list[Any], absent: int) -> dict[str, Any]:
    """The schema node for the values seen at one place; `absent` counts parents that lack the key."""
    types = sorted({_type(v) for v in values if v is not None})
    out: dict[str, Any] = {}
    if types:
        out["type"] = types[0] if len(types) == 1 else types
    kind = "object" if "object" in types else "array" if "array" in types else "leaf"
    if kind == "object":
        dicts = [v for v in values if isinstance(v, dict) and v]
        keys = list(dict.fromkeys(k for d in dicts for k in d))
        if dicts and _is_open(dicts, keys):
            out["x-open"] = True
        else:
            out["properties"] = {
                k: _node([d[k] for d in dicts if k in d], sum(k not in d for d in dicts)) for k in keys
            }
    elif kind == "array":
        elements = [x for v in values if isinstance(v, list) for x in v]
        if elements:
            out["items"] = _node(elements, 0)
    seen = Counter(e for e in map(_empty, values) if e is not None)
    if absent:
        seen["missing"] = absent
    fits = [e for e in _EMPTY_ORDER if e in seen and e in _ALLOWED[kind]]
    if fits:
        out["x-empty"] = max(fits, key=lambda e: seen[e])  # first of the most frequent
    if len(seen) > 1:
        out["x-mixed-empty"] = True
    return out


def derive_schema(samples: list[Any]) -> dict[str, Any]:
    """The schema for the samples (objects, or bare arrays as the `$` root with `"type": "array"`)."""
    return _node(samples, 0)


def _child(path: str, key: str) -> str:
    step = key if _IDENT.match(key) else f"[{json.dumps(key, ensure_ascii=False)}]"
    return step if not path else f"{path}{step}" if step.startswith("[") else f"{path}.{step}"


def flags(schema: dict[str, Any], path: str = "") -> list[tuple[str, str]]:
    """The fields that need a look, in schema order: `mixed types`, `mixed empty`, `always null`."""
    at = path or "$"
    out = [(at, "mixed types")] if isinstance(schema.get("type"), list) else []
    if schema.get("x-mixed-empty"):
        out.append((at, "mixed empty"))
    if "type" not in schema:
        out.append((at, "always null"))
    for k, child in schema.get("properties", {}).items():
        out += flags(child, _child(path, k))
    if "items" in schema:
        out += flags(schema["items"], f"{path}[]")
    return out
