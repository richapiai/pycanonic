"""Compiling, loading and validating specs: the functions of `canonic`'s API in snake_case."""

from __future__ import annotations

import os
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from functools import partial
from pathlib import Path
from typing import Any, Literal

from . import _core
from ._core import Artifact

Transform = Callable[..., Any]
"""`f(value)` or `f(value, arg)`; returns the new value, or `None` to reject it."""


class SpecError(ValueError):
    """A spec or canonical schema `compile_spec` rejects; the message names the field."""


@dataclass(frozen=True)
class Issue:
    """One grammar problem: where it is (`spec.fields["email"]`) and what is wrong."""

    at: str
    message: str


def compile_spec(
    spec: dict[str, Any] | str | bytes,
    canonical: dict[str, Any] | str | bytes,
    *,
    transforms: dict[str, Transform] | None = None,
) -> Artifact:
    """Compile `spec` against the `canonical` schema (each a `dict` or JSON text).

    `transforms` adds your own transforms by name; a name that matches a default replaces it.
    Raises `SpecError` (a `ValueError`) naming the field when the spec or the schema is wrong.
    """
    try:
        return _core.compile(spec, canonical, transforms)
    except ValueError as e:
        raise SpecError(str(e)) from None


def load(
    path_or_json: str | bytes | os.PathLike[str],
    *,
    transforms: dict[str, Transform] | None = None,
) -> Artifact:
    """An artifact from a plan written by `Artifact.plan_json()`, without compiling the spec again.

    An `os.PathLike` is a file path, read as UTF-8. A `str` naming an existing file is read the
    same way; any other `str`, and `bytes`, is the plan JSON itself. Pass the same `transforms`
    the plan was compiled with.
    """
    if isinstance(path_or_json, os.PathLike) or (
        isinstance(path_or_json, str) and os.path.isfile(path_or_json)
    ):
        text = Path(path_or_json).read_text(encoding="utf-8")
    elif isinstance(path_or_json, bytes):
        text = path_or_json.decode("utf-8")
    else:
        text = path_or_json
    return _core.load_plan(text, transforms)


def validate_spec(spec: dict[str, Any] | str | bytes) -> list[Issue]:
    """Every grammar problem in `spec`, without a schema and without compiling; empty when fine."""
    return [Issue(at, message) for at, message in _core.validate_spec(spec)]


def validate_canonical(schema: dict[str, Any] | str | bytes) -> list[Issue]:
    """What `compile_spec` would reject in a canonical schema, without a spec; empty when it loads."""
    return [Issue(at, message) for at, message in _core.validate_canonical(schema)]


_DEFAULT_NAMES = (
    "cast",
    "lower",
    "trim",
    "split_name",
    "join",
    "parse_date",
    "normalize_phone",
    "normalize_linkedin",
    "to_domain",
)
_SEMANTICS = ("email", "phone", "domain", "linkedin_url", "url", "iso_date", "money")

default_transforms: dict[str, Transform] = {
    name: partial(_core.default_transform, name) for name in _DEFAULT_NAMES
}
"""The built-in transforms by name: `default_transforms["lower"]("ÀDA") == "àda"`."""

validators: dict[str, Transform] = {s: partial(_core.validator, s) for s in _SEMANTICS}
"""The routing validators by semantic: `validators["email"](v)` returns `v` or `None`."""


@dataclass(frozen=True)
class FieldAlert:
    """A field whose fill fell (`kind="fill"`) or whose errors rose (`kind="errors"`).

    `responses` is what `rate` is out of: responses, or elements for a field inside an item
    (`"jobs[].title"`).
    """

    field: str
    kind: Literal["fill", "errors"]
    rate: float
    baseline: float
    responses: int


class FillMonitor:
    """Compares each `stats.snapshot()` window (then `reset()`, once per period) with the trailing windows.

    A window under `min_responses` is merged with the next. A field that alerts keeps its old
    baseline, so the alert repeats every window until the spec is fixed.
    """

    def __init__(
        self,
        *,
        min_responses: int = 200,
        history: int = 24,
        sigmas: float = 4.0,
        min_drop: float = 0.25,
        min_rise: float = 0.01,
    ) -> None:
        self._core = _core.FillMonitor(
            min_responses=min_responses,
            history=history,
            sigmas=sigmas,
            min_drop=min_drop,
            min_rise=min_rise,
        )

    def check(self, window: Mapping[str, Any]) -> list[FieldAlert]:
        """Feeds one window; the fields whose fill fell or whose errors rose, once it is big enough."""
        return [FieldAlert(**a) for a in self._core.check(window)]
