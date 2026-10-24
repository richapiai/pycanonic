"""canonic schema: derives the canonical schema from the default provider's samples."""

from __future__ import annotations

import sys

from ..fit.derive import derive_schema, flags
from ..io import Transforms, UsageError, js_json, json_files, parser, read_json

USAGE = """canonic schema <samples>... [--out <file>]

  Derives a language 2 canonical schema from the default provider's sample responses (files, or
  directories of .json files). It records key order, types and the empty form of each field
  (x-empty). The schema goes to stdout or --out. Fields to check go to stderr: mixed types,
  mixed empty (the samples disagree on the empty form) and always null.

  --out <file>   write the schema here instead of stdout"""


def run(args: list[str], transforms: Transforms) -> int:
    p = parser(USAGE)
    p.add_argument("--out")
    p.add_argument("paths", nargs="*")
    a = p.parse_intermixed_args(args)
    if not a.paths:
        raise UsageError(USAGE)
    files = json_files(a.paths)
    samples = [read_json(f) for f in files]
    for name, s in zip(files, samples):
        if not isinstance(s, (dict, list)):
            raise ValueError(f"{name}: not an object or array")
    schema = derive_schema(samples)
    for path, why in flags(schema):
        print(f"{path}  {why}", file=sys.stderr)
    text = js_json(schema, 2) + "\n"
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0
