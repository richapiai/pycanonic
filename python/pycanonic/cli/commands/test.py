"""canonic test: maps fixtures through a spec and checks the fields each one's "expected" names."""

from __future__ import annotations

import json
import os
import sys
from typing import Any

from ..._api import compile_spec
from ..._core import Artifact, Rejected
from ..io import Transforms, UsageError, json_files, parser, read_json

USAGE = """canonic test --schema <canonical.json> [--transforms <module.py>] <spec.json> <fixtures>...

  Maps each fixture's payload through the spec and checks the fields its "expected" names.
  A fixture file is {"payload": ..., "expected": ...} or {"cases": [{"payload", "expected"}, ...]}.
  A case may add "context": {...}, the request inputs that $input paths read. "expected":
  {"$rejected": "<reason>"} means the spec must reject the payload with that reason.
  Fields left out of "expected" aren't checked, inside list items too. Exits 1 if any case fails."""


def _js(v: Any) -> str:
    return json.dumps(v, ensure_ascii=False, separators=(",", ":"))


def mismatches(expected: Any, actual: Any, at: str = "") -> list[str]:
    """Paths where actual differs from what expected lists. Objects and same-length lists are compared field by field."""
    if isinstance(expected, dict) and isinstance(actual, dict):
        return [m for k, v in expected.items() for m in mismatches(v, actual.get(k), f"{at}.{k}" if at else k)]
    if isinstance(expected, list) and isinstance(actual, list) and len(expected) == len(actual):
        return [m for i, (e, a) in enumerate(zip(expected, actual)) for m in mismatches(e, a, f"{at}[{i}]")]
    # Leaves are scalars here. JSON tells true from 1, Python's == does not.
    if expected == actual and isinstance(expected, bool) == isinstance(actual, bool):
        return []
    return [f"{at}: expected {_js(expected)}, got {_js(actual)}"]


def check(artifact: Artifact, case: Any) -> list[str]:
    """Mismatches of one fixture case, a rejection included."""
    want = case["expected"]
    rejected = want.get("$rejected") if isinstance(want, dict) else None
    try:
        got = artifact.map(case["payload"], context=case.get("context"))
    except Rejected as e:
        if rejected is None:
            return [f"rejected {e.reason}"]
        return [] if e.reason == rejected else [f"expected rejected {rejected}, got rejected {e.reason}"]
    if rejected is not None:
        return [f"expected rejected {rejected}, got an output"]
    return mismatches(want, got)


def _cases(file: str) -> list[Any]:
    f = read_json(file)
    if isinstance(f, dict) and isinstance(f.get("cases"), list):
        return list(f["cases"])
    if isinstance(f, dict) and "payload" in f and "expected" in f:
        return [f]
    raise ValueError(f"{file}: a fixture needs payload and expected, or cases")


def run(args: list[str], transforms: Transforms) -> int:
    p = parser(USAGE)
    p.add_argument("--schema")
    p.add_argument("paths", nargs="*")
    a = p.parse_intermixed_args(args)
    if not a.schema or len(a.paths) < 2:
        raise UsageError(USAGE)
    spec_path, *fixtures = a.paths
    artifact = compile_spec(read_json(spec_path), read_json(a.schema), transforms=transforms)
    total = failed = 0
    for file in json_files(fixtures):
        for i, c in enumerate(_cases(file)):
            total += 1
            bad = check(artifact, c)
            if bad:
                failed += 1
                for m in bad:
                    print(f"{os.path.basename(file)}#{i} {m}", file=sys.stderr)
    print(f"{total} cases, {failed} failed")
    return 1 if failed else 0
