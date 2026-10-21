"""Port of canonic's bench/routing.js: ~10.7 KB responses with 5 routing fields spread from 10%
to 90% of the document. `raw(i)` is byte-identical to `JSON.stringify(payload(i))` there."""

import math

from person import WORDS, js_stringify, rng
from pycanonic import validators as V


def _route(semantic):
    return {"type": "string", "semantic": semantic, "routing": True}


canonical = {
    "type": "object",
    "properties": {
        "email": _route("email"),
        "phone": _route("phone"),
        "linkedin_url": _route("linkedin_url"),
        "website": _route("url"),
        "domain": _route("domain"),
    },
}

fields = {
    "email": "contact.workEmail",
    "phone": "contact.mobile",
    "linkedin_url": "social.li",
    "website": "org.homepage",
    "domain": "org.domain",
}
spec = {"provider": "bench", "endpoint": "person.route", "version": 1, "status": "promoted", "fields": fields}
anchored_spec = {
    **spec,
    "anchors": {"email": '"workEmail":', "phone": '"mobile":', "linkedin_url": '"li":', "website": '"homepage":', "domain": '"domain":'},
}


def filler(r, size, tag):
    """~size bytes of keys the mapping ignores."""
    o = {}
    i = 0
    while len(js_stringify(o)) < size:
        key = f"{tag}_{WORDS[i % 10]}_{i}"
        o[key] = f"{WORDS[math.floor(r() * 10)]} {WORDS[math.floor(r() * 10)]}" if r() < 0.5 else math.floor(r() * 1e6)
        i += 1
    return o


def payload(i):
    r = rng(i + 7)
    return {
        "id": f"p_{i}",
        "a": filler(r, 1000, "a"),
        "contact": {"workEmail": f"user{i}@example.com", "b": filler(r, 2100, "b"), "mobile": f"+1415555{str(1000 + i)[-4:]}"},
        "c": filler(r, 2200, "c"),
        "social": {"li": f"https://www.linkedin.com/in/user-{i}"},
        "d": filler(r, 2200, "d"),
        "org": {"homepage": f"https://example{i}.com/", "e": filler(r, 2200, "e"), "domain": f"example{i}.com"},
        "f": filler(r, 950, "f"),
    }


def raw(i):
    return js_stringify(payload(i))


_email, _phone, _li, _url, _domain = V["email"], V["phone"], V["linkedin_url"], V["url"], V["domain"]


def hand_read(p):
    """The same five fields, read by hand from a parsed payload."""
    c = p.get("contact") or {}
    o = p.get("org") or {}
    s = p.get("social") or {}
    return {
        "email": _email(c.get("workEmail")),
        "phone": _phone(c.get("mobile")),
        "linkedin_url": _li(s.get("li")),
        "website": _url(o.get("homepage")),
        "domain": _domain(o.get("domain")),
    }
