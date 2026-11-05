"""canonic infer: drafts a spec from sample responses, with a per-field report on stderr."""

from __future__ import annotations

import os
import sys
from typing import Any

from ..infer.infer import FieldReport, infer, to_fixed
from ..infer.learn import learn_synonyms
from ..infer.ask import ASK_TIMEOUT
from ..io import Transforms, UsageError, is_obj, js_json, js_number, json_files, pad, parse_json, parser, read_json, read_text, seconds

USAGE = """canonic infer --schema <canonical.json> --provider <name> --endpoint <name> [options] <samples>...

  Drafts a spec from sample responses (files, or directories of .json files; 5 to 20 is right).
  The spec goes to stdout or --out with status "draft"; the report goes to stderr.

  --known <file>      canonical output for entities you already know: {"<sample file name>": {...}}
  --synonyms <file>   extra name synonyms, {"token": ["canonical", "tokens"]}, merged over the seed
  --learn <spec>      an approved spec to learn synonyms from (repeatable); --synonyms wins on conflicts
  --min-score <n>     pairs scoring below this stay unmapped (default 0.4)
  --ask <command>     optional model pass: each unmapped or low-confidence field is sent to this shell
                      command as a JSON question on stdin; it prints one source path or null
                      (for example --ask "./ask.sh"). Answers are still marked for review.
  --ask-timeout <s>   seconds one --ask command may run before its field is skipped (default 120)
  --out <file>        write the spec here instead of stdout"""


def report(fields: list[FieldReport], spec: dict[str, Any]) -> str:
    lines = []
    for f in fields:
        if f.how == "unmapped":
            best = f"  best: {f.best} ({js_number(f.confidence or 0)})" if f.best else ""
            lines.append(f"{pad(f.field, 28)}unmapped{best}")
        else:
            flag = "  <- check" if f.check else ""
            lines.append(f"{pad(f.field, 28)}{pad(f.how, 9)}{to_fixed(f.confidence or 0)}  {js_json(f.rule)}{flag}")
    for field, token in (spec.get("anchors") or {}).items():
        lines.append(f"anchor: {field} {token}")
    return "\n".join(lines)


def min_score(text: str) -> float:
    from ..._api import default_transforms

    n = default_transforms["cast"](text, "number")  # JavaScript's Number(), without 0x literals
    if not (isinstance(n, (int, float)) and 0 <= n <= 1):
        raise UsageError(f"--min-score needs a number from 0 to 1, got {js_json(text)}")
    return float(n)


def extras(known: str | None, synonyms: str | None, learn: list[str] | None, min_score_text: str | None, files: list[str], schema: Any) -> dict[str, Any]:
    """infer() keyword arguments from the shared --known, --synonyms, --learn and --min-score flags."""
    by_name = read_json(known) if known else None
    approved = [read_json(f) for f in learn or []]
    for f, s in zip(learn or [], approved):
        if not (is_obj(s) and is_obj(s.get("fields"))):
            raise ValueError(f"{f}: an approved spec needs fields")
    learned = learn_synonyms([s["fields"] for s in approved], schema) if approved else {}
    merged = {**learned, **(read_json(synonyms) if synonyms else {})}
    out: dict[str, Any] = {}
    if known:
        out["known"] = [by_name.get(os.path.basename(f)) if is_obj(by_name) else None for f in files]
    if merged:
        out["synonyms"] = merged
    if min_score_text:
        out["min_score"] = min_score(min_score_text)
    return out


def run(args: list[str], transforms: Transforms) -> int:
    p = parser(USAGE)
    for flag in ("--schema", "--provider", "--endpoint", "--known", "--synonyms", "--min-score", "--out", "--ask"):
        p.add_argument(flag)
    p.add_argument("--learn", action="append")
    p.add_argument("--ask-timeout", type=seconds, default=ASK_TIMEOUT)
    p.add_argument("paths", nargs="*")
    a = p.parse_intermixed_args(args)
    if not a.schema or not a.provider or not a.endpoint or not a.paths:
        raise UsageError(USAGE)
    files = json_files(a.paths)
    raws = [read_text(f) for f in files]
    schema = read_json(a.schema)
    samples = [parse_json(t, f) for t, f in zip(raws, files)]
    options = extras(a.known, a.synonyms, a.learn, a.min_score, files, schema)
    spec, fields = infer(samples, schema, provider=a.provider, endpoint=a.endpoint, raws=raws, ask=a.ask, ask_timeout=a.ask_timeout, **options)
    if len(files) < 5:
        print(f"only {len(files)} samples: a field that happens to be null in all of them can't be found", file=sys.stderr)
    print(report(fields, spec), file=sys.stderr)
    text = js_json(spec, 2) + "\n"
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0
