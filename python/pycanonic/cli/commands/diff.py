"""canonic diff: shape and fill drift between two sets of samples, and with a spec, the next version
with just the broken fields re-inferred."""

from __future__ import annotations

import math
import sys
from collections.abc import Callable
from typing import Any

from ... import fingerprint
from ..._api import compile_spec
from ..._core import Rejected
from ..infer.infer import infer_draft
from ..infer.sources import collect, elements, fill
from ..io import Transforms, UsageError, is_obj, js_json, json_files, pad, parse_json, parser, read_json, read_text
from .infer import extras

USAGE = """canonic diff [--schema <canonical.json> --spec <spec.json> [options]] <old> <new>

  Compares two sets of sample responses from one provider (files or directories): new shapes, key
  paths that appeared, disappeared or changed fill rate. With a spec, it also reports the spec
  fields whose fill dropped, fields inside list items included, and proposes edits for just those,
  re-inferred from the new samples, as the next version in "shadow" (stdout, or --out).
  Exits 1 when it finds drift, 0 when it finds none, 2 when it can't run.

  --out <file>        write the proposed spec here instead of stdout
  --transforms <module.py>
                      private transforms the spec uses (see canonic --help)
  --known, --synonyms, --learn, --min-score
                      passed to the re-inference, as in canonic infer (--known is keyed by the
                      new samples' file names)"""

Spec = dict[str, Any]

GONE = 0.1
MOVED = 0.25


def pct(x: float) -> str:
    y = x * 100  # Math.round: halves up
    return f"{math.floor(y) + (1 if y - math.floor(y) >= 0.5 else 0)}%"


def shape_drift(old: list[Any], now: list[Any]) -> list[str]:
    seen = {fingerprint(x) for x in old}
    fresh = {f for f in map(fingerprint, now) if f not in seen}
    out = [f"{len(fresh)} new response shape{'s' if len(fresh) > 1 else ''}"] if fresh else []
    return out + path_drift(old, now)


def path_drift(old: list[Any], now: list[Any]) -> list[str]:
    before = {s.path: fill(s) for s in collect(old)}
    after = {s.path: fill(s) for s in collect(now)}
    out = [f"- {p} (was in {pct(f)})" for p, f in before.items() if f >= GONE and not after.get(p)]
    out += [f"+ {p} (in {pct(f)})" for p, f in after.items() if f >= GONE and not before.get(p)]
    for p, f in before.items():
        g = after.get(p)
        if g and abs(g - f) >= MOVED:
            out.append(f"~ {p} {pct(f)} -> {pct(g)}")
    return out


def fill_rates(spec: Spec, schema: Any, samples: list[Any], transforms: Transforms) -> dict[str, float]:
    """Fill rate of every spec field over the samples. A field inside an item map ("jobs[].title") is a rate over elements.
    Samples a reject condition stops do not count."""
    a = compile_spec(spec, schema, transforms=transforms)
    for x in samples:
        try:
            a.map(x)
        except Rejected:
            pass
    snap = a.stats.snapshot()
    return {f: s["fill"] / (s.get("total", snap["responses"]) or 1) for f, s in snap["fields"].items()}


def dropped(spec: Spec, schema: Any, old: list[Any], now: list[Any], transforms: Transforms) -> list[tuple[str, float, float]]:
    """Spec fields filled in at least 10% of the old samples that lost more than a quarter of that."""
    after = fill_rates(spec, schema, now, transforms)
    rows = [(f, before, after.get(f, 0.0)) for f, before in fill_rates(spec, schema, old, transforms).items()]
    return [r for r in rows if r[1] >= GONE and r[2] < r[1] * (1 - MOVED)]


def item_field(field: str) -> tuple[str, str] | None:
    """"jobs[].title" as (the list's spec key, the field inside its item map). None outside any list."""
    i = field.find("[].")
    return None if i == -1 else (field[: i + 2], field[i + 3 :])


def items_at(schema: Any, path: str) -> Any:
    """The items schema of the canonical list at a dotted path."""
    s = schema
    for k in path.split("."):
        s = s["properties"].get(k) if is_obj(s) and is_obj(s.get("properties")) else None
    return s.get("items") if is_obj(s) else None


def replace(rules: dict[str, Any], fresh: dict[str, Any], broken: list[str], at: str = "") -> list[str]:
    """Replaces each broken rule in `rules` with the re-inferred one. Returns one report line per rule."""
    lines = []
    for f in broken:
        if f not in fresh:
            lines.append(f"{pad(at + f, 28)}no replacement found; needs a person")
            continue
        lines.append(f"{pad(at + f, 28)}{js_json(rules[f])} -> {js_json(fresh[f])}")
        rules[f] = fresh[f]
    return lines


def fix_items(spec: Spec, schema: Any, now: list[Any], options: dict[str, Any], key: str, broken: list[str]) -> tuple[Any, list[str]]:
    """Fields that broke inside one list's item map. The list still comes from the same place, so only
    its elements are re-inferred, against the list's items schema, and the other item rules stay."""
    rule = spec["fields"][key]
    source = next((s for s in collect(now) if s.path == rule.get("from") and s.kind == "objects"), None)
    items = items_at(schema, key[:-2])
    # Known outputs and raw texts are per response. Elements have neither.
    opts = {k: v for k, v in options.items() if k not in ("known", "raws")}
    fresh = infer_draft(items, elements(source), provider=spec["provider"], endpoint=spec["endpoint"], **opts).spec["fields"] if source and items else {}
    item = {**(rule.get("item") or {})}
    return {**rule, "item": item}, replace(item, fresh, broken, f"{key}.")


def propose(spec: Spec, schema: Any, now: list[Any], options: dict[str, Any], dropped: list[str]) -> tuple[Spec, list[str]]:
    draft = infer_draft(schema, now, provider=spec["provider"], endpoint=spec["endpoint"], **options).spec
    fields = {**spec["fields"]}
    broken = [f for f in dropped if not item_field(f)]
    lines = replace(fields, draft["fields"], broken)
    # infer_draft above re-inferred a list that broke as a whole, with its items. Only the lists that are still there need their items fixed.
    in_lists: dict[str, list[str]] = {}
    for key, sub in (x for x in map(item_field, dropped) if x):
        if key not in broken:
            in_lists.setdefault(key, []).append(sub)
    for key, subs in in_lists.items():
        fields[key], fixed = fix_items(spec, schema, now, options, key, subs)
        lines += fixed
    # An anchor names the old key, so a re-mapped field loses it, and takes the new one if inference found one.
    anchors = {f: t for f, t in (spec.get("anchors") or {}).items() if f not in broken}
    anchors.update({f: t for f, t in (draft.get("anchors") or {}).items() if f in broken})
    nxt = {**spec, "version": spec["version"] + 1, "status": "shadow", "fields": fields}
    if anchors:
        nxt["anchors"] = anchors
    else:
        nxt.pop("anchors", None)
    return nxt, lines


def run(args: list[str], transforms: Transforms) -> int:
    p = parser(USAGE)
    for flag in ("--schema", "--spec", "--out", "--known", "--synonyms", "--min-score"):
        p.add_argument(flag)
    p.add_argument("--learn", action="append")
    p.add_argument("paths", nargs="*")
    a = p.parse_intermixed_args(args)
    if len(a.paths) != 2 or bool(a.schema) != bool(a.spec):
        raise UsageError(USAGE)
    old_files, new_files = (json_files([x]) for x in a.paths)
    old = [read_json(f) for f in old_files]
    raws = [read_text(f) for f in new_files]
    now = [parse_json(t, f) for t, f in zip(raws, new_files)]
    drift = shape_drift(old, now)
    # The report goes to stderr when stdout carries a proposed spec.
    out = sys.stderr if a.spec and not a.out else sys.stdout

    def log(line: str) -> None:
        print(line, file=out)

    for line in drift:
        log(line)
    spec_drift = field_drift(a, transforms, old, now, raws, new_files, log) if a.spec else False
    return 1 if drift or spec_drift else 0


def field_drift(a: Any, transforms: Transforms, old: list[Any], now: list[Any], raws: list[str], files: list[str], log: Callable[[str], None]) -> bool:
    """Reports spec fields whose fill dropped and writes the proposed next version. True if any dropped."""
    spec = read_json(a.spec)
    schema = read_json(a.schema)
    broken = dropped(spec, schema, old, now, transforms)
    for f, before, after in broken:
        log(f"field {f}: filled {pct(before)} -> {pct(after)}")
    if not broken:
        return False
    options = {"raws": raws, **extras(a.known, a.synonyms, a.learn, a.min_score, files, schema)}
    nxt, lines = propose(spec, schema, now, options, [f for f, _, _ in broken])
    for line in lines:
        log(line)
    text = js_json(nxt, 2) + "\n"
    if a.out:
        with open(a.out, "w", encoding="utf-8") as fh:
            fh.write(text)
    else:
        sys.stdout.write(text)
    return True
