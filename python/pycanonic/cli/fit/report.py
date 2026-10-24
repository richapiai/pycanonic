"""The `fit` report: what was decided per field, the evidence, and what can break (design table
"What is decided automatically, and what can break"), as JSON and as Markdown."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

_TRANSFORM = "A value format that the samples did not show, such as a date without a month. The transform returns null and counts an error. Watch the error counts after launch."

# The design table's texts, verbatim. Kinds with no row of their own: the other transforms share the
# transform row, and join, wrap, flatten, filter, null_if, keys and asked have short texts of their own.
RISKS: dict[str, str] = {
    "path": "The provider renames a key later. Caught by drift detection and the fingerprint, not by `fit`.",
    **{k: _TRANSFORM for k in ["cast", "trim", "lower", "upper", "format_date", "parse_date", "split", "split_name", "title", "to_string", "parse_number", "parse_range", "to_domain", "normalize_phone", "normalize_linkedin"]},
    "strip_prefix": "A record with a different prefix maps to null.",
    "strip_suffix": "A record with a different prefix maps to null.",
    "empty": "The default provider itself is inconsistent (sometimes null, sometimes `[]`). The schema flags fields where samples disagree, and those fields go to review.",
    "coalesce": "The second source has different meaning on records the samples did not cover.",
    "lookup": 'A value outside the table. `"else"` decides: keep (wrong vocabulary passes through), null (data lost), or a fixed value.',
    "const": "The default's value only looked constant in the samples. The most common cause of silent wrong data. Add pairs that differ.",
    "regex": "A URL or string format the samples did not show.",
    "group": "The grouping key is wrong for records with gaps or repeats, for example two stints at one company with another job between them. Check with a pair that has that pattern.",
    "presence": "If the provider sends the parent object with different content, the object can be null by mistake.",
    "when": "An input combination the samples did not show.",
    "omit": "Customers parsing a fixed key set.",
    "reject": "Too strict: good answers fail over to the next provider and cost money. Too loose: empty answers reach customers.",
    "absent": "Customers that filter on that field get nulls. A business decision, not a mapping one.",
    "disagree": "A value mismatch that a lookup table cannot fix, for example old and new industry names. Decide per field: lookup table, custom transform, or accept the difference.",
    "none": "Covered by the generated stub and its doctests.",
    "join": "A record with one part missing, or with a different separator, maps to a wrong or partial value.",
    "wrap": "A provider that sometimes sends a list here gets a list inside a list.",
    "flatten": "Elements from inner arrays that the samples did not show get mixed in.",
    "filter": "A status value that the samples did not show drops the element, or keeps it.",
    "null_if": "A real value that equals the null marker is lost.",
    "keys": "Two keys that differ only in case: the first one in the body wins.",
    "kept": "kept by reviewer",
    "asked": "A model chose this source path. Check it against the examples.",
}


@dataclass
class Row:
    """One field of the report: a top-level spec key, an item leaf (`jobs[].title`) or `reject[i]`."""

    field: str
    tier: str
    kind: str
    rule: Any
    support: int
    total: int
    holdout: float
    examples: list[dict[str, Any]]
    note: str = ""

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

    def summary(self) -> dict[str, int]:
        return {t: sum(r.tier == t for r in self.fields) for t in ("auto", "review", "manual")}


def to_json(result: FitResult) -> dict[str, Any]:
    return {
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


def write_json(result: FitResult, path: str | Path) -> None:
    Path(path).write_text(json.dumps(to_json(result), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def _code(v: Any) -> str:
    return "`" + json.dumps(v, ensure_ascii=False).replace("`", "'").replace("|", "\\|") + "`"


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
    auto = [r for r in result.fields if r.tier == "auto"]
    if auto:
        out += ["## Auto", "", "| field | kind | rule | pairs | holdout |", "|---|---|---|---|---|"]
        out += [f"| {r.field} | {r.kind} | {_code(r.rule)} | {r.support} of {r.total} | {r.holdout:.0%} |" for r in auto]
        out.append("")
    for r in result.fields:
        if r.tier == "auto":
            continue
        out += [f"## {r.field}: {r.tier}, {_kind(r)}", "", f"- rule: {_code(r.rule)}", f"- evidence: {r.support} of {r.total} pairs, holdout {r.holdout:.0%}"]
        out += [f"- {e['pair']}: default {_code(e['want'])}, provider {_code(e['got'])}" for e in r.examples[:3]]
        out += [f"- what can break: {r.risk()}", ""]
    return "\n".join(out)


def write_md(result: FitResult, path: str | Path) -> None:
    Path(path).write_text(to_md(result), encoding="utf-8")
