"""Grow the synonym dictionary from specs a person has approved: every reviewed mapping says which
provider key means which canonical name, and the next provider often abbreviates the same way."""

from __future__ import annotations

import json
import re
from typing import Any

from ..io import is_obj, js_keys
from .score import walk_canonical
from .tokens import SEED, Synonyms, split, synonyms_of, tokens

# The key segments of a spec path. canonic's own path parser is not part of its public API.
SEGMENT = re.compile(r'([A-Za-z_$][A-Za-z0-9_$-]*)|\["((?:[^"\\]|\\.)*)"\]|\[[0-9]+\]')


def keys_of(rule: Any) -> list[str]:
    """The keys along a simple rule's single from path, or none for anything else."""
    frm = rule if isinstance(rule, str) else rule.get("from") if is_obj(rule) else None
    if not isinstance(frm, str):
        return []
    out: list[str] = []
    for m in SEGMENT.finditer(frm):
        if m[1]:
            out.append(m[1])
        elif m[2] is not None:
            out.append(json.loads(f'"{m[2]}"'))
    return out


def prefixes(keys: list[str]) -> set[str]:
    """First tokens shared by two or more keys of one spec ("p" in p_fn, p_ln) are prefixes, not words."""
    firsts = [split(k)[0] for k in keys if split(k)]
    return {t for i, t in enumerate(firsts) if firsts.index(t) != i}


def learn_synonyms(specs: list[dict[str, Any]], canonical: Any, base: Synonyms = SEED) -> Synonyms:
    """Learns token -> canonical tokens from approved specs' fields. Only a key with exactly one unexplained
    token is used, and only when neither the key nor its parent already shares a word with the
    canonical name ("emails.work" says email through its parent, so "work" is not learned as email)."""
    leaves = walk_canonical(canonical)[0]
    by_path = {l.path: l for l in leaves}
    vocabulary = {t for l in leaves for t in [*split(l.parent), *(w for n in l.names for w in split(n))]}
    out: Synonyms = {}
    for fields in specs:
        pairs = [(by_path[f], keys_of(fields[f])) for f in js_keys(fields) if f in by_path]
        pairs = [(leaf, keys) for leaf, keys in pairs if keys]
        noise = prefixes([keys[-1] for _, keys in pairs])
        for leaf, keys in pairs:
            want = tokens(leaf.names[0], base)
            context = [t for k in keys[-2:] for t in tokens(k, base)]
            if any(t in want or t in tokens(leaf.parent, base) for t in context):
                continue
            unknown = [t for t in split(keys[-1]) if t not in noise and t not in vocabulary and synonyms_of(base, t) is None]
            if len(unknown) == 1 and unknown[0] not in out:
                out[unknown[0]] = want
    return out
