"""canonic build: compiles each spec to a plan file next to it, loaded later with `pycanonic.load`."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ..._api import compile_spec
from ..io import Transforms, UsageError, parser, read_json

USAGE = """canonic build --schema <canonical.json> [--transform <name>]... <spec.json>...

  Compiles each spec to a plan next to it (spec.json -> spec.plan.json), which pycanonic.load
  reads without compiling again. --transform names a deployment-private transform the plan will
  be given at load time. --transforms <module.py> declares every transform the module defines."""


def _declared(*_: Any) -> None:
    return None


def run(args: list[str], loaded: Transforms) -> int:
    p = parser(USAGE)
    p.add_argument("--schema")
    p.add_argument("--transform", action="append", default=[])
    p.add_argument("specs", nargs="*")
    a = p.parse_intermixed_args(args)
    if not a.schema or not a.specs:
        raise UsageError(USAGE)
    canonical = read_json(a.schema)
    # Only the names matter at build time; load() receives the real functions.
    transforms: Transforms = {**{name: _declared for name in a.transform}, **loaded}
    for path in a.specs:
        out = path.removesuffix(".json") + ".plan.json"
        plan = compile_spec(read_json(path), canonical, transforms=transforms).plan_json()
        Path(out).write_text(plan, encoding="utf-8")
        print(f"{path} -> {out}")
    return 0
