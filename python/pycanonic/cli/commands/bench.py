"""canonic bench: times a spec on your own sample responses, next to json.loads of the same bodies."""

from __future__ import annotations

import json
from functools import reduce
from typing import Any

from ..._api import compile_spec
from ..._core import Artifact, Rejected
from ..io import Transforms, UsageError, json_files, pad, parser, read_json, read_text
from ..measure import fmt, measure

USAGE = """canonic bench --schema <canonical.json> [--transforms <module.py>] <spec.json> <samples>...

  Times the spec on your own sample responses: map (from parsed objects and from bytes), map_json,
  a view with its routing fields read, and the extractor, next to json.loads of the same bodies.
  A language 2 spec has no view row, and samples its reject conditions stop are left out.
  Single core, median of 15 rounds."""


def _kept(art: Artifact, raw: str) -> bool:
    try:
        art.map(raw)
    except Rejected:
        return False
    return True


def run(args: list[str], transforms: Transforms) -> int:
    p = parser(USAGE)
    p.add_argument("--schema")
    p.add_argument("paths", nargs="*")
    a = p.parse_intermixed_args(args)
    if not a.schema or len(a.paths) < 2:
        raise UsageError(USAGE)
    spec_path, *samples = a.paths
    art = compile_spec(read_json(spec_path), read_json(a.schema), transforms=transforms)
    every = [read_text(f) for f in json_files(samples)]
    raws = [r for r in every if _kept(art, r)]
    if not raws:
        raise ValueError("the spec rejects every sample")
    art.stats_reset()
    raw_bytes = [r.encode() for r in raws]
    parsed = [json.loads(r) for r in raws]
    n = len(raws)
    routing = [p.split(".") for p in art.routing]

    def read_routing(v: Any) -> list[Any]:
        return [reduce(lambda o, k: None if o is None else getattr(o, k), path, v) for path in routing]

    view = [("view + routing reads", lambda i: read_routing(art.view(parsed[i])))]
    rows = [
        ("json.loads", lambda i: json.loads(raws[i])),
        ("map", lambda i: art.map(parsed[i])),
        *(view if json.loads(art.plan_json())["format"] == 1 else []),
        ("extract (routing, from text)", lambda i: art.extract(raws[i])),
        ("map (bytes in)", lambda i: art.map(raw_bytes[i])),
        ("map_json (bytes in, bytes out)", lambda i: art.map_json(raw_bytes[i])),
    ]
    times = measure(rows, rounds=15, batch_ns=5_000_000, modulus=n)
    kb = sum(len(r) for r in raws) / n / 1024
    left_out = f" ({len(every) - n} rejected, left out)" if len(every) > n else ""
    print(f"{art.provider} {art.endpoint} v{art.version}: {n} samples{left_out}, {kb:.1f} KB average, {len(routing)} routing fields\n")
    for (name, _), ns in zip(rows, times):
        print(f"{pad(name, 30)}{fmt(ns):>10}")
    return 0
