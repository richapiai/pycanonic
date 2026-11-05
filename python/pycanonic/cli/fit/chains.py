"""Transform chains of length 0, 1 or 2, with arguments learned from one source value and its target.

Chains run through a compiled one-field language 2 spec, so `fit` and the runtime agree on every
result, and a chain's regex compiles once, not once per value."""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any

from ... import _api
from .compare import Tolerance, match, same

# The design's safe list first, then the transforms that always go to review. `lookup` is learned
# over all pairs in leaves.py. `null_if` cannot produce a non-null target, and `replace` is covered
# by `regex`.
SAFE = ["cast", "trim", "lower", "upper", "format_date", "parse_date", "strip_prefix", "strip_suffix", "join"]
ORDER = [*SAFE, "to_string", "title", "split", "split_name", "parse_number", "parse_range", "to_domain", "normalize_phone", "normalize_linkedin", "regex"]

_FIXED: dict[str, list[Any]] = {
    "cast": ["integer", "number", "boolean", "string"],
    "format_date": ["date", "datetime", "datetime_ms", "epoch_s", "epoch_ms"],
    "parse_date": [None, *({"formats": [f]} for f in ["%B %Y", "%b %Y", "%m/%Y", "%m/%d/%Y", "%d/%m/%Y", "%d %B %Y", "%d %b %Y"])],
    "split_name": ["first", "last"],
    "parse_number": [None, {"units": True}],
    "parse_range": ["min", "max"],
}
_NUMBER_OUT = {("cast", "integer"), ("cast", "number"), ("format_date", "epoch_s"), ("format_date", "epoch_ms")}
_SCHEMA = {"type": "object", "properties": {"v": {}}}
_META = re.compile(r"([\\.+*?()|\[\]{}^$#&\-~])")
_SEP = re.compile(r"[^A-Za-z0-9]+")


@dataclass
class Chain:
    steps: list[tuple[str, Any]] = field(default_factory=list)

    def spec(self) -> list[Any]:
        """The steps in spec syntax: `"trim"` or `{"cast": "integer"}`."""
        return [n if a is None else {n: a} for n, a in self.steps]


# Unbounded, one artifact per distinct chain in the run; an LRU bound if a run grows large.
_compiled: dict[str, Any] = {}


def apply(chain: Chain, v: Any) -> Any:
    """`v` through the chain as the runtime maps it. A list of two or more values reads as a `from`
    with that many paths, the way `join` gets its input. A chain that does not compile gives None."""
    if not chain.steps:
        return v
    many = isinstance(v, list) and len(v) >= 2
    paths: Any = [f"v{i}" for i in range(len(v))] if many else "v"
    key = repr((paths, chain.steps))  # cheaper than JSON text in the search loop
    art = _compiled.get(key, False)
    if art is False:
        spec = {"language": 2, "provider": "fit", "endpoint": "chain", "version": 1, "status": "draft", "fields": {"v": {"from": paths, "transform": chain.spec()}}}
        try:
            art = _api.compile_spec(spec, _SCHEMA)
        except ValueError:
            art = None
        _compiled[key] = art
    if art is None:
        return None
    return art.map({f"v{i}": x for i, x in enumerate(v)} if many else {"v": v})["v"]


def apply_rule(rule: dict[str, Any], body: Any, node: dict[str, Any] | None = None, context: dict[str, Any] | None = None) -> Any:
    """What the runtime maps for one field `rule` over a whole provider body, with `node` as the
    field's canonical schema node (an array node takes an array rule). None if it does not compile."""
    key = repr((rule, node))
    art = _compiled.get(key, False)
    if art is False:
        name = "v[]" if node is not None and node.get("type") == "array" else "v"
        spec = {"language": 2, "provider": "fit", "endpoint": "rule", "version": 1, "status": "draft", "fields": {name: rule}}
        try:
            art = _api.compile_spec(spec, {"type": "object", "properties": {"v": {} if node is None else node}})
        except ValueError:
            art = None
        _compiled[key] = art
    if art is None:
        return None
    out = art.map(body) if context is None else art.map(body, context=context)
    return out.get("v")


def _kind(v: Any) -> str:
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, (int, float)):
        return "number"
    return "string" if isinstance(v, str) else "other"


def text(v: Any) -> str:
    """A scalar as the runtime writes it as a string."""
    if isinstance(v, str):
        return v
    s = apply(Chain([("to_string", None)]), v)
    return s if isinstance(s, str) else ""


def _out(name: str, arg: Any) -> str:
    if name == "cast" and arg == "boolean":
        return "boolean"
    if name in ("parse_number", "parse_range") or (isinstance(arg, str) and (name, arg) in _NUMBER_OUT):
        return "number"
    return "string"


def _escape(s: str) -> str:
    return _META.sub(r"\\\1", s)


def _regexes(v: str, w: str) -> list[str]:
    """A pattern around the first place `w` shows in `v`: the separator, word and separator before it
    (or the start), and the text up to the next character after it (or the end)."""
    i = v.find(w)
    if i < 0 or not w or w == v:
        return []
    m = re.search(r"[^A-Za-z0-9]*[A-Za-z0-9]*[^A-Za-z0-9]+\Z", v[:i])
    left = "^" if i == 0 else _escape(m.group(0) if m else v[:i][-8:])
    end = i + len(w)
    if re.fullmatch(r"[A-Za-z0-9_.-]+", w):
        group = "([A-Za-z0-9_.-]+)"
    else:
        group = f"([^{_escape(v[end])}]+)" if end < len(v) else "(.+)$"
    return [left + group]


def _learned(name: str, v: Any, w: str) -> list[Any]:
    """Arguments for `name` taken from value `v` and the wanted text `w`."""
    if not isinstance(v, str) or not w:
        return []
    lv, lw = v.lower(), w.lower()
    if len(lv) != len(v) or len(v) <= len(w):
        return []
    if name == "strip_prefix":
        return [v[: len(v) - len(w)]] if lv.endswith(lw) else []
    if name == "strip_suffix":
        return [v[len(w) :]] if lv.startswith(lw) else []
    if name == "regex":
        return _regexes(v, w)
    out: list[Any] = []
    for sep in dict.fromkeys(_SEP.findall(v)):  # split: every separator run, index from the start and the end
        parts = v.split(sep)
        for i, p in enumerate(parts):
            if p.strip().lower() == lw:
                out += [{"sep": sep, "index": i}, {"sep": sep, "index": i - len(parts)}]
    return out


def _join_seps(v: list[Any], w: str) -> list[Any]:
    """The separator between the first two values that `w` joins."""
    a, b = (text(x) for x in v[:2]) if len(v) >= 2 else ("", "")
    if not a or not b or not w.startswith(a) or len(w) <= len(a) + len(b):
        return []
    end = w.find(b, len(a) + 1)
    return [w[len(a) : end]] if end > 0 else []


def _steps(v: Any, w: str, want: str | None) -> list[tuple[str, Any]]:
    """Single steps for value `v` in ORDER, wanted text `w`; `want` keeps only steps that give that JSON type."""
    out: list[tuple[str, Any]] = []
    for name in ORDER:
        if isinstance(v, list) != (name == "join"):
            continue
        if name == "join":
            args = _join_seps(v, w)
        elif name in ("strip_prefix", "strip_suffix", "split", "regex"):
            args = _learned(name, v, w)
        else:
            args = _FIXED.get(name, [None])
        out += [(name, a) for a in args if want is None or _out(name, a) == want]
    return out


def candidates(target: Any, source: Any, tol: Tolerance = Tolerance()) -> list[Chain]:
    """Every chain of length 0, 1 or 2 that maps `source` to `target` (within `tol`), by length, then ORDER."""
    ok = lambda a, b: match(a, b, tol) != "miss"
    want = _kind(target)
    if want == "other" or source is None:
        return []
    w = text(target)
    found = [Chain()] if ok(source, target) else []
    for st in _steps(source, w, want):
        if ok(apply(Chain([st]), source), target):
            found.append(Chain([st]))
    for first in _steps(source, w, None):
        mid = apply(Chain([first]), source)
        if mid is None or same(mid, source) or same(mid, target):
            continue
        for second in _steps(mid, w, want):
            if ok(apply(Chain([second]), mid), target):
                found.append(Chain([first, second]))
    return found
