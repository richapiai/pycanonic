"""Compiling, loading and validating specs: the functions of `canonic`'s API in snake_case."""

from __future__ import annotations

import os
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from functools import partial
from pathlib import Path
from typing import Any, Literal

from . import _core
from ._core import Artifact

Transform = Callable[..., Any]
"""`f(value)` or `f(value, arg)`. It returns the new value, or `None` to reject the value."""


class SpecError(ValueError):
    """A spec or canonical schema that `compile_spec` rejects. The message names the field."""


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

    `transforms` adds your own transforms by name. A name that matches a default replaces it.
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
    same way. A `str` that starts with `{` or `[`, and `bytes`, are the plan JSON itself. Any other
    `str` is a missing file: `FileNotFoundError`. Pass the same `transforms` the plan was
    compiled with.
    """
    if isinstance(path_or_json, os.PathLike) or (
        isinstance(path_or_json, str) and os.path.isfile(path_or_json)
    ):
        text = Path(path_or_json).read_text(encoding="utf-8")
    elif isinstance(path_or_json, str) and not path_or_json.lstrip().startswith(("{", "[")):
        raise FileNotFoundError(path_or_json)
    elif isinstance(path_or_json, bytes):
        text = path_or_json.decode("utf-8")
    else:
        text = path_or_json
    return _core.load_plan(text, transforms)


def validate_spec(spec: dict[str, Any] | str | bytes) -> list[Issue]:
    """Every grammar problem in `spec`, found without a schema and without compiling. Empty when there is none."""
    return [Issue(at, message) for at, message in _core.validate_spec(spec)]


def validate_canonical(schema: dict[str, Any] | str | bytes) -> list[Issue]:
    """What `compile_spec` rejects in a canonical schema, checked without a spec. Empty when the schema loads."""
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
    # Language 2 only in specs. Callable here for any caller.
    "regex",
    "strip_prefix",
    "strip_suffix",
    "replace",
    "lookup",
    "null_if",
    "upper",
    "title",
    "split",
    "to_string",
    "format_date",
    "parse_number",
    "parse_range",
    "len",
    "add",
    "sub",
    "mul",
    "div",
    "ceil_div",
    "floor_div",
    "min",
    "max",
    "eq",
    "lt",
    "lte",
    "gt",
    "gte",
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
        """Feeds one window. Returns the fields whose fill fell or whose errors rose, once the window is big enough."""
        return [FieldAlert(**a) for a in self._core.check(window)]


@dataclass(frozen=True)
class GateResult:
    """`merge` is true only when `reasons`, the rules the candidate broke, is empty."""

    merge: bool
    reasons: list[str]


def merge_gate(
    report: Mapping[str, Any],
    *,
    max_change: float = 0.01,
    known: Sequence[tuple[Mapping[str, Any], Mapping[str, Any]] | Mapping[str, Mapping[str, Any]]] | None = None,
    fill_alerts: Sequence[FieldAlert | Mapping[str, Any]] | None = None,
) -> GateResult:
    """Whether a shadow candidate can be promoted without a person, given `ShadowDiff.snapshot()`.

    Routing fields: no regressions, no conflicts, improvements only when a `known` entity states
    the field's value. Every other field: regressions plus conflicts at most `max_change` of
    `compared`. Any of `fill_alerts` blocks the merge. `known` holds `(candidate, expected)` pairs
    (or `{"candidate": ..., "expected": ...}`): the candidate's output for an entity whose true
    routing values are known, next to those values.
    """
    r = _core.merge_gate(report, max_change=max_change, known=known, fill_alerts=fill_alerts)
    return GateResult(merge=r["merge"], reasons=r["reasons"])
