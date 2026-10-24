# API reference

This page lists everything `pycanonic` and `pycanonic.stream` export, with the types. The package ships type stubs (`_core.pyi`) and a `py.typed` marker. The guides explain the use of each piece. This page says what each one takes and returns.

```python
from pycanonic import (
    compile_spec, load, validate_spec, validate_canonical, SpecError, Issue,
    Artifact, Stats, Registry, Transform, default_transforms, validators,
    FillMonitor, FieldAlert, shape, fingerprint, ShadowDiff, merge_gate, GateResult,
)
from pycanonic.stream import routing_tap, RoutingTap
```

## Compiling

### `compile_spec(spec, canonical, *, transforms=None) -> Artifact`

```python
def compile_spec(
    spec: dict[str, Any] | str | bytes,
    canonical: dict[str, Any] | str | bytes,
    *,
    transforms: dict[str, Transform] | None = None,
) -> Artifact
```

This function checks the spec, resolves it against the canonical schema and builds the plan. The spec and the schema are each a `dict` or JSON text. A spec or schema error raises `SpecError`, a `ValueError` subclass. The message names the field.

| Parameter | Type | |
|---|---|---|
| `spec` | `dict`, `str` or `bytes` | see [spec-format.md](spec-format.md) |
| `canonical` | JSON Schema as `dict`, `str` or `bytes` | see [Canonical schema](spec-format.md#canonical-schema) |
| `transforms` | `dict[str, Transform]` | your own transforms. A name that matches a default replaces it. |

### `load(path_or_json, *, transforms=None) -> Artifact`

```python
def load(
    path_or_json: str | bytes | os.PathLike[str],
    *,
    transforms: dict[str, Transform] | None = None,
) -> Artifact
```

This function loads an artifact from a plan that `artifact.plan_json()` or `canonic build` wrote. It does not compile the spec again. A path object is read as UTF-8. A `str` that names an existing file is read the same way. A `str` that starts with `{` or `[` (after whitespace), and `bytes`, is the plan JSON itself. Any other `str` raises `FileNotFoundError`. Pass the same `transforms` the plan was compiled with.

### `validate_spec(spec) -> list[Issue]`

This function runs the grammar check alone and returns every problem it finds. It needs no canonical schema and does not compile. The list is empty for a valid spec.

### `validate_canonical(schema) -> list[Issue]`

This function returns what `compile_spec` rejects in a canonical schema, without a spec. The list is empty for a schema that loads.

### `Issue`

```python
@dataclass(frozen=True)
class Issue:
    at: str       # 'spec.fields["email"]'
    message: str
```

### `SpecError`

`class SpecError(ValueError)`. `compile_spec` raises it for a bad spec or schema.

## The artifact

```python
class Artifact:
    provider: str                     # read-only properties
    endpoint: str
    version: int
    status: Literal["draft", "shadow", "promoted", "deprecated"]
    routing: list[str]                # canonical paths of the routing fields this spec maps
    source: str                       # the plan listing, one line per field
    stats: Stats

    def map(self, src: Json | Parsed) -> dict[str, Any]: ...
    def map_json(self, raw: Json) -> bytes: ...
    def view(self, src: Json | Parsed) -> View: ...
    def extract(self, raw: Json) -> dict[str, Any]: ...
    def stream(self) -> ExtractStream: ...
    def plan_json(self) -> str: ...
    def stats_snapshot(self) -> dict: ...   # the same as stats.snapshot()
    def stats_reset(self) -> None: ...      # the same as stats.reset()
```

`Json` is JSON text: `bytes | str | bytearray | memoryview`. A `bytearray` or `memoryview` is copied before parsing. `Parsed` is what `json.loads` returns (`dict`, `list`, `int`, `float`, `bool`, `None`) or a `tuple`. At the root, it can also be any other `Mapping`, which is copied one level into a `dict`. Below the root only `dict`, `list` and `tuple` have members.

- `map(src)` returns the full canonical object as nested dicts. It never raises on bad data, and it counts stats. A root that is neither `Json` nor `Parsed` (a `set`, an arbitrary object) raises `TypeError`.
- `map_json(raw)` takes JSON text and returns the canonical object as JSON `bytes`. It builds no Python objects and counts stats. The bytes equal what `JSON.stringify` writes for the same value in the TypeScript package.
- `view(src)` returns a lazy canonical object. Each field is mapped the first time you read it and then kept. It counts no stats.
- `extract(raw)` reads the routing fields from JSON text without parsing the whole body. The result is flat and keyed by spec field (`{"company.domain": "acme.com"}`).
- `stream()` returns a new incremental reader for the same fields. Use one per response.
- `plan_json()` returns the compiled plan as JSON, for `load`. The JSON carries `"format": 1`. `load` raises `ValueError` for a plan without it or with another value. In that case, rebuild the plan from its spec.

### `View`

```python
class View:
    def __getattr__(self, field: str) -> Any: ...   # view.email
    def __getitem__(self, field: str) -> Any: ...   # view["email"], KeyError for an unknown field
    def to_dict(self) -> dict[str, Any]: ...        # maps the whole level, equal to map(src) there
    def __iter__(self) -> Iterator[str]: ...        # the canonical field names
```

A nested canonical object is a view too, so `view.company.domain` works. `view.name` reads a field unless `View` has an attribute with that name (`to_dict`) or the name is a `__dunder__`. `view["name"]` always reads the field.

### `ExtractStream`

```python
class ExtractStream:
    def push(self, chunk: Json) -> bool: ...          # True once every routing field is read
    def result(self) -> dict[str, Any]: ...           # ends the input, fields not found are None
```

### `Stats`

```python
class Stats:
    def snapshot(self) -> StatsSnapshot: ...
    def reset(self) -> None: ...

class StatsSnapshot(TypedDict):
    responses: int
    fields: dict[str, FieldStats]      # keyed by spec field

class FieldStats(TypedDict):
    fill: int     # responses (or elements) where the field had a value
    errors: int   # values a transform or a routing validator rejected
    total: NotRequired[int]   # only for a field inside an item map ("jobs[].title"): the elements seen
```

`map` and `map_json` count. `view`, `extract` and `stream` do not count.

## Transforms

```python
Transform = Callable[..., Any]   # f(value) or f(value, arg), returns the new value or None
```

`default_transforms` is a `dict` of the built-in transforms by name. `validators` is a `dict` of the routing validators by semantic. You can call any of them directly: `validators["email"](v)` returns `v` or `None`. [transforms.md](transforms.md) lists them. [custom-transforms.md](custom-transforms.md) has the contract for your own.

## Registry

```python
class Registry:
    def put(self, provider: str, endpoint: str, version: int, artifact: Artifact) -> None: ...
    def get(self, provider: str, endpoint: str) -> Artifact | None: ...         # the promoted artifact
    def get_shadow(self, provider: str, endpoint: str) -> Artifact | None: ...  # the shadow artifact
```

The registry holds one promoted and at most one shadow artifact per provider and endpoint. `put` raises `ValueError` for a `draft` or `deprecated` artifact. It also raises for an artifact whose provider, endpoint or version differ from the arguments. A promoted artifact with the version in the shadow slot clears that slot. Readers need no lock. Run one writer at a time.

## Drift

```python
def shape(value: object) -> list[str]: ...    # sorted key paths of a parsed response, or of JSON bytes
def fingerprint(value: object) -> str: ...    # 8 hex digits, equal for equal shapes and to canonic's

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
    def check(self, window: Mapping[str, Any]) -> list[FieldAlert]: ...

@dataclass(frozen=True)
class FieldAlert:
    field: str
    kind: Literal["fill", "errors"]
    rate: float
    baseline: float
    responses: int   # what the rate is out of: elements for a field inside an item map
```

On a Python object, `shape` and `fingerprint` raise `ValueError` past 512 levels, 1,000,000 containers or 64 MiB of path text. `bytes` input is parsed and never truncated. A body deeper than 512 levels raises `ValueError("invalid JSON: Depth")`. Only `dict`, `list` and `tuple` have members. Any other value, a `str` included, is a scalar.

## Shadow comparison

```python
class ShadowDiff:
    def __init__(self, routing: Sequence[str]) -> None: ...   # the promoted artifact's routing
    def add(self, promoted: Mapping[str, Any], candidate: Mapping[str, Any]) -> None: ...
    def snapshot(self) -> ShadowReport: ...

class ShadowReport(TypedDict):
    responses: int
    routing: list[str]
    fields: dict[str, FieldDiff]

class FieldDiff(TypedDict):
    compared: int
    same: int
    improvements: int
    regressions: int
    conflicts: int

def merge_gate(
    report: Mapping[str, Any],
    *,
    max_change: float = 0.01,
    known: Sequence[tuple[Mapping, Mapping] | Mapping[str, Mapping]] | None = None,
    fill_alerts: Sequence[FieldAlert | Mapping[str, Any]] | None = None,
) -> GateResult: ...

@dataclass(frozen=True)
class GateResult:
    merge: bool
    reasons: list[str]
```

`ShadowDiff.add` raises `ValueError` for a value nested deeper than 512 levels or with more than 1,000,000 values, and records nothing. `known` holds `(candidate, expected)` pairs, or `{"candidate": ..., "expected": ...}` dicts. [drift-and-learning-loop.md](drift-and-learning-loop.md) defines each count and each rule.

## `pycanonic.stream`

```python
def routing_tap(artifact: Artifact, source: AsyncIterable[bytes]) -> RoutingTap: ...

class RoutingTap:                       # an async iterator over source, chunks unchanged
    routing: asyncio.Future[dict[str, Any]]
```

`routing` resolves once the tap reads every routing field, or at the end of the body. See [streaming.md](streaming.md).

## The CLI

`pip install pycanonic` installs the `canonic` command, with the subcommands `infer`, `test`, `diff`, `build` and `bench`. See [inference.md](inference.md) and the README.
