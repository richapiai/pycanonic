"""`canonic fit`: fits every canonical node in schema order, decides the tiers, proves the spec by
compiling it and mapping every pair, and writes the spec, the report and the transform stubs."""

from __future__ import annotations

import argparse
import copy
import json
import keyword
import re
import sys
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from ... import Rejected, _api
from ..infer.ask import MAX_SOURCES, Ask, answer, ask_with, question, rule_for
from ..infer.score import Leaf
from ..infer.sources import collect
from ..io import Transforms, read_json, tolerance
from . import align, decide
from .arrays import MISSING, agree, fit_array, get
from .chains import apply_rule
from .compare import Tolerance, match, near, same
from .derive import _child, derive_schema
from .index import Occurrence, scalars
from .leaves import LeafFit, _related, _segs, fit_leaf
from .pairs import Pair, load_pairs
from .report import FitResult, Row, to_md, weak_lookup, write_json, write_md
from .structure import _paths, _rejects, fit_const, fit_object_presence, fit_when, propose_reject

FLOOR = 0.8


@dataclass
class FitOptions:
    provider: str
    endpoint: str
    min_auto: float = 0.95
    min_pairs: int = 5
    keep: dict[str, Any] = field(default_factory=dict)  # top-level field key to the approved rule
    ask: Ask | None = None
    transforms: Transforms = field(default_factory=dict)  # for kept rules that name private transforms
    tolerance: Tolerance = field(default_factory=Tolerance)


def _val(doc: Any, key: str) -> Any:
    """The value at a field key (`hq.city`, `jobs[]`, `[]`), or MISSING."""
    return get(doc, _segs(key.removesuffix("[]")))


def _cmp(a: Any, b: Any, tol: Tolerance = Tolerance()) -> str:
    """`match` that also compares MISSING by identity."""
    if MISSING in (a, b):
        return "exact" if a is b else "miss"
    return match(a, b, tol)


def _eq(a: Any, b: Any, tol: Tolerance = Tolerance()) -> bool:
    return _cmp(a, b, tol) != "miss"


def _empty(v: Any) -> bool:
    return v is MISSING or v is None or v == [] or v == {}


def short(key: str, rule: Any) -> Any:
    """A leaf rule with only a path as the bare path string, item maps included."""
    if not isinstance(rule, dict):
        return rule
    if not key.endswith("[]") and set(rule) == {"from"} and isinstance(rule["from"], str):
        return rule["from"]
    if isinstance(rule.get("item"), dict):
        return {**rule, "item": {k: short(k, r) for k, r in rule["item"].items()}}
    return rule


def _nodes(schema: dict[str, Any], prefix: str = "") -> Iterator[tuple[str, dict[str, Any]]]:
    """(field key, node) in schema order: arrays as `key[]`, objects before their children."""
    if not prefix and schema.get("type") == "array":
        yield "[]", schema
        return
    for k, sub in (schema.get("properties") or {}).items():
        sub = sub if isinstance(sub, dict) else {}
        key = _child(prefix, k)
        if sub.get("type") == "array":
            yield key + "[]", sub
        else:
            yield key, sub
            if "properties" in sub:
                yield from _nodes(sub, key)


def _row(f: LeafFit) -> Row:
    return Row(f.field, "", f.kind, short(f.field, f.rule), f.support, f.total, f.holdout, f.examples, tolerated=f.tolerated)


class _Fitter:
    def __init__(self, pairs: list[Pair], schema: dict[str, Any], opts: FitOptions, reject: list[Any], drift: bool = False) -> None:
        self.pairs, self.schema, self.opts, self.reject, self.drift = pairs, schema, opts, reject, drift
        self.sources: list[list[Occurrence]] = [scalars(p.provider) for p in pairs]
        self.fields: dict[str, Any] = {}
        self.rows: list[Row] = []
        self.inputs = any(p.inputs for p in pairs)
        self.disagree: list[str] = []  # pairs where fit's reject check and the runtime disagree

    def node(self, key: str, node: dict[str, Any]) -> None:
        start = len(self.rows)
        self._node(key, node)
        for r in self.rows[start:]:
            if r.field == key and node.get("x-mixed-empty"):
                r.mixed_empty = True
                r.note = r.note or "The default's samples disagree on the empty form."

    def _node(self, key: str, node: dict[str, Any]) -> None:
        if key in self.opts.keep:
            self.fields[key] = self.opts.keep[key]
            self.rows.append(Row(key, "", "kept", self.opts.keep[key], 0, 0, None, []))
            return
        vals = [_val(p.default, key) for p in self.pairs]
        if all(map(_empty, vals)):  # the default never sends a value: leave it to x-empty
            self.rows.append(Row(key, "", "empty", None, 0, 0, 1.0, []))
        elif key.endswith("[]"):
            self.array(key, node, vals)
        elif "oneOf" in node:  # fit learns no branch conditions: one manual row
            note = "A union: write the one_of branches by hand (each branch needs a when and its fields)."
            self.rows.append(Row(key, "", "union", None, 0, len(self.pairs), 0.0, [], note))
        elif node.get("x-open"):
            self.open_map(key, [None if v is MISSING else v for v in vals])
        elif "properties" in node:
            rule = fit_object_presence(key, self.pairs)
            if rule:
                self.fields[key] = rule
                self.rows.append(Row(key, "", "presence", rule, len(self.pairs), len(self.pairs), None, []))
            elif "x-empty" not in node and not any(map(_empty, vals)):
                # never empty in the samples: null when every leaf is null, the usual form
                self.fields[key] = rule = {"if_empty": "null"}
                note = f"The default sent this object in all {len(self.pairs)} pairs, never empty, so its empty form is a guess."
                self.rows.append(Row(key, "", "if_empty", rule, len(self.pairs), len(self.pairs), None, [], note))
        else:
            self.leaf(key, node, [None if v is MISSING else v for v in vals])

    def open_map(self, key: str, targets: list[Any]) -> None:
        """An x-open map: the provider object that the runtime copies to the default's map."""
        live = [j for j, t in enumerate(targets) if t is not None]
        hits: dict[str, set[int]] = {}
        for path in dict.fromkeys(x for p in self.pairs for x in _paths(p.provider)):
            hits[path] = {j for j in live if same(apply_rule({"from": path}, self.pairs[j].provider, {"type": "object", "x-open": True}), targets[j])}

        def best(skip: int | None = None) -> str | None:
            top = max(hits, key=lambda p: len(hits[p] - {skip}), default=None)  # first seen wins a tie
            return top if top is not None and len(hits[top] - {skip}) >= FLOOR * (len(live) - (skip is not None)) else None

        chosen = best()
        if chosen is None:
            self.rows.append(Row(key, "", "none", None, 0, len(live), 0.0, [{"pair": self.pairs[j].name, "want": targets[j], "got": None} for j in live[:3]]))
            return
        held = sum(j in hits.get(best(j) or "", set()) for j in live) / len(live)
        wrong = [j for j in live if j not in hits[chosen]]
        examples = [{"pair": self.pairs[j].name, "want": targets[j], "got": apply_rule({"from": chosen}, self.pairs[j].provider)} for j in wrong[:3]]
        self.fields[key] = chosen
        self.rows.append(Row(key, "", "path", chosen, len(hits[chosen]), len(live), held, examples))

    def array(self, key: str, node: dict[str, Any], vals: list[Any]) -> None:
        rule, fits = fit_array(key, node, self.pairs, tol=self.opts.tolerance)
        if rule is None and (const := fit_const(key, vals)):
            rule, fits = const, [LeafFit(key, const, "const", len(vals), len(vals), 1.0, [], [])]
        if rule is not None:
            self.fields[key] = short(key, rule)
        head = fits[0] if fits else LeafFit(key, None, "none", 0, 0, 0.0, [], [])
        self.rows.append(_row(head))
        for f in fits[1:]:
            # an item leaf the default never fills: the array's own evidence covers it
            f = LeafFit(f.field, None, "empty", head.support, head.total, head.holdout, [], []) if f.total == 0 else f
            self.rows.append(_row(f))

    def leaf(self, key: str, node: dict[str, Any], targets: list[Any]) -> None:
        f = fit_leaf(key, self.pairs, targets, self.sources, True, FLOOR, tol=self.opts.tolerance)
        if f.rule is not None:
            rule: dict[str, Any] = f.rule
            kind = f.kind
            if self.inputs and (cond := fit_when(key, self.pairs)):
                rule, kind = {**rule, **cond}, "when"
            self.fields[key] = short(key, rule)
            self.rows.append(Row(key, "", kind, short(key, rule), f.support, f.total, f.holdout, f.examples, tolerated=f.tolerated))
            return
        row = self.unmapped(f, targets)
        if row.kind != "absent" and self.opts.ask:
            row = self.asked(key, node, targets) or row
        self.rows.append(row)

    def unmapped(self, f: LeafFit, targets: list[Any]) -> Row:
        """disagree when some source matches part of the pairs, absent when nothing in the
        provider looks like the value, else none (needs code)."""
        alt = next((a for a in f.alternatives if a["kind"] != "const" and a["support"] > 0), None)
        if alt is not None:
            outs = [apply_rule(alt["rule"], p.provider) for p in self.pairs]
            raw = {k: v for k, v in alt["rule"].items() if k != "transform"}  # no rule is written, so show the provider's value
            live = [j for j, t in enumerate(targets) if t is not None]
            order = sorted(live, key=lambda j: same(outs[j], targets[j]))
            examples = [{"pair": self.pairs[j].name, "want": targets[j], "got": apply_rule(raw, self.pairs[j].provider)} for j in order[:3]]
            note = f"Closest rule: {json.dumps(alt['rule'])} on {alt['support']} of {f.total} pairs."
            return Row(f.field, "", "disagree", None, 0, f.total, 0.0, examples, note)
        related = any(_related(o.value, t) for t, occs in zip(targets, self.sources) if t is not None for o in occs)
        return Row(f.field, "", "none" if related else "absent", None, 0, f.total, 0.0, f.examples)

    def asked(self, key: str, node: dict[str, Any], targets: list[Any]) -> Row | None:
        """The --ask answer for a manual leaf, if it names a provider path."""
        assert self.opts.ask is not None
        srcs = [s for s in collect([p.provider for p in self.pairs]) if s.kind == "scalar"][:MAX_SOURCES]
        keys = [k for k in _segs(key) if isinstance(k, str)]
        leaf = Leaf(key, [keys[-1] if keys else key], keys[-2] if len(keys) > 1 else "", None, False, node.get("type"))
        picked = {s.path: s for s in srcs}.get(answer(self.opts.ask(question(leaf, None, srcs))) or "")
        if picked is None:
            return None
        rule = rule_for(leaf, picked)
        full = {"from": rule} if isinstance(rule, str) else rule
        outs = [apply_rule(full, p.provider) for p in self.pairs]
        live = [j for j, t in enumerate(targets) if t is not None]
        hits = [j for j in live if same(outs[j], targets[j])]
        order = sorted(live, key=lambda j: j in hits)
        examples = [{"pair": self.pairs[j].name, "want": targets[j], "got": outs[j]} for j in order[:3]]
        self.fields[key] = rule
        return Row(key, "", "asked", rule, len(hits), len(live), None, examples)


_EMPTY_FORM: dict[str, Any] = {"null": None, "[]": [], "{}": {}, "missing": None}


def _node_at(schema: dict[str, Any], key: str) -> dict[str, Any]:
    """The schema node of a field key, item fields included (`jobs[].org.id`). `jobs[]` is the array."""
    node = schema
    for i, part in enumerate(key.removesuffix("[]").split("[]")):
        node = (node.get("items") or {}) if i else node
        for s in _segs(part):
            node = (node.get("properties") or {}).get(s) or {}
    return node


def _empty_form(node: dict[str, Any], strict: bool) -> Any:
    """What the runtime writes for the node with no value: its x-empty, else the strict table, else
    null. Under x-empty "missing" a null leaves the key out."""
    if node.get("x-empty") in _EMPTY_FORM:
        return _EMPTY_FORM[node["x-empty"]]
    if not strict:
        return None
    return [] if node.get("type") == "array" else {} if node.get("x-open") else None


def _with_holes(f: _Fitter) -> tuple[dict[str, Any], dict[str, Any]]:
    """The fitted rules plus a placeholder, `{"const": <empty form>}`, for every manual field, top
    level or inside item maps, so the draft has every canonical key. Also each placed key
    with its empty form.
    An object node gets none: a const there would replace the leaves mapped below it."""
    fields, placed = copy.deepcopy(f.fields), {}
    strict = f.schema.get("x-shape") == "strict"
    for r in f.rows:
        node = _node_at(f.schema, r.field)
        if r.kind not in decide.MANUAL or r.rule is not None or "properties" in node:
            continue
        head, *rest = r.field.split("[].")
        hole = {"const": _empty_form(node, strict)}
        if not rest:
            fields.setdefault(r.field, hole)
            placed[r.field] = hole["const"]
            continue
        rule = fields.get(head + "[]")
        for part in rest[:-1]:
            rule = rule["item"].get(part + "[]") if isinstance(rule, dict) and isinstance(rule.get("item"), dict) else None
        if isinstance(rule, dict) and isinstance(rule.get("item"), dict):
            rule["item"].setdefault(rest[-1], hole)
            placed[r.field] = hole["const"]
    order = {r.field: i for i, r in enumerate(f.rows)}  # a top-level placeholder in schema order
    return dict(sorted(fields.items(), key=lambda kv: order.get(kv[0], len(order)))), placed


def _spec(opts: FitOptions, fields: dict[str, Any], reject: list[Any] | None = None, drift: bool = False) -> dict[str, Any]:
    spec: dict[str, Any] = {"language": 2, "provider": opts.provider, "endpoint": opts.endpoint, "version": 1, "status": "draft"}
    if drift:
        spec["keys"] = "case_insensitive"
    spec["fields"] = fields
    return {**spec, "reject": reject} if reject else spec


def _key_paths(v: Any, at: str, out: set[str]) -> None:
    if isinstance(v, dict):
        for k, x in v.items():
            out.add(f"{at}.{k}")
            _key_paths(x, f"{at}.{k}", out)
    elif isinstance(v, list):
        for x in v:
            _key_paths(x, at + "[]", out)


def _drifting(pairs: list[Pair]) -> list[str]:
    """The first provider key path that the pairs spell in more than one case, every spelling
    in first-seen order, or [] when no key drifts."""
    seen: dict[str, dict[str, None]] = {}
    for p in pairs:
        s: set[str] = set()
        _key_paths(p.provider, "", s)
        for k in sorted(s):
            seen.setdefault(k.lower(), {})[k] = None
    return next(([k.lstrip(".") for k in ks] for ks in seen.values() if len(ks) > 1), [])


def _map(art: Any, p: Pair) -> Any:
    return art.map(p.provider) if p.inputs is None else art.map(p.provider, context=p.inputs)


def _hit(rows: list[Row], key: str, got: Any, want: Any, tol: Tolerance = Tolerance()) -> bool:
    """The runtime reproduces the default's value. An array is judged as fit_array judges it: on the
    item leaves that have a rule, so an item leaf the provider lacks does not fail it."""
    if not key.endswith("[]"):
        return _eq(got, want, tol) or near(got, want)
    keys = [r.field[len(key) + 1 :] for r in rows if r.field.startswith(key + ".") and r.rule is not None]
    return agree(got, want, keys, tol) if keys else _eq(got, want)


def _check(f: _Fitter) -> None:
    """Maps every pair through the compiled spec. A field whose runtime output disagrees with what
    fit reported loses its rule and becomes manual, kind none, until the spec agrees with itself.
    A pair that the runtime rejects but fit did not drops the reject rules (fit_pairs makes them manual)."""
    pairs, n, tol = f.pairs, len(f.pairs), f.opts.tolerance
    while f.fields:
        art = _api.compile_spec(_spec(f.opts, _with_holes(f)[0], f.reject, f.drift), f.schema, transforms=f.opts.transforms)
        outs: list[Any] = []
        for p in pairs:
            try:
                outs.append(_map(art, p))
            except Rejected:
                f.disagree.append(p.name)
                break
        if len(outs) < n:
            f.reject = []
            continue
        bad: list[tuple[Row, int, list[Any], list[Any]]] = []
        for r in f.rows:
            if (r.field not in f.fields and r.kind != "empty") or "[]." in r.field:
                continue  # item rows: the array's own row checks them
            got = [_val(o, r.field) for o in outs]
            want = [_val(p.default, r.field) for p in pairs]
            if r.kind == "empty":
                r.support, r.total = sum(map(_eq, got, want)), n
                continue
            if r.kind in ("presence", "if_empty"):
                support = sum(_empty(g) == _empty(w) for g, w in zip(got, want))
            else:
                live = [j for j in range(n) if want[j] is not MISSING and want[j] is not None]
                support = sum(_hit(f.rows, r.field, got[j], want[j], tol) for j in live)
                if r.kind == "kept":
                    r.support, r.total = support, len(live)
            if f.drift and support > r.support:  # fit searched exact keys, the runtime also matches other cases
                r.support = support
            elif r.kind != "kept" and support != r.support:
                bad.append((r, support, got, want))
        if not bad:
            return
        for r, support, got, want in bad:
            r.note = f"Runtime check: the rule {json.dumps(r.rule)} maps {support} pairs, fit found {r.support}."
            wrong = [j for j in range(n) if not _hit(f.rows, r.field, got[j], want[j], tol) and not _empty(want[j])]
            r.examples = [{"pair": pairs[j].name, "want": want[j], "got": None if got[j] is MISSING else got[j]} for j in wrong[:3]]
            del f.fields[r.field]
            r.kind, r.rule, r.support, r.holdout, r.tolerated = "none", None, 0, 0.0, None
            if r.field.endswith("[]"):  # its item rows describe a rule the spec no longer has
                f.rows = [x for x in f.rows if not x.field.startswith(r.field + ".")]


def _doc(line: str) -> str:
    """Docstring text that reads back as `line`: backslashes and quotes escaped."""
    return line.replace("\\", "\\\\").replace('"', '\\"')


def _stub_name(key: str) -> str:
    name = re.sub(r"\W+", "_", key).strip("_") or "field"
    return f"f_{name}" if name[0].isdigit() or keyword.iskeyword(name) else name


def stubs(rows: list[Row], pairs: list[Pair], provider: str, endpoint: str) -> str:
    """One function per manual field outside arrays that needs code. Its doctests are the pairs fit
    could not reproduce, so they fail until the function is written."""
    by = {p.name: p for p in pairs}
    out = [
        f'"""Transform stubs from canonic fit for {provider} {endpoint}.',
        "",
        "Each function gets the whole provider body and returns the canonical value. Map the field",
        'with {"from": "$", "transform": "<function name>"} and pass this file with --transforms.',
        '"""',
    ]
    for r in rows:
        if r.kind != "none" or "[]" in r.field or r.field.startswith("reject["):
            continue
        name = _stub_name(r.field)
        tests = [line for e in r.examples[:3] if e["pair"] in by for line in (f"    >>> {name}({by[e['pair']].provider!r})", f"    {e['want']!r}")]
        doc = [_doc(t) for t in [f"The canonical field {r.field}.", "", *tests]]
        out += ["", "", f"def {name}(body):", f'    """{doc[0]}', *doc[1:], '    """', "    return None"]
    return "\n".join(out) + "\n"


def _defaults(doc: Any, parts: list[str]) -> Iterator[Any]:
    """The default's values for a field key split on `[]` (`jobs[].title` gives `["jobs", ".title"]`),
    one per element for item fields."""
    head, rest = parts[0].lstrip("."), parts[1:]
    v = get(doc, _segs(head)) if head else doc
    if rest in ([], [""]):
        yield v
    elif isinstance(v, list):
        for x in v:
            yield from _defaults(x, rest)


def _one_value(pairs: list[Pair], key: str) -> bool:
    """The default sends the same value, or no value, in every pair: no rule beats a constant there."""
    return len({json.dumps(v, sort_keys=True) for p in pairs for v in _defaults(p.default, key.split("[]")) if not _empty(v)}) < 2


def _fit(pairs: list[Pair], schema: dict[str, Any], options: FitOptions, reject: list[Any], drift: bool, failover: list[Pair]) -> _Fitter:
    """Fits every node and proves the spec on `pairs`, and checks that it rejects every `failover`
    pair. A pair where fit and the runtime disagree on a reject lands in `disagree`."""
    f = _Fitter(pairs, schema, options, reject, drift)
    for key, node in _nodes(schema):
        f.node(key, node)
    _check(f)
    if f.fields and f.reject:  # never write a spec that has not mapped every pair, rejects included
        art = _api.compile_spec(_spec(options, _with_holes(f)[0], f.reject, f.drift), schema, transforms=options.transforms)
        for p in failover:
            try:
                _map(art, p)
                f.disagree.append(p.name)
            except Rejected:
                pass
    return f


def fit_pairs(pairs: list[Pair], schema: dict[str, Any], options: FitOptions) -> FitResult:
    """The in-process core of `run`: fit, decide, prove. Writes nothing.

    Proposed rejects come first. A pair that they reject is a failover: it is named in
    the reject row and left out of every field's support and total. If fit's own reject check and
    the runtime disagree on a pair, the spec leaves the rejects out, their rows are manual, and
    every field is fitted again on every pair, the failovers included."""
    reject = propose_reject(schema, pairs) if schema.get("type") != "array" else []
    failover = [p for p in pairs if any(_rejects(c, p.provider) for c in reject)]
    if len(failover) == len(pairs):  # a reject that throws every answer away is no proposal
        reject, failover = [], []
    drift = _drifting(pairs)
    f = _fit([p for p in pairs if p not in failover], schema, options, reject, bool(drift), failover)
    if f.disagree:
        disagree, f = f.disagree, _fit(pairs, schema, options, [], bool(drift), [])
        f.disagree = disagree
    fields, placed = _with_holes(f)
    spec = _spec(options, fields, f.reject, f.drift) if f.fields else None
    for r in f.rows:
        if spec and r.field in placed:
            r.note = " ".join(filter(None, [r.note, f"placeholder: the draft writes {json.dumps(placed[r.field])}, the schema's empty form, until a rule replaces it."]))
    if drift:  # one decision for the whole spec, always for review
        note = f"The provider spells keys in more than one case, for example {', '.join(drift)}, so keys match without regard to case."
        f.rows.insert(0, Row("spec.keys", "", "keys", "case_insensitive", len(pairs), len(pairs), None, [], note))
    for i, cond in enumerate(reject):
        hit = [p.name for p in failover if _rejects(cond, p.provider)]
        note = f"Rejects {len(hit)} of {len(pairs)} pairs: rejects {', '.join(hit)}, where the default answered. Those pairs are left out of every other field's support."
        if f.disagree:
            note = f"fit and the runtime disagree on which pairs the reject rules reject ({', '.join(f.disagree)}), so the spec leaves them out. Every field was fitted again on all {len(pairs)} pairs."
        f.rows.append(Row(f"reject[{i}]", "", "reject", cond, len(hit), len(pairs), None, [], note))
    for r in f.rows:
        one = not r.field.startswith("reject[") and _one_value(f.pairs, r.field)
        lf = LeafFit(r.field, None, r.kind, r.support, r.total, r.holdout or 0.0, [], [], r.mixed_empty, one, r.tolerated)
        r.tier = decide.tier(lf, min_auto=options.min_auto, min_pairs=options.min_pairs, floor=FLOOR)
        if one and r.tier == "review" and r.kind in decide.AUTO:
            r.note = " ".join(filter(None, [r.note, "The default sent one value or none in every pair, so the samples cannot tell this rule from a constant."]))
        if r.tolerated and r.tier == "review":
            parts = r.tolerated.split(", ")
            flags, cut = [x for x in parts if x.startswith("--")], [x for x in parts if not x.startswith("--")]
            r.note = " ".join(filter(None, [r.note, f"matches within {', '.join(flags)}." if flags else "", f"{cut[0]}." if cut else ""]))
        if r.kind == "lookup" and weak_lookup(r.rule):
            r.note = " ".join(filter(None, [r.note, "weak, possibly by chance: a target value has fewer than two source values in the table."]))
        if r.kind == "reject" and f.disagree:
            r.tier = "manual"
    return FitResult(options.provider, options.endpoint, len(pairs), f.rows, spec, stubs(f.rows, pairs, options.provider, options.endpoint))


def exit_code(result: FitResult, strict: bool = False) -> int:
    """2 for a manual field or a skipped pair, 1 for a field to review (2 with `strict`), else 0.
    Kept fields do not count: a person already reviewed them."""
    if result.skipped or any(r.tier == "manual" for r in result.fields):
        return 2
    if any(r.tier == "review" and r.kind != "kept" for r in result.fields):
        return 2 if strict else 1
    return 0


def run(args: argparse.Namespace, transforms: Transforms | None = None) -> int:
    """`canonic fit` with parsed arguments (see commands/fit.py for the flags)."""
    pairs, skipped = load_pairs(Path(args.default), Path(args.provider), Path(args.inputs) if args.inputs else None)
    schema = read_json(args.schema) if args.schema else derive_schema([p.default for p in pairs])
    pairs, schema, aligned = align.from_args(args, pairs, schema)
    if len(pairs) < 3:
        print(f"need at least 3 pairs, got {len(pairs)}", file=sys.stderr)
        return 2
    if len(pairs) < 8:
        print(f"fit: only {len(pairs)} pairs; 8 or more that differ give rules you can trust", file=sys.stderr)
    keep = read_json(args.keep) if args.keep else {}
    if not isinstance(keep, dict):
        raise ValueError(f"{args.keep}: must be an object of field key to rule")
    known = {k for k, _ in _nodes(schema)}
    for k in keep:
        if k not in known:
            print(f'fit: --keep key "{k}" matches no field', file=sys.stderr)
    opts = FitOptions(args.provider_name, args.endpoint, args.min_auto, args.min_pairs, keep, ask_with(args.ask) if args.ask else None, transforms or {}, tolerance(args))
    result = fit_pairs(pairs, schema, opts)
    result.skipped, result.aligned = skipped, aligned
    out = Path(args.out)
    if result.spec is None:
        print("fit: no field could be mapped, so no spec was written", file=sys.stderr)
    else:
        prev = read_json(str(out)) if out.exists() else {}
        if not isinstance(prev, dict):
            print(f"fit: {out} is not a JSON object, so changed rules are not compared", file=sys.stderr)
        old = prev.get("fields") if isinstance(prev, dict) and out.exists() else None
        if isinstance(old, dict):
            new = result.spec["fields"]
            for key in dict.fromkeys([*old, *new]):
                if key not in keep and old.get(key) != new.get(key):
                    print(f"fit: {key} changed since the last spec; add it to --keep to hold it", file=sys.stderr)
        out.write_text(json.dumps(result.spec, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    if args.report:
        write_md(result, f"{args.report}.md")
        write_json(result, f"{args.report}.json")
    else:
        sys.stdout.write(to_md(result))
    if args.transforms_out:
        Path(args.transforms_out).write_text(result.transforms, encoding="utf-8")
    s = result.summary()
    print(f"fit: {s['auto']} auto, {s['review']} review, {s['manual']} manual", file=sys.stderr)
    return exit_code(result, args.strict)
