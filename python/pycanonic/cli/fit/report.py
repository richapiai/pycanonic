"""The `fit` report: what was decided per field, the evidence, and what can break (design table
"What is decided automatically, and what can break"), as JSON and as Markdown."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import align

_TRANSFORM = "A value format that the samples did not show, such as a date without a month. The transform returns null and counts an error. Watch the error counts after launch."

# The design table's texts, verbatim except the strip rows: a value without the prefix passes
# through unchanged, it does not map to null. Kinds with no row of their own: the other transforms
# share the transform row, and join, wrap, flatten, filter, null_if, keys, if_empty and asked have
# short texts of their own.
RISKS: dict[str, str] = {
    "path": "The provider renames a key later. Caught by drift detection and the fingerprint, not by `fit`.",
    **{k: _TRANSFORM for k in ["cast", "trim", "lower", "upper", "format_date", "parse_date", "split", "split_name", "title", "to_string", "parse_number", "parse_range", "to_domain", "normalize_phone", "normalize_linkedin"]},
    "strip_prefix": "A record with a different prefix keeps it, so the value comes out with that prefix still on.",
    "strip_suffix": "A record with a different suffix keeps it, so the value comes out with that suffix still on.",
    "empty": "The default provider itself is inconsistent (sometimes null, sometimes `[]`). The schema flags fields where samples disagree, and those fields go to review.",
    "coalesce": "The second source has different meaning on records the samples did not cover.",
    "lookup": 'A value outside the table. `"else"` decides: keep (wrong vocabulary passes through), null (data lost), or a fixed value.',
    "const": "The default's value only looked constant in the samples. The most common cause of silent wrong data. Add pairs that differ.",
    "format": "A record whose value needs a different prefix or suffix gets this one, so the text comes out wrong without an error.",
    "regex": "A URL or string format the samples did not show.",
    "group": "The grouping key is wrong for records with gaps or repeats, for example two runs under one key with another element between them. Check with a pair that has that pattern.",
    "presence": "If the provider sends the parent object with different content, the object can be null by mistake.",
    "if_empty": "If every leaf in it is null, the object is null. If the default sends {} or an object of nulls there, the output differs.",
    "when": "An input combination the samples did not show.",
    "omit": "Customers parsing a fixed key set.",
    "reject": "Too strict: good answers fail over to the next provider and cost money. Too loose: empty answers reach customers.",
    "absent": "Customers that filter on that field get nulls. A business decision, not a mapping one.",
    "disagree": "A value mismatch that a lookup table cannot fix, for example old and new industry names. Decide per field: lookup table, custom transform, or accept the difference.",
    "union": "The draft writes the union's empty form until you write the one_of branches.",
    "none": "Covered by the generated stub and its doctests.",
    "join": "A record with one part missing, or with a different separator, maps to a wrong or partial value.",
    "wrap": "A provider that sometimes sends a list here gets a list inside a list.",
    "flatten": "Elements from inner arrays that the samples did not show get mixed in.",
    "filter": "A status value that the samples did not show drops the element, or keeps it.",
    "null_if": "A real value that equals the null marker is lost.",
    "keys": "Two keys that differ only in case: the exact key wins, else the first key in the body that matches without regard to case.",
    "kept": "kept by reviewer",
    "asked": "A model chose this source path. Check it against the examples.",
}


def weak_lookup(rule: Any) -> bool:
    """A lookup table in which some target value has fewer than two distinct source values."""
    step = next((t["lookup"] for t in (rule or {}).get("transform", []) if isinstance(t, dict) and "lookup" in t), None) if isinstance(rule, dict) else None
    if not isinstance(step, dict):
        return False
    by: dict[str, set[str]] = {}
    for k, t in step["table"].items():
        by.setdefault(json.dumps(t, sort_keys=True), set()).add(k)
    return any(len(v) < 2 for v in by.values())


@dataclass
class Row:
    """One field of the report: a top-level spec key, an item leaf (`jobs[].title`) or `reject[i]`."""

    field: str
    tier: str
    kind: str
    rule: Any
    support: int
    total: int
    holdout: float | None  # None: not measured (kept, asked, presence, if_empty, keys, reject)
    examples: list[dict[str, Any]]
    note: str = ""
    mixed_empty: bool = False
    tolerated: str | None = None  # the flags some pair matched only within

    def risk(self) -> str:
        text = RISKS.get(self.kind, _TRANSFORM)
        return f"{self.note} {text}" if self.note else text


@dataclass
class FitResult:
    provider: str
    endpoint: str
    pairs: int
    fields: list[Row]
    spec: dict[str, Any] | None  # None when no field could be mapped (a spec needs one field)
    transforms: str  # the stub module text
    skipped: list[tuple[str, str]] = field(default_factory=list)
    aligned: list[align.AlignCount] = field(default_factory=list)

    def summary(self) -> dict[str, int]:
        return {t: sum(r.tier == t for r in self.fields) for t in ("auto", "review", "manual")}


def to_json(result: FitResult) -> dict[str, Any]:
    out = {
        "provider": result.provider,
        "endpoint": result.endpoint,
        "pairs": result.pairs,
        "skipped": [{"name": n, "reason": r} for n, r in result.skipped],
        "fields": [
            {"field": r.field, "tier": r.tier, "kind": r.kind, "rule": r.rule, "support": r.support, "total": r.total, "holdout": r.holdout, "examples": r.examples, "risk": r.risk()}
            for r in result.fields
        ],
        "summary": result.summary(),
    }
    return {**out, "aligned": align.to_json(result.aligned)} if result.aligned else out


def write_json(result: FitResult, path: str | Path) -> None:
    Path(path).write_text(json.dumps(to_json(result), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def _code(v: Any) -> str:
    return "`" + json.dumps(v, ensure_ascii=False).replace("`", "'").replace("|", "\\|") + "`"


def _held(r: Row) -> str:
    return "n/a" if r.holdout is None else f"{r.holdout:.0%}"


def _kind(r: Row) -> str:
    return "kept by reviewer" if r.kind == "kept" else r.kind


def to_md(result: FitResult) -> str:
    s = result.summary()
    skipped = f", {len(result.skipped)} skipped" if result.skipped else ""
    out = [f"# fit: {result.provider} {result.endpoint}", "", f"{result.pairs} pairs{skipped}. {s['auto']} auto, {s['review']} review, {s['manual']} manual.", ""]
    for name, why in result.skipped:
        out.append(f"- skipped {name}: {why}")
    if result.skipped:
        out.append("")
    if result.aligned:
        out += [f"- {line}" for line in align.lines(result.aligned)] + [""]
    auto = [r for r in result.fields if r.tier == "auto"]
    if auto:
        out += ["## Auto", "", "| field | kind | rule | pairs | holdout |", "|---|---|---|---|---|"]
        out += [f"| {r.field} | {r.kind} | {_code(r.rule)} | {r.support} of {r.total} | {_held(r)} |" for r in auto]
        out.append("")
    for r in result.fields:
        if r.tier == "auto":
            continue
        out += [f"## {r.field}: {r.tier}, {_kind(r)}", "", f"- rule: {_code(r.rule)}", f"- evidence: {r.support} of {r.total} pairs, holdout {_held(r)}"]
        # with a rule, `got` is what the spec maps; without one, the provider's own value
        side = "mapped" if r.rule is not None else "provider"
        out += [f"- {e['pair']}: default {_code(e['want'])}, {side} {_code(e['got'])}" for e in r.examples[:3]]
        out += [f"- what can break: {r.risk()}", ""]
    return "\n".join(out)


def write_md(result: FitResult, path: str | Path) -> None:
    Path(path).write_text(to_md(result), encoding="utf-8")
