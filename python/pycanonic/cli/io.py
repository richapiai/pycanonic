"""What the commands share: usage errors, reading JSON files, the `--transforms` table, and the few
JavaScript behaviors (key order, `JSON.stringify`, `trim`) the ported commands need to match byte for byte."""

from __future__ import annotations

import argparse
import json
import math
import os
import re
from decimal import Decimal
from typing import Any, NoReturn, TypeGuard

from .._api import Transform

Transforms = dict[str, Transform]
"""Deployment-private transforms loaded by --transforms."""


class UsageError(Exception):
    """The command could not run as asked; the message (usually the usage text) goes to stderr."""


class _Parser(argparse.ArgumentParser):
    def error(self, message: str) -> NoReturn:
        # One line, `canonic: <message>`, as the TypeScript CLI prints parseArgs errors.
        raise ValueError(message)


def parser(usage: str) -> argparse.ArgumentParser:
    """An argparse parser that raises `ValueError` (one line, exit 2) instead of exiting."""
    return _Parser(usage=usage, add_help=False, allow_abbrev=False)


def read_text(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        return f.read()


def parse_json(text: str, path: str) -> Any:
    try:
        return json.loads(text)
    except json.JSONDecodeError as e:
        raise ValueError(f"{path}: {e}") from None


def read_json(path: str) -> Any:
    return parse_json(read_text(path), path)


def json_files(args: list[str]) -> list[str]:
    """The files named on the command line; a directory stands for the .json files in it, sorted."""
    return [
        p
        for a in args
        for p in (
            [os.path.join(a, f) for f in sorted(os.listdir(a)) if f.endswith(".json")]
            if os.path.isdir(a)
            else [a]
        )
    ]


def pad(s: str, n: int) -> str:
    return f"{s} " if len(s) >= n else s.ljust(n)


def is_obj(v: Any) -> TypeGuard[dict[str, Any]]:
    return isinstance(v, dict)


def get(o: Any, path: str) -> Any:
    """The value at a dotted canonical path ("company.domain", "skills[]"), or None."""
    v = o
    for k in (path[:-2] if path.endswith("[]") else path).split("."):
        v = v.get(k) if isinstance(v, dict) else None
    return v


# JavaScript's whitespace for trim() and \s, which Python's str.strip() and re's \s differ from.
JS_WS = "\t\n\v\f\r \u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"

_TRIM = re.compile(f"\\A[{JS_WS}]+|[{JS_WS}]+\\Z")


def js_trim(s: str) -> str:
    return _TRIM.sub("", s)


def _is_index(k: str) -> bool:
    return k.isascii() and k.isdigit() and (k == "0" or k[0] != "0") and int(k) < 2**32 - 1


def js_keys(d: dict[str, Any]) -> list[str]:
    """`Object.keys` order: array-index keys ("0", "10") first in numeric order, then the rest as inserted."""
    keys = list(d)
    return sorted((k for k in keys if _is_index(k)), key=int) + [k for k in keys if not _is_index(k)]


def js_number(f: float) -> str:
    """`String(n)` for a JavaScript number."""
    if isinstance(f, int):
        return str(f)
    if not math.isfinite(f):
        return "NaN" if f != f else ("Infinity" if f > 0 else "-Infinity")
    if f.is_integer() and abs(f) < 1e21:
        return str(int(f))
    r = repr(f)
    if "e" not in r:
        return r
    mantissa, exp = r.split("e")
    e = int(exp)
    if e < -6 or e >= 21:
        return f"{mantissa}e{'+' if e > 0 else '-'}{abs(e)}"
    return format(Decimal(r), "f")


def js_json(v: Any, indent: int = 0, at: str = "") -> str:
    """`JSON.stringify(v)`, or `JSON.stringify(v, null, indent)`: JavaScript key order and number format."""
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return js_number(v) if math.isfinite(v) else "null"
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    inner = at + " " * indent
    sep, nl = (",\n" + inner, "\n" + inner) if indent else (",", "")
    end = "\n" + at if indent else ""
    if isinstance(v, list):
        return f"[{nl}{sep.join(js_json(x, indent, inner) for x in v)}{end}]" if v else "[]"
    if isinstance(v, dict):
        colon = ": " if indent else ":"
        items = (f"{json.dumps(k, ensure_ascii=False)}{colon}{js_json(v[k], indent, inner)}" for k in js_keys(v))
        return f"{{{nl}{sep.join(items)}{end}}}" if v else "{}"
    raise TypeError(f"not JSON: {type(v).__name__}")
