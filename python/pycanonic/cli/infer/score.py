"""Scoring one canonical leaf against one source path, from three signals: name tokens, the shape of
the sample values, and (when known entities are given) whether the values are the known ones."""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any, Union

from ..._api import default_transforms as T
from ..._api import validators as V
from ..io import JS_WS, get, is_obj, js_json, js_keys, js_trim
from .sources import Source, fill
from .tokens import WRAPPERS, Synonyms, similarity, split, tokens

Step = Union[str, dict[str, Any]]
Steps = list[Step]


@dataclass
class Leaf:
    path: str  # canonical path, "company.domain"
    names: list[str]  # own name, then aliases
    parent: str  # enclosing canonical object's name, "" at the root
    semantic: str | None
    routing: bool
    type: Any


@dataclass
class ArrayField:
    path: str
    name: str
    item: Any  # the items schema; an object schema for item, anything else for pluck
    objects: bool


def type_of(s: dict[str, Any]) -> Any:
    t = s.get("type")
    return next((x for x in t if x != "null"), None) if isinstance(t, list) else t


def to_leaf(s: dict[str, Any], name: str, path: str, parent: str) -> Leaf:
    aliases = [a for a in s["aliases"] if isinstance(a, str)] if isinstance(s.get("aliases"), list) else []
    semantic = s["semantic"] if isinstance(s.get("semantic"), str) else None
    return Leaf(path, [name, *aliases], parent, semantic, s.get("routing") is True, type_of(s))


def walk_canonical(schema: Any, prefix: str = "", parent: str = "") -> tuple[list[Leaf], list[ArrayField]]:
    """Leaves and arrays of a canonical JSON Schema object, in schema order. Arrays aren't descended."""
    leaves: list[Leaf] = []
    arrays: list[ArrayField] = []
    props = schema["properties"] if is_obj(schema) and is_obj(schema.get("properties")) else {}
    for name in js_keys(props):
        s = props[name]
        if not is_obj(s):
            continue
        path = prefix + name
        if is_obj(s.get("properties")):
            inner_leaves, inner_arrays = walk_canonical(s, f"{path}.", name)
            leaves += inner_leaves
            arrays += inner_arrays
        elif type_of(s) == "array":
            items = s.get("items")
            arrays.append(ArrayField(path, name, items, is_obj(items) and is_obj(items.get("properties"))))
        else:
            leaves.append(to_leaf(s, name, path, parent))
    return leaves, arrays


# The transforms that bring a value of each semantic type into canonical form.
SEMANTIC_STEPS: dict[str, Steps] = {
    "email": ["trim", "lower"],
    "phone": ["normalize_phone"],
    "domain": ["to_domain"],
    "linkedin_url": ["normalize_linkedin"],
    "url": ["trim"],
    "iso_date": ["parse_date"],
    "epoch": ["parse_date"],
}


def apply(steps: Steps, v: Any) -> Any:
    for st in steps:
        if v is None:
            return None
        if isinstance(st, str):
            name, arg = st, None
        else:
            name = js_keys(st)[0]
            arg = st[name]
        v = T[name](v, arg)
    return v


def is_num(v: Any) -> bool:
    """`typeof v === "number"`: a bool is not one."""
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def present(s: Source) -> list[Any]:
    return [v for v in s.values if v is not None]


def share(vals: list[Any], ok: Callable[[Any], bool]) -> float:
    return len([v for v in vals if ok(v)]) / len(vals) if vals else 0


def is_numeric(v: Any) -> bool:
    # JavaScript's Number() as the core's `cast number` reads it: decimal with an optional exponent, no 0x.
    return isinstance(v, str) and js_trim(v) != "" and T["cast"](v, "number") is not None


_PHONE_MARK = re.compile(f"[+(\\-{JS_WS}]")


def looks_phone(v: Any) -> bool:
    return isinstance(v, str) and bool(_PHONE_MARK.search(v)) and len(re.findall("[0-9]", v)) >= 7 and V["phone"](v) is not None


_JS_SPACE = re.compile(f"[{JS_WS}]+")


def _word(w: str) -> bool:
    # \p{L}[\p{L}'.-]*: str.isalpha() is Unicode categories L*, the same set as \p{L}.
    return w[:1].isalpha() and all(c.isalpha() or c in "'.-" for c in w[1:])


def looks_full_name(v: Any) -> bool:
    """A value that looks like a person's full name: two or more words of letters."""
    if not isinstance(v, str):
        return False
    words = _JS_SPACE.split(js_trim(v))
    return len(words) >= 2 and all(_word(w) for w in words)


def looks_epoch(v: Any) -> bool:
    """Seconds or milliseconds between 2001 and 2033: counts and IDs rarely land in both ranges' shape."""
    return is_num(v) and (isinstance(v, int) or v.is_integer()) and (1e9 <= v < 2e9 or 1e12 <= v < 2e12)


# Checked in this order: a LinkedIn URL is also a URL, and a date's digits also pass as a phone.
DETECT: list[tuple[str, Callable[[Any], bool]]] = [
    ("email", lambda v: V["email"](v) is not None),
    ("linkedin_url", lambda v: V["linkedin_url"](v) is not None),
    ("url", lambda v: V["url"](v) is not None),
    ("iso_date", lambda v: isinstance(v, str) and V["iso_date"](v) is not None and len(v) >= 7),
    ("epoch", looks_epoch),
    ("phone", looks_phone),
]

# The word a canonical name must contain before a detected type is trusted for it.
IMPLIED = {"email": "email", "phone": "phone", "url": "url", "linkedin_url": "linkedin", "domain": "domain"}
DATE_WORDS = ["date", "at", "on", "time", "day", "year", "founded", "updated", "created", "modified", "start", "end", "since", "until", "birth", "dob"]


def datey(l: Leaf) -> bool:
    return any(w in DATE_WORDS for n in l.names for w in split(n))


def semantic_of(l: Leaf, s: Source) -> str | None:
    """The leaf's semantic type; without one, the type detected in the values when the leaf's name agrees
    ("linkedin_url" with LinkedIn URLs, "updated_at" with dates or epoch timestamps)."""
    if l.semantic:
        return l.semantic
    seen = detect(s)
    if not seen:
        return None
    if seen in ("iso_date", "epoch"):
        return seen if l.type != "number" and l.type != "integer" and datey(l) else None
    word = IMPLIED.get(seen)
    return seen if word and any(word in n.lower() for n in l.names) else None


def detect(s: Source) -> str | None:
    """The semantic type that at least 80% of the source's values have, if any."""
    vals = present(s)
    return next((name for name, ok in DETECT if share(vals, ok) >= 0.8), None)


def cast_for(type: Any, vals: list[Any]) -> Step | None:
    if type in ("integer", "number") and any(isinstance(v, str) for v in vals):
        return {"cast": type}
    if type == "boolean" and any(not isinstance(v, bool) for v in vals):
        return {"cast": "boolean"}
    if type == "string" and any(is_num(v) for v in vals):
        return {"cast": "string"}
    return None


def steps_for(l: Leaf, s: Source) -> Steps:
    """The transform steps for mapping source s into leaf l, chosen from the observed values."""
    vals = present(s)
    sem = semantic_of(l, s)
    if sem and sem in SEMANTIC_STEPS:
        return SEMANTIC_STEPS[sem]
    cast = cast_for(l.type, vals)
    if cast:
        return [cast]
    padded = any(isinstance(v, str) and v != js_trim(v) for v in vals)
    return ["trim"] if padded else []


def _boolish(v: Any) -> bool:
    # ["true", "false", "yes", "no", 0, 1].includes(v): strict equality, so True is not 1.
    return isinstance(v, bool) or v in ("true", "false", "yes", "no") or (is_num(v) and v in (0, 1))


def type_score(type: Any, vals: list[Any]) -> float:
    if type in ("integer", "number"):
        return share(vals, lambda v: is_num(v) or is_numeric(v))
    if type == "boolean":
        return share(vals, _boolish)
    if type == "string":
        return 0.7 * share(vals, lambda v: isinstance(v, str)) + 0.4 * share(vals, is_num)
    return 0.5


def value_score(l: Leaf, s: Source) -> float:
    """0..1: how well the values fit the leaf. Semantic leaves check their validator after the transforms."""
    vals = present(s)
    sem = semantic_of(l, s)
    validate = V.get("iso_date" if sem == "epoch" else sem) if sem else None
    if validate:
        steps = steps_for(l, s)
        return share(vals, lambda v: validate(apply(steps, v)) is not None)
    # A column of emails or URLs is unlikely to be a plain text field that happens to share a name.
    if detect(s):
        return 0.2
    if l.semantic == "full_name" or any("full" in split(n) and "name" in split(n) for n in l.names):
        return share(vals, looks_full_name)
    return type_score(l.type, vals)


def last_key(s: Source) -> str:
    return next((x for x in reversed(s.segs) if isinstance(x, str)), "")


def parent_key(s: Source) -> str:
    keys = [x for x in s.segs if isinstance(x, str)]
    return keys[-2] if len(keys) > 1 else ""


@dataclass
class ScoreCtx:
    syn: Synonyms
    known: list[Any] | None
    min: float = 0.4
    noise: dict[str, str] = field(default_factory=dict)
    """Key prefixes shared across this provider's keys: "c" in c_nm, c_dom -> "company" when it's the
    initial of one canonical object, "" (dropped) otherwise, like "p" in p_fn, p_ln."""
    objects: set[str] = field(default_factory=set)
    """Tokens of the canonical objects (company, location): a path naming one belongs to it."""


def key_tokens(k: str, ctx: ScoreCtx) -> list[str]:
    return [x for t in tokens(k, ctx.syn) for x in (([ctx.noise[t]] if ctx.noise[t] else []) if t in ctx.noise else [t])]


def path_tokens(s: Source, ctx: ScoreCtx) -> list[str]:
    """Tokens of every key along the source path, wrappers like "data" left out."""
    return [t for k in s.segs if isinstance(k, str) for t in key_tokens(k, ctx) if t not in WRAPPERS]


def name_score(l: Leaf, s: Source, ctx: ScoreCtx) -> float:
    """0..1 from names: the better of (the leaf's name or an alias against the source key, plus the parent
    names) and (the whole canonical path against the whole source path, for "data.company.headcount.value")."""
    key = key_tokens(last_key(s), ctx)
    sp = parent_key(s)
    parent = similarity(tokens(l.parent, ctx.syn), key_tokens(sp, ctx)) if l.parent and sp else (0.3 if l.parent or sp else 1)
    along = path_tokens(s, ctx)
    up = tokens(l.parent, ctx.syn)
    return max(
        max(0.8 * similarity(tokens(n, ctx.syn), key) + 0.2 * parent, similarity([*up, *tokens(n, ctx.syn)], along)) for n in l.names
    )


def same(a: Any, b: Any) -> bool:
    """Known values are canonical output already, so they're compared exactly (`===` or equal JSON)."""
    return js_json(a) == js_json(b)


def _at(xs: list[Any], i: int) -> Any:
    return xs[i] if i < len(xs) else None


def overlap_with(l: Leaf, s: Source, known: list[Any] | None, steps: Steps) -> float | None:
    """Share of samples with a known value where the transformed source value equals it; None with no known values."""
    if known is None:
        return None
    n = hits = 0
    for i, k in enumerate(known):
        want = get(k, l.path)
        if want is None:
            continue
        n += 1
        if same(apply(steps, _at(s.values, i)), want):
            hits += 1
    return hits / n if n else None


# Known values show choices the samples alone can't, like lowercasing an industry name.
VARIANTS: list[Steps] = [[], ["trim"], ["lower"], ["trim", "lower"]]


def choose_steps(l: Leaf, s: Source, known: list[Any] | None) -> Steps:
    """steps_for, or with known values the variant that reproduces the most of them (ties keep steps_for)."""
    base = steps_for(l, s)
    if known is None or any(not isinstance(v, str) for v in present(s)):
        return base
    best = base
    top = overlap_with(l, s, known, base) or 0
    for extra in VARIANTS:
        steps = [*base, *(x for x in extra if x not in base)]
        o = overlap_with(l, s, known, steps) or 0
        if o > top:
            best, top = steps, o
    return best


def backs(l: Leaf, main: Source, c: Source, ctx: ScoreCtx) -> bool:
    """Whether source c can back up source main for leaf l: it has values where main is empty, and on those
    samples it matches the known values, or without them it passes the leaf's validator and shares a
    name token anywhere along its path ("emails[0].address" for email)."""
    syn, known = ctx.syn, ctx.known
    gaps = [i for i, v in enumerate(c.values) if v is not None and main.values[i] is None]
    if not gaps:
        return False
    steps = choose_steps(l, main, known)
    checked = [i for i in gaps if get(_at(known, i), l.path) is not None] if known is not None else []
    if checked:
        assert known is not None
        return share(checked, lambda i: same(apply(steps, c.values[i]), get(_at(known, i), l.path))) >= 0.8
    named = any(similarity(tokens(n, syn), path_tokens(c, ctx)) > 0 for n in l.names)
    return named and semantic_of(l, c) is not None and value_score(l, c) >= 0.8


def overlap_score(l: Leaf, s: Source, known: list[Any] | None) -> float | None:
    return overlap_with(l, s, known, choose_steps(l, s, known))


@dataclass
class Scored:
    score: float
    name: float
    value: float
    overlap: float | None = None


def is_first_or_last(l: Leaf, syn: Synonyms) -> bool:
    return any(t in tokens(n, syn) for n in l.names for t in ("first", "last"))


def combine(typed: bool, name: float, value: float, overlap: float | None) -> float:
    """Known values outweigh everything; for typed fields (email, phone) the values carry half; otherwise names
    lead. Dates count as untyped: every date column passes a date check, so only the name tells them apart."""
    if overlap is not None:
        return 0.6 * overlap + 0.25 * name + 0.15 * value
    if not typed:
        return 0.7 * name + 0.3 * value
    return 0.5 * value * name if value < 0.5 else 0.5 * value + 0.5 * name


def typed(sem: str | None) -> bool:
    return sem is not None and sem != "iso_date" and sem != "epoch"


def structure(l: Leaf, s: Source, ctx: ScoreCtx) -> float:
    """Multipliers for structure: what the path says about where the value belongs."""
    f = 1.0
    # One element of a list is weak evidence for a single-valued field; it's fine as a coalesce backup.
    if any(isinstance(x, int) for x in s.segs):
        f *= 0.5
    # A path through another canonical object ("org_name" for full_name) belongs to that object.
    own = {*tokens(l.parent, ctx.syn), *(t for n in l.names for t in tokens(n, ctx.syn))}
    if any(t in ctx.objects and t not in own for t in path_tokens(s, ctx)):
        f *= 0.6
    # A full name mapped straight into first_name is wrong; the split pass handles it.
    if is_first_or_last(l, ctx.syn) and share(present(s), looks_full_name) >= 0.8:
        f *= 0.5
    return f


def score(l: Leaf, s: Source, ctx: ScoreCtx) -> Scored:
    if s.kind != "scalar" or fill(s) == 0:
        return Scored(0, 0, 0)
    name = name_score(l, s, ctx)
    value = value_score(l, s)
    overlap = overlap_score(l, s, ctx.known)
    total = combine(typed(semantic_of(l, s)), name, value, overlap) * structure(l, s, ctx)
    if l.routing and value < 0.8:
        total *= value
    return Scored(total, name, value, overlap)
