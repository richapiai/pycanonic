"""The optional model pass. Scalar fields that inference left unmapped, or matched below the
confidence bar, go to an ask function (the CLI's --ask pipes each question to a command). It
answers with a source path or None; an answer that isn't one of the sample paths is ignored.
The report always flags an answer for a person to check."""

from __future__ import annotations

import json
import subprocess
import sys
from collections.abc import Callable
from dataclasses import replace
from typing import Any

from ..._api import compile_spec
from ..io import js_json, js_trim
from .infer import FieldReport, Inferred
from .score import Leaf, steps_for, walk_canonical
from .sources import Source, collect

Question = dict[str, Any]
"""{instruction, field, type?, semantic?, current?, sources: [{path, examples}]}; `current` is the
source inference picked, when it picked one with low confidence."""

Ask = Callable[[Question], "str | None"]
"""Returns the reply text, or None when there is no answer."""

LOW = 0.6
"""Matches below this confidence are flagged for review, and asked about."""
INSTRUCTION = "Which source path holds this canonical field? Reply with exactly one path from sources, or null if none of them does."
# the first 200 scalar paths only; rank them by score first if real payloads need more
MAX_SOURCES = 200


def _no_constants(name: str) -> Any:
    raise ValueError(name)  # JSON.parse has no NaN or Infinity


def answer(reply: str | None) -> str | None:
    """The path in a reply: the last non-empty line, as a JSON string or bare."""
    line = js_trim(js_trim(reply).split("\n")[-1]) if reply is not None else ""
    if not line or line == "null":
        return None
    try:
        v = json.loads(line, parse_constant=_no_constants)
    except ValueError:
        return line
    return v if isinstance(v, str) else None


def _distinct(values: list[Any]) -> list[Any]:
    """[...new Set(values)]: SameValueZero, so 1 and 1.0 are one value but True and 1 are two."""
    seen: set[tuple[bool, Any]] = set()
    out = []
    for v in values:
        key = (isinstance(v, bool), v)
        if key not in seen:
            seen.add(key)
            out.append(v)
    return out


def question(leaf: Leaf, current: str | None, sources: list[Source]) -> Question:
    q: Question = {"instruction": INSTRUCTION, "field": leaf.path}
    if leaf.type:
        q["type"] = leaf.type
    if leaf.semantic:
        q["semantic"] = leaf.semantic
    if current:
        q["current"] = current
    q["sources"] = [{"path": s.path, "examples": _distinct([v for v in s.values if v is not None])[:3]} for s in sources]
    return q


def from_of(rule: Any) -> Any:
    return rule if isinstance(rule, str) else rule.get("from") if isinstance(rule, dict) else None


def doubtful(r: Inferred) -> dict[str, tuple[str | None, float]]:
    """Fields worth asking about, with the low-confidence source inference chose, if any."""
    out: dict[str, tuple[str | None, float]] = {}
    for u in r.unmapped:
        out[u["field"]] = (None, u.get("score") or 0)
    for f in r.fields:
        frm = from_of(f.rule)
        if f.how == "match" and f.confidence is not None and f.confidence < LOW and isinstance(frm, str):
            out[f.field] = (frm, f.confidence)
    return out


def rule_for(leaf: Leaf, s: Source) -> Any:
    steps = steps_for(leaf, s)
    return {"from": s.path, "transform": steps[0] if len(steps) == 1 else steps} if steps else s.path


def ask_low_confidence(r: Inferred, schema: Any, samples: list[Any], ask: Ask) -> Inferred:
    leaves = {l.path: l for l in walk_canonical(schema)[0]}
    sources = [s for s in collect(samples) if s.kind == "scalar"][:MAX_SOURCES]
    by_path = {s.path: s for s in sources}
    fields = dict(r.spec["fields"])
    report = list(r.fields)
    anchors = dict(r.spec.get("anchors") or {})
    used = {f for f in map(from_of, fields.values()) if isinstance(f, str)}
    unmapped = r.unmapped
    for field, (current, confidence) in doubtful(r).items():
        leaf = leaves.get(field)
        if not leaf:
            continue
        picked = by_path.get(answer(ask(question(leaf, current, sources))) or "")
        if not picked or picked.path in used:
            continue  # the current pick, or a source another field reads
        used.add(picked.path)
        rule = rule_for(leaf, picked)
        fields[field] = rule
        entry = FieldReport(field, "ask", rule, confidence)
        at = next((i for i, f in enumerate(report) if f.field == field), -1)
        if at == -1:
            report.append(entry)
        else:
            report[at] = entry
        unmapped = [u for u in unmapped if u["field"] != field]
        anchors.pop(field, None)  # its key token belonged to the old source
    spec = {**r.spec, "fields": fields}
    if anchors:
        spec["anchors"] = anchors
    else:
        spec.pop("anchors", None)
    if fields:
        compile_spec(spec, schema)  # an empty draft isn't a valid spec, as in infer_draft()
    return replace(r, spec=spec, fields=report, unmapped=unmapped)


def ask_with(command: str) -> Ask:
    """Runs the --ask command once per question. A failing command skips that field; it's never required."""

    def run(q: Question) -> str | None:
        try:
            done = subprocess.run(command, shell=True, input=js_json(q), capture_output=True, text=True, encoding="utf-8", errors="replace")
            # Lossy, as node decodes: a stray byte makes the answer unknown, not the run fail.
        except OSError as e:
            print(f"--ask failed for {q['field']}: {e}", file=sys.stderr)
            return None
        if done.returncode == 0:
            return done.stdout
        print(f"--ask failed for {q['field']}: {(done.stderr or str(done.returncode)).strip()}", file=sys.stderr)
        return None

    return run
