"""The comparison rule shared by `fit`, `verify` and the conformance runners (conformance/README.md
states it, crates/canonic-core/tests/common/mod.rs has the Rust twin)."""

from __future__ import annotations

import math
import re
from dataclasses import dataclass
from typing import Any, Literal

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


@dataclass(frozen=True)
class Tolerance:
    """Opt-in leniency for live data. The default is strict: `match` then equals `same`."""

    num: float = 0.0  # relative: abs(a - b) <= num * max(abs(a), abs(b))
    url_query: bool = False  # two scheme:// URLs equal without the part from `?` to before `#`
    space: bool = False  # runs of white space collapse to one space, and the ends are trimmed

    def flags(self) -> list[str]:
        """The command-line flags this tolerance stands for."""
        return [*([f"--num-tolerance {self.num:g}"] if self.num else []), *(["--url-ignore-query"] if self.url_query else []), *(["--text-ignore-space"] if self.space else [])]


_URL = re.compile(r"[A-Za-z][A-Za-z0-9+.-]*://")


def _text(s: str, tol: Tolerance) -> str:
    s = fffd(s)
    if tol.space:
        s = " ".join(s.split())
    return re.sub(r"\?[^#]*", "", s, count=1) if tol.url_query and _URL.match(s) else s  # the fragment stays


def match(a: Any, b: Any, tol: Tolerance) -> Literal["exact", "tolerated", "miss"]:
    """`exact` when `same` holds, `tolerated` when the pair is equal only within `tol`, else `miss`.
    Booleans never match within a number tolerance. Containers recurse: tolerated if a part needed
    tolerance, miss if a part misses. Two strings equal after `tol` normalization differ in the
    query only when both are URLs, so a text that merely contains `?` is not cut."""
    if same(a, b):
        return "exact"
    if tol == Tolerance() or isinstance(a, bool) or isinstance(b, bool):
        return "miss"
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return "tolerated" if tol.num and abs(a - b) <= tol.num * max(abs(a), abs(b)) else "miss"
    if isinstance(a, str) and isinstance(b, str):
        both = _URL.match(fffd(a)) and _URL.match(fffd(b))
        t = tol if both else Tolerance(space=tol.space)
        return "tolerated" if _text(a, t) == _text(b, t) else "miss"
    if isinstance(a, dict) and isinstance(b, dict):
        if [fffd(k) for k in a] != [fffd(k) for k in b]:
            return "miss"
        parts = [match(x, y, tol) for x, y in zip(a.values(), b.values())]
    elif isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
        parts = [match(x, y, tol) for x, y in zip(a, b)]
    else:
        return "miss"
    return "tolerated" if "miss" not in parts and "tolerated" in parts else "miss"


def used(a: Any, b: Any, tol: Tolerance) -> list[str]:
    """The flags that made `a` and `b` a tolerated pair: the single flags that suffice, else all of them."""
    singles = [Tolerance(num=tol.num), Tolerance(url_query=tol.url_query), Tolerance(space=tol.space)]
    hit = [t.flags()[0] for t in singles if t != Tolerance() and match(a, b, t) == "tolerated"]
    return hit or tol.flags()


NEAR_CUT = 20  # the shorter text of a cut-short pair has at least this many characters


def near(a: Any, b: Any) -> bool:
    """Two different texts that are the same after trim and white-space collapse, or where the
    shorter, of at least `NEAR_CUT` characters, is a strict prefix of the other."""
    if not (isinstance(a, str) and isinstance(b, str)) or same(a, b):
        return False
    x, y = " ".join(fffd(a).split()), " ".join(fffd(b).split())
    if x == y:
        return True
    lo, hi = sorted((x, y), key=len)
    return len(lo) >= NEAR_CUT and hi.startswith(lo)
