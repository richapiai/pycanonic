"""Source fields: every path that appears in the sample responses, with its value in each sample."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from typing import Any, Literal, Union

from ..io import is_obj, js_keys

Seg = Union[str, int]
Kind = Literal["scalar", "objects", "scalars"]


@dataclass
class Source:
    path: str  # in the spec path grammar
    segs: list[Seg]
    kind: Kind
    values: list[Any]
    """One entry per sample; None where the path is missing. Lists for the two array kinds."""


IDENT = re.compile(r"[A-Za-z_$][A-Za-z0-9_$-]*")
MAX_SOURCES = 5000


def format_path(segs: list[Seg]) -> str:
    """The empty path is `$`, the response itself, so a first key that is literally $ has to be quoted."""
    if not segs:
        return "$"
    return "".join(
        f"[{s}]"
        if isinstance(s, int)
        else ((f".{s}" if i else s) if IDENT.fullmatch(s) and not (i == 0 and s == "$") else f"[{json.dumps(s, ensure_ascii=False)}]")
        for i, s in enumerate(segs)
    )


def array_kind(v: list[Any]) -> Kind | None:
    present = [x for x in v if x is not None]
    if not present:
        return None
    if all(is_obj(x) for x in present):
        return "objects"
    return None if any(isinstance(x, (dict, list)) for x in present) else "scalars"


class Collector:
    def __init__(self, n: int) -> None:
        self.n = n
        self.by_path: dict[str, Source] = {}

    def add(self, segs: list[Seg], kind: Kind, sample: int, value: Any) -> None:
        path = format_path(segs)
        s = self.by_path.get(path)
        if s is None:
            if len(self.by_path) >= MAX_SOURCES:
                return
            s = Source(path, segs, kind, [None] * self.n)
            self.by_path[path] = s
        if s.kind == kind:
            s.values[sample] = value

    def walk(self, v: dict[str, Any], segs: list[Seg], sample: int) -> None:
        for k in js_keys(v):
            x = v[k]
            at = [*segs, k]
            if is_obj(x):
                self.walk(x, at, sample)
            elif isinstance(x, list):
                self.array(x, at, sample)
            elif x is not None:
                self.add(at, "scalar", sample, x)

    # Arrays are also read at [0], which is how a spec picks "the first email" or "the first phone".
    def array(self, x: list[Any], at: list[Seg], sample: int) -> None:
        kind = array_kind(x)
        if not kind:
            return
        self.add(at, kind, sample, x)
        if kind == "objects" and is_obj(x[0]):
            self.walk(x[0], [*at, 0], sample)
        if kind == "scalars" and x[0] is not None:
            self.add([*at, 0], "scalar", sample, x[0])


def collect(samples: list[Any]) -> list[Source]:
    """All source fields across the samples, in first-seen order."""
    c = Collector(len(samples))
    for i, s in enumerate(samples):
        if is_obj(s):
            c.walk(s, [], i)
        elif isinstance(s, list):
            c.array(s, [], i)  # a response that is itself an array: the source `$`
    return list(c.by_path.values())


def elements(s: Source) -> list[Any]:
    """Every element of an objects-array source, across all samples: the samples for inferring its item."""
    return [x for v in s.values if isinstance(v, list) for x in v if is_obj(x)]


def fill(s: Source) -> float:
    """Share of samples where the source has a value."""
    return len([v for v in s.values if v is not None]) / len(s.values)
