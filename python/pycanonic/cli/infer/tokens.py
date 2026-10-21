"""Name tokens for matching provider keys to canonical fields: split camel, snake, kebab and digits,
singularize, then rewrite through the synonym dictionary so "fname" and "first_name" meet."""

from __future__ import annotations

import re

Synonyms = dict[str, list[str]]
"""token -> the canonical tokens it stands for. A deployment passes its own with --synonyms."""

# A small public seed. Keys are lowercase single tokens as split() produces them.
SEED: Synonyms = {
    "fname": ["first", "name"],
    "lname": ["last", "name"],
    "fullname": ["full", "name"],
    "display": ["full"],
    "given": ["first"],
    "forename": ["first"],
    "family": ["last"],
    "surname": ["last", "name"],
    "org": ["company"],
    "organization": ["company"],
    "organisation": ["company"],
    "employer": ["company"],
    "firm": ["company"],
    "biz": ["company"],
    "mobile": ["phone"],
    "cell": ["phone"],
    "tel": ["phone"],
    "telephone": ["phone"],
    "msisdn": ["phone"],
    "mail": ["email"],
    "li": ["linkedin"],
    "lnkd": ["linkedin"],
    "website": ["url"],
    "homepage": ["url"],
    "site": ["url"],
    "web": ["url"],
    "link": ["url"],
    "headline": ["title"],
    "position": ["title"],
    "role": ["title"],
    "job": ["title"],
    "employees": ["headcount"],
    "staff": ["headcount"],
    "size": ["headcount"],
    "hq": ["headquarters"],
    "addr": ["address"],
    "dob": ["birth", "date"],
    "desc": ["description"],
    "num": ["number"],
    "no": ["number"],
    "cnt": ["count"],
    "qty": ["quantity"],
    "amt": ["amount"],
    "dt": ["date"],
    "ts": ["date"],
    "updated": ["update"],
    "created": ["create"],
    "country_code": ["country"],
    "zip": ["postal", "code"],
    "postcode": ["postal", "code"],
    "history": ["experience"],
    "employment": ["experience"],
    "school": ["education"],
    "expertise": ["skill"],
    "competency": ["skill"],
    "tag": ["skill"],
    "raised": ["funding"],
    "sales": ["revenue"],
    "sector": ["industry"],
    "vertical": ["industry"],
    "inception": ["founded"],
    "level": ["seniority"],
    "from": ["start"],
    "started": ["start"],
    "began": ["start"],
    "ended": ["end"],
    "finished": ["end"],
    "since": ["start"],
    "begin": ["start"],
    "to": ["end"],
    "until": ["end"],
    "till": ["end"],
}

# ASCII classes, as in the TypeScript (no `u` flag there): a non-ASCII letter starts no token.
WORD = re.compile(r"[A-Z]+(?![a-z])|[A-Z]?[a-z]+|[0-9]+")


def split(name: str) -> list[str]:
    """"workEmailAddress" -> ["work", "email", "address"]; "li_url" -> ["li", "url"]."""
    return [w.lower() for w in WORD.findall(name)]


def singular(t: str) -> str:
    """Plural to singular for the common English endings; names are short and regular enough."""
    if len(t) > 4 and t.endswith("ies"):
        return t[:-3] + "y"
    if len(t) > 4 and re.search(r"(ss|x|ch|sh)es\Z", t):
        return t[:-2]
    if len(t) > 3 and t.endswith("s") and not t.endswith("ss") and not t.endswith("us"):
        return t[:-1]
    return t


def synonyms_of(synonyms: Synonyms, token: str) -> list[str] | None:
    """A token's entry. A dict has no prototype, so "constructor" is just a word here too."""
    return synonyms.get(token)


def tokens(name: str, synonyms: Synonyms) -> list[str]:
    """Tokens of a name after synonyms, deduplicated, in order."""
    out: list[str] = []
    raw = split(name)
    # A whole-name entry ("fullname", "country_code") wins over its parts.
    whole = synonyms_of(synonyms, "_".join(raw))
    if whole is None:
        whole = synonyms_of(synonyms, "".join(raw))
    for t in whole if whole is not None and len(raw) > 1 else raw:
        s = singular(t)
        found = synonyms_of(synonyms, t)
        if found is None:
            found = synonyms_of(synonyms, s)
        for x in found if found is not None else [s]:
            if x not in out:
                out.append(x)
    return out


WRAPPERS = {"data", "result", "response", "payload", "body", "attribute", "field", "record", "detail", "info"}
"""Keys that wrap a payload rather than name anything; they don't count in path matching."""


def subsequence(short: str, long: str) -> bool:
    i = 0
    for c in long:
        if i < len(short) and c == short[i]:
            i += 1
    return i == len(short)


def abbreviates(s: str, c: str) -> bool:
    """Whether source token s abbreviates canonical token c: same first letter, letters in order ("nm" for
    name, "ctry" for country). Two-letter abbreviations only count for short words, so "id" isn't
    "industry"."""
    if len(s) < 2 or len(s) >= len(c) or s[0] != c[0]:
        return False
    return (len(s) >= 3 or len(c) <= 5) and subsequence(s, c)


def initials(s: str, canon: list[str], i: int) -> int:
    """Length of the run of canonical tokens from i whose initials spell s ("fn" for first name), or 0."""
    if len(s) < 2 or len(s) > 3 or i + len(s) > len(canon):
        return 0
    return len(s) if all(t[:1] == s[k] for k, t in enumerate(canon[i : i + len(s)])) else 0


def similarity(canon: list[str], source: list[str]) -> float:
    """Dice similarity of canonical tokens against source tokens, 0..1. A source token matches a
    canonical token it equals or abbreviates, or a run of canonical tokens it gives the initials of."""
    if not canon or not source:
        return 0
    hit_canon: set[int] = set()
    hit_source = 0
    for s in source:
        i = next((k for k, c in enumerate(canon) if k not in hit_canon and (c == s or abbreviates(s, c))), -1)
        if i != -1:
            hit_canon.add(i)
            hit_source += 1
            continue
        start = next((k for k in range(len(canon)) if initials(s, canon, k) > 0 and k not in hit_canon), -1)
        if start == -1:
            continue
        hit_canon.update(range(start, start + len(s)))
        hit_source += 1
    return (len(hit_canon) + hit_source) / (len(canon) + len(source))
