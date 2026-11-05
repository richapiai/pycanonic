"""canonic verify: maps every sample pair through a spec and gates on shape, values and rejects."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from ..fit import align, verify
from ..fit.pairs import load_pairs
from ..io import Transforms, UsageError, add_tolerance, parser, read_json, tolerance

USAGE = """canonic verify <spec.json> --schema <file> --default <dir> --provider <dir> [options]

  Maps each provider body through the spec and compares the output with the default's body of
  the same name. It checks the shape at every depth (keys, key order, JSON types, null against
  an object or array; an x-open map may differ in keys) and, per mapped field, per leaf inside list items (`items[].name`, by position) and per list
  length (`items[] (count)`), the share of pairs that
  match. A rejected pair fails only if the spec without its reject rules maps it to exactly the
  default's body, or if too many pairs are rejected. A skipped pair (on one side only, or not
  JSON) fails too. A leaf the spec does not map but the default fills is listed as not mapped.
  Exit 0 when all checks pass, 1 when a check fails, 2 when a file cannot be read or there are
  no pairs.

  --inputs <dir>       request inputs per pair, for fields that read $input
  --transforms <file>  the custom transforms the spec names (a Python module)
  --min-match <f>      share of pairs each mapped field must match (default 0.95)
  --max-reject <f>     share of pairs the spec may reject (default 0.10)
  --ignore <field>     leave a field and everything below it out of the scoring and the shape
                       check; repeat for more (`items` covers `items[]`, `items[].x`, `items[] (count)`)
  --fail-unmapped      fail when the default fills a leaf the spec does not map, unless it is allowed
  --allow FIELD=REASON expect a field to stay unmapped, with the reason; it covers the field and
                       everything below it, and repeats. The report names the first five pairs
                       with a value, and notes an allowed field that is mapped or empty in every pair
  --allow-file <json>  the same as a JSON object of field to reason; both sources merge
  --num-tolerance <f>  count two numbers as equal when they differ by at most this share of the larger
                       one (0.01 is 1%); a tolerated pair counts as a match, and the field reports how many
  --url-ignore-query   compare two scheme:// URLs without their query (the part from ? to before #)
  --text-ignore-space  compare two texts after collapsing white space and trimming
  --align-items <default list> <provider list> --by <default id> <provider id> [--id-pattern <regex>]
                       verify the items of a list: pair the elements of the two lists by id (the
                       digits of the id, or group 1 of the regex, else the whole match) as pairs named
                       `<pair>#<n>`, and map each provider item through the spec against the item
                       schema. --schema and the spec are item-level. The report counts matched,
                       default-only and provider-only items per pair
  --json               print the result as JSON instead of a table"""


def _table(r: verify.VerifyResult, pairs: int, skipped: list[tuple[str, str]]) -> str:
    lines = [f"skipped  {n}  {why}" for n, why in skipped]
    width = max((len(k) for k in r.fields), default=5)
    for k, (m, t) in r.fields.items():
        miss = f"  misses {', '.join(r.misses[k])}" if r.misses.get(k) else ""
        tol = f"  tolerated {r.tolerated[k]}" if r.tolerated.get(k) else ""
        lines.append(f"{k.ljust(width)}  {m}/{t}  {m / t if t else 1.0:.2f}{tol}{miss}")
    for k, ns in r.unmapped.items():
        more = ", ..." if len(ns) > 5 else ""
        why = f"  allowed: {r.allowed[k]}" if k in r.allowed else ""
        lines.append(f"not mapped: {k} ({len(ns)} pairs with a value: {', '.join(ns[:5])}{more}){why}")
    lines += [f"allow: {a} is mapped or empty in every pair" for a in r.stale_allow]
    lines += [f"shape  {p}  {at}  {msg}" for p, at, msg in r.shape_failures]
    lines += [f"rejected  {n}" for n in r.rejected]
    lines += [f"rejected wrongly  {n}  (without its reject rules the spec maps it to the default's body)" for n in r.rejected_wrongly]
    lines.append(f"verify: {'ok' if r.ok else 'failed'} ({pairs} pairs, {len(r.rejected)} rejected)")
    return "\n".join(lines) + "\n"


def _allow(pairs: list[str], path: str | None) -> dict[str, str]:
    """The allowed fields and their reasons from `--allow FIELD=REASON` and `--allow-file`."""
    out: dict[str, str] = {}
    if path:
        data = read_json(path)
        if not isinstance(data, dict) or not all(isinstance(k, str) and k and isinstance(v, str) for k, v in data.items()):
            raise ValueError(f"--allow-file {path}: expected a JSON object of field to reason, such as {{\"status\": \"not-sold\"}}")
        out.update(data)
    for item in pairs:
        field, sep, why = item.partition("=")
        if not sep or not field.strip():
            raise ValueError(f'--allow {item!r}: expected FIELD=REASON, such as status=not-sold')
        out[field.strip()] = why
    return out


def run(args: list[str], transforms: Transforms) -> int:
    p = parser(USAGE)
    for flag in ("--schema", "--default", "--provider", "--inputs"):
        p.add_argument(flag)
    p.add_argument("--min-match", type=float, default=0.95)
    p.add_argument("--max-reject", type=float, default=0.10)
    p.add_argument("--json", action="store_true")
    p.add_argument("--ignore", action="append", default=[])
    p.add_argument("--fail-unmapped", action="store_true")
    p.add_argument("--allow", action="append", default=[])
    p.add_argument("--allow-file")
    add_tolerance(p)
    align.add_flags(p)
    p.add_argument("spec", nargs="?")
    a = p.parse_intermixed_args(args)
    if not (a.spec and a.schema and a.default and a.provider):
        raise UsageError(USAGE)
    allow = _allow(a.allow, a.allow_file)
    pairs, skipped = load_pairs(Path(a.default), Path(a.provider), Path(a.inputs) if a.inputs else None)
    schema = read_json(a.schema)
    pairs, schema, aligned = align.from_args(a, pairs, schema)
    if not pairs:
        print("verify: no pairs", file=sys.stderr)
        return 2
    r = verify.run(read_json(a.spec), schema, pairs, a.min_match, transforms, a.max_reject, skipped, a.fail_unmapped, a.ignore, allow, tolerance(a))
    if a.json:
        out = r.to_json(len(pairs), skipped)
        sys.stdout.write(json.dumps({**out, "aligned": align.to_json(aligned)} if aligned else out, indent=2, ensure_ascii=False) + "\n")
    else:
        sys.stdout.write("".join(line + "\n" for line in align.lines(aligned)) + _table(r, len(pairs), skipped))
    return 0 if r.ok else 1
