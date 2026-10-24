"""Every scalar in a provider body, by spec path with concrete array positions (`positions[2].title`)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ..infer.sources import Seg, format_path


@dataclass(frozen=True)
class Occurrence:
    path: str
    value: Any


def _walk(v: Any, segs: list[Seg], out: list[Occurrence]) -> None:
    if isinstance(v, dict):
        for k, x in v.items():
            _walk(x, [*segs, k], out)
    elif isinstance(v, list):
        for i, x in enumerate(v):
            _walk(x, [*segs, i], out)
    elif v is not None:
        out.append(Occurrence(format_path(segs), v))


def scalars(body: Any) -> list[Occurrence]:
    """Non-null scalars in document order. A bare-array body reads from `[0]`, a scalar body is `$`."""
    out: list[Occurrence] = []
    _walk(body, [], out)
    return out
