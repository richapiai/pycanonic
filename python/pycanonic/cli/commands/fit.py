"""canonic fit: drafts a language 2 spec from sample pairs of the default and the new provider."""

from __future__ import annotations

from ..fit import align, fit
from ..io import Transforms, UsageError, add_tolerance, parser

USAGE = """canonic fit --default <dir> --provider <dir> --provider-name <name> --endpoint <name> --out <file> [options]

  Pairs the files in the two folders by name (10 to 30 pairs that differ; it warns below 8 and
  stops below 3), finds the rule for every canonical field, and writes a language 2 draft spec.
  Each field gets a tier: auto (a reviewer can skip it), review, or manual (no rule found).
  Exit 0 when every field is auto, 1 when a field needs review, 2 for a manual field or a
  skipped pair.

  --inputs <dir>          request inputs per pair, for fields that depend on them ($input)
  --schema <file>         the canonical schema (default: derived from the --default samples)
  --keep <file>           reviewed rules to keep as written: {"<field key>": <rule>}
  --ask <command>         ask this shell command about manual fields, as canonic infer --ask does
  --strict                a field to review also exits 2
  --min-auto <f>          support and holdout share an auto field needs (default 0.95)
  --min-pairs <n>         pairs with a value an auto field needs (default 5)
  --report <file>         write the report as <file>.md and <file>.json (default: Markdown to stdout)
  --transforms-out <file> write a Python stub per manual field that needs code, with doctests
  --num-tolerance <f>     count two numbers as equal when they differ by at most this share of the
                          larger one (0.01 is 1%); a field that needs it, a leaf inside list items
                          included, is review, not auto
  --url-ignore-query      compare two scheme:// URLs without their query (the part from ? to before #)
  --text-ignore-space     compare two texts after collapsing white space and trimming
  --align-items <default list> <provider list> --by <default id> <provider id> [--id-pattern <regex>]
                          fit the items of a list, not the whole bodies: pair the elements of the two
                          lists by id, in the default list's order, as pairs named `<pair>#<n>`. The
                          list paths are dotted (`data.items`), the id paths are relative to an element.
                          The id is its digits (`urn:example:item:7` and "7" match), or group 1 of the
                          regex, or the whole match. --schema is the whole-response schema (it is cut to the list's
                          items); --keep and the spec are item-level.
                          The report counts matched, default-only and provider-only items per pair"""


def run(args: list[str], transforms: Transforms) -> int:
    p = parser(USAGE)
    for flag in ("--default", "--provider", "--inputs", "--schema", "--keep", "--ask", "--provider-name", "--endpoint", "--out", "--report", "--transforms-out"):
        p.add_argument(flag)
    p.add_argument("--strict", action="store_true")
    p.add_argument("--min-auto", type=float, default=0.95)
    p.add_argument("--min-pairs", type=int, default=5)
    add_tolerance(p)
    align.add_flags(p)
    a = p.parse_args(args)
    if not (a.default and a.provider and a.provider_name and a.endpoint and a.out):
        raise UsageError(USAGE)
    return fit.run(a, transforms)
