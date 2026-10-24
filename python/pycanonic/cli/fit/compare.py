"""The comparison rule shared by `fit`, `verify` and the conformance runners (conformance/README.md
states it, crates/canonic-core/tests/common/mod.rs has the Rust twin)."""

from __future__ import annotations

import math
from typing import Any

TWO_53 = 2.0**53


def fffd(s: str) -> str:
    """`s` with every lone surrogate replaced by U+FFFD; surrogate pairs decode to their character."""
    return s.encode("utf-16", "surrogatepass").decode("utf-16", "replace")


def _num_eq(a: float, b: float) -> bool:
    """By value; when either side is at least 2^53 in magnitude, integer or not, within one double
    step of `a` (TypeScript writes a rounded big integer, pycanonic keeps it exact)."""
    if max(abs(a), abs(b)) >= TWO_53:
        return abs(float(a) - float(b)) <= math.ulp(float(a))
    return a == b


def diff(got: Any, want: Any, path: str = "") -> str | None:
    """The first difference between `got` and `want` as `path: got != want`, or `None` when equal."""
    differ = f"{path}: got {got!r}, want {want!r}"
    if isinstance(got, bool) or isinstance(want, bool):
        return None if type(got) is type(want) and got == want else differ
    if isinstance(got, (int, float)) and isinstance(want, (int, float)):
        return None if _num_eq(got, want) else differ
    if isinstance(got, str) and isinstance(want, str):
        return None if fffd(got) == fffd(want) else differ
    if isinstance(got, dict) and isinstance(want, dict):
        ka, kb = [fffd(k) for k in got], [fffd(k) for k in want]
        if ka != kb:
            return f"{path}: keys {ka}, want {kb}"
        for (k, x), y in zip(got.items(), want.values()):
            if d := diff(x, y, f"{path}.{k}"):
                return d
        return None
    if isinstance(got, list) and isinstance(want, list) and len(got) == len(want):
        for i, (x, y) in enumerate(zip(got, want)):
            if d := diff(x, y, f"{path}[{i}]"):
                return d
        return None
    return None if got is None and want is None else differ


def same(a: Any, b: Any) -> bool:
    """Numbers by value, objects key by key in order, strings with lone surrogates as U+FFFD."""
    return diff(a, b) is None
