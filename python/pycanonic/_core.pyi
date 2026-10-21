"""Type stubs for the Rust extension `pycanonic._core`."""

from collections.abc import Callable, Iterator, Mapping, Sequence
from typing import Any, Literal, TypeVar, final

from typing_extensions import NotRequired, TypedDict

__all__ = [
    "Artifact",
    "ExtractStream",
    "FillMonitor",
    "ShadowDiff",
    "Stats",
    "View",
    "compile",
    "default_transform",
    "fingerprint",
    "load_plan",
    "merge_gate",
    "shape",
    "validate_canonical",
    "validate_spec",
    "validator",
    "version",
]

_T = TypeVar("_T")
_Document = dict[str, Any] | str | bytes
_Transforms = dict[str, Callable[..., Any]]

class _FieldStats(TypedDict):
    fill: int
    errors: int
    total: NotRequired[int]

class _StatsSnapshot(TypedDict):
    responses: int
    fields: dict[str, _FieldStats]

@final
class Stats:
    def snapshot(self) -> _StatsSnapshot: ...
    def reset(self) -> None: ...

@final
class ExtractStream:
    def push(self, chunk: bytes | str) -> bool: ...
    def result(self) -> dict[str, Any]: ...

class _FieldAlert(TypedDict):
    field: str
    kind: Literal["fill", "errors"]
    rate: float
    baseline: float
    responses: int

@final
class FillMonitor:
    def __init__(
        self,
        *,
        min_responses: int = 200,
        history: int = 24,
        sigmas: float = 4.0,
        min_drop: float = 0.25,
        min_rise: float = 0.01,
    ) -> None: ...
    def check(self, window: Mapping[str, Any]) -> list[_FieldAlert]: ...

@final
class View:
    """A lazy canonical object: each field is mapped on its first read and kept; no stats.

    `view.name` reads a field unless `View` has an attribute of that name (`to_dict`) or the name
    is a `__dunder__`; `view["name"]` always reads the field and raises `KeyError` for an unknown one.
    """

    def __getattr__(self, field: str) -> Any: ...
    def __getitem__(self, field: str) -> Any: ...
    def to_dict(self) -> dict[str, Any]: ...
    def __iter__(self) -> Iterator[str]: ...
    def __dir__(self) -> list[str]: ...

@final
class Artifact:
    @property
    def provider(self) -> str: ...
    @property
    def endpoint(self) -> str: ...
    @property
    def version(self) -> int: ...
    @property
    def status(self) -> Literal["draft", "shadow", "promoted", "deprecated"]: ...
    @property
    def routing(self) -> list[str]: ...
    @property
    def source(self) -> str: ...
    @property
    def stats(self) -> Stats: ...
    def map(self, src: object) -> dict[str, Any]: ...
    def view(self, src: object) -> View: ...
    def map_json(self, raw: bytes | str) -> bytes: ...
    def extract(self, raw: bytes | str) -> dict[str, Any]: ...
    def stream(self) -> ExtractStream: ...
    def stats_snapshot(self) -> _StatsSnapshot: ...
    def stats_reset(self) -> None: ...
    def plan_json(self) -> str: ...

def version() -> str: ...
def compile(spec: _Document, canonical: _Document, transforms: _Transforms | None = None) -> Artifact: ...
def load_plan(plan_json: str, transforms: _Transforms | None = None) -> Artifact: ...
def validate_spec(spec: _Document) -> list[tuple[str, str]]: ...
def validate_canonical(schema: _Document) -> list[tuple[str, str]]: ...
def default_transform(name: str, value: object, arg: object = None) -> Any: ...
def validator(semantic: str, value: _T) -> _T | None: ...
def shape(value: object) -> list[str]:
    """Sorted key paths of a parsed response, or of JSON text given as `bytes`.

    Only a `dict` and a `list`/`tuple` have members; anything else (a `str`, a `set`) is a scalar.
    A Python object deeper than 512 levels, with more than 1,000,000 containers or 64 MiB of
    distinct paths (a cyclic one, or one that shares its children) raises `ValueError` rather than
    giving a cut shape. `bytes` are parsed and never cut.
    """

def fingerprint(value: object) -> str:
    """FNV-1a over `shape(value)`, as 8 hex digits, equal to canonic's; raises where `shape` does."""

class _FieldDiff(TypedDict):
    compared: int
    same: int
    improvements: int
    regressions: int
    conflicts: int

class _ShadowReport(TypedDict):
    responses: int
    routing: list[str]
    fields: dict[str, _FieldDiff]

@final
class ShadowDiff:
    """Compares a candidate's `map()` output with the promoted one's, field by field, response by response."""

    def __init__(self, routing: Sequence[str]) -> None: ...
    def add(self, promoted: Mapping[str, Any], candidate: Mapping[str, Any]) -> None:
        """Records one response. A value nested deeper than 512 containers or with more than 1,000,000 values
        (a cyclic one) raises `ValueError` and records nothing."""

    def snapshot(self) -> _ShadowReport: ...

class _GateResult(TypedDict):
    merge: bool
    reasons: list[str]

def merge_gate(
    report: Mapping[str, Any],
    *,
    max_change: float = 0.01,
    known: Sequence[tuple[Mapping[str, Any], Mapping[str, Any]] | Mapping[str, Mapping[str, Any]]] | None = None,
    fill_alerts: Sequence[object] | None = None,
) -> _GateResult: ...
