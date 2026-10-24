"""canonic verify: maps every sample pair through a spec and gates on shape, values and rejects."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from ..fit import verify
from ..fit.pairs import load_pairs
from ..io import Transforms, UsageError, parser, read_json

USAGE = """canonic verify <spec.json> --schema <file> --default <dir> --provider <dir> [options]

  Maps each provider body through the spec and compares the output with the default's body of
  the same name. It checks the shape at every depth (keys, key order, JSON types, null against
  an object or array; an x-open map may differ in keys) and, per mapped field, the share of pairs that
  match. A rejected pair fails only if the spec without its reject rules maps it to exactly the
  default's body, or if too many pairs are rejected. A skipped pair (on one side only, or not
  JSON) fails too. A leaf the spec does not map but the default fills is listed as not mapped.
  Exit 0 when all checks pass, 1 when a check fails, 2 when a file cannot be read or there are
  no pairs.

  --inputs <dir>       request inputs per pair, for fields that read $input
  --transforms <file>  the custom transforms the spec names (a Python module)
  --min-match <f>      share of pairs each mapped field must match (default 0.95)
  --max-reject <f>     share of pairs the spec may reject (default 0.10)
  --fail-unmapped      fail when the default fills a leaf the spec does not map
  --json               print the result as JSON instead of a table"""


def _table(r: verify.VerifyResult, pairs: int, skipped: list[tuple[str, str]]) -> str:
    lines = [f"skipped  {n}  {why}" for n, why in skipped]
    width = max((len(k) for k in r.fields), default=5)
    for k, (m, t) in r.fields.items():
        miss = f"  misses {', '.join(r.misses[k])}" if r.misses.get(k) else ""
        lines.append(f"{k.ljust(width)}  {m}/{t}  {m / t if t else 1.0:.2f}{miss}")
    lines += [f"not mapped: {k} ({n} pairs with a value)" for k, n in r.unmapped.items()]
    lines += [f"shape  {p}  {at}  {msg}" for p, at, msg in r.shape_failures]
    lines += [f"rejected  {n}" for n in r.rejected]
    lines += [f"rejected wrongly  {n}  (without its reject rules the spec maps it to the default's body)" for n in r.rejected_wrongly]
    lines.append(f"verify: {'ok' if r.ok else 'failed'} ({pairs} pairs, {len(r.rejected)} rejected)")
    return "\n".join(lines) + "\n"


def run(args: list[str], transforms: Transforms) -> int:
    p = parser(USAGE)
    for flag in ("--schema", "--default", "--provider", "--inputs"):
        p.add_argument(flag)
    p.add_argument("--min-match", type=float, default=0.95)
    p.add_argument("--max-reject", type=float, default=0.10)
    p.add_argument("--json", action="store_true")
    p.add_argument("--fail-unmapped", action="store_true")
    p.add_argument("spec", nargs="?")
    a = p.parse_intermixed_args(args)
    if not (a.spec and a.schema and a.default and a.provider):
        raise UsageError(USAGE)
    pairs, skipped = load_pairs(Path(a.default), Path(a.provider), Path(a.inputs) if a.inputs else None)
    if not pairs:
        print("verify: no pairs", file=sys.stderr)
        return 2
    r = verify.run(read_json(a.spec), read_json(a.schema), pairs, a.min_match, transforms, a.max_reject, skipped, a.fail_unmapped)
    if a.json:
        sys.stdout.write(json.dumps(r.to_json(len(pairs), skipped), indent=2, ensure_ascii=False) + "\n")
    else:
        sys.stdout.write(_table(r, len(pairs), skipped))
    return 0 if r.ok else 1
