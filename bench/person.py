"""Port of canonic's bench/person.js: the person.enrich mapping, the same mapping written by hand,
and the synthetic payloads. `raw(i)` is byte-identical to `JSON.stringify(payload(i))` there.
The hand-written mapper calls the same transforms, so the comparison measures mapping overhead."""

import json
import math
from decimal import Decimal

from pycanonic import default_transforms as T
from pycanonic import validators as V

_str = {"type": "string"}
canonical = {
    "type": "object",
    "properties": {
        "first_name": _str, "last_name": _str, "full_name": _str,
        "email": {"type": "string", "semantic": "email", "routing": True},
        "phone": {"type": "string", "semantic": "phone", "routing": True},
        "title": _str, "seniority": _str, "linkedin_url": _str,
        "location": {"type": "object", "properties": {"city": _str, "country": _str}},
        "company": {
            "type": "object",
            "properties": {
                "name": _str, "domain": {"type": "string", "semantic": "domain", "routing": True},
                "headcount": {"type": "integer"}, "industry": _str, "founded": _str,
            },
        },
        "experiences": {"type": "array", "items": {"type": "object", "properties": {"title": _str, "company": _str, "start": _str, "end": _str}}},
        "skills": {"type": "array", "items": _str},
        "updated_at": _str,
    },
}

spec = {
    "provider": "bench", "endpoint": "person.enrich", "version": 1, "status": "promoted",
    "fields": {
        "first_name": {"from": "profile.fullName", "transform": {"split_name": "first"}},
        "last_name": {"from": "profile.fullName", "transform": {"split_name": "last"}},
        "full_name": "profile.fullName",
        "email": {"coalesce": ["contact.workEmail", "contact.emails[0].address"], "transform": ["trim", "lower"]},
        "phone": {"from": "contact.mobile", "transform": "normalize_phone"},
        "title": {"from": "profile.headline", "transform": "trim"},
        "seniority": "profile.level",
        "linkedin_url": {"from": "social.li", "transform": "normalize_linkedin"},
        "location.city": "geo.city",
        "location.country": "geo.countryCode",
        "company.name": "org.name",
        "company.domain": {"from": "org.website", "transform": "to_domain"},
        "company.headcount": {"from": "org.size", "transform": {"cast": "integer"}},
        "company.industry": {"from": "org.industry", "transform": "lower"},
        "company.founded": {"from": "org.foundedOn", "transform": "parse_date"},
        "experiences[]": {
            "from": "workHistory",
            "item": {"title": "role", "company": "orgName", "start": {"from": "from", "transform": "parse_date"}, "end": {"from": "to", "transform": "parse_date"}},
        },
        "skills[]": {"from": "tags", "pluck": "label", "transform": "lower"},
        "updated_at": {"from": "_meta.updated", "transform": "parse_date"},
    },
}

# Anchors need single-path fields, so this variant reads email from workEmail alone.
anchored_spec = {
    **spec,
    "anchors": {"email": '"workEmail":', "phone": '"mobile":', "company.domain": '"website":'},
    "fields": {**spec["fields"], "email": {"from": "contact.workEmail", "transform": ["trim", "lower"]}},
}

_trim, _lower, _split, _phone, _li = T["trim"], T["lower"], T["split_name"], T["normalize_phone"], T["normalize_linkedin"]
_domain, _cast, _date = T["to_domain"], T["cast"], T["parse_date"]
_v_email, _v_phone, _v_domain = V["email"], V["phone"], V["domain"]


def _email(contact):
    email = contact.get("workEmail")
    if email is None:
        emails = contact.get("emails")
        email = emails[0].get("address") if emails else None
    if email is not None:
        email = _trim(email)
    if email is not None:
        email = _lower(email)
    if email is not None:
        email = _v_email(email)
    return email


def _routing(contact, org):
    email = _email(contact)
    phone = contact.get("mobile")
    if phone is not None:
        phone = _phone(phone)
    if phone is not None:
        phone = _v_phone(phone)
    website = org.get("website")
    domain = None if website is None else _domain(website)
    if domain is not None:
        domain = _v_domain(domain)
    return email, phone, domain


# Written the way glue code is: inline, one statement per field, transforms called directly.
# canonic's null semantics are kept (a transform only runs on a present value) so outputs match.
def hand_written(p):
    """What a developer would write for this mapping without canonic."""
    profile = p.get("profile") or {}
    contact = p.get("contact") or {}
    org = p.get("org") or {}
    geo = p.get("geo") or {}
    name = profile.get("fullName")
    headline = profile.get("headline")
    li = (p.get("social") or {}).get("li")
    size = org.get("size")
    industry = org.get("industry")
    founded = org.get("foundedOn")
    updated = (p.get("_meta") or {}).get("updated")
    jobs = p.get("workHistory")
    tags = p.get("tags")
    email, phone, domain = _routing(contact, org)
    return {
        "first_name": None if name is None else _split(name, "first"),
        "last_name": None if name is None else _split(name, "last"),
        "full_name": name,
        "email": email,
        "phone": phone,
        "title": None if headline is None else _trim(headline),
        "seniority": profile.get("level"),
        "linkedin_url": None if li is None else _li(li),
        "location": {"city": geo.get("city"), "country": geo.get("countryCode")},
        "company": {
            "name": org.get("name"),
            "domain": domain,
            "headcount": None if size is None else _cast(size, "integer"),
            "industry": None if industry is None else _lower(industry),
            "founded": None if founded is None else _date(founded),
        },
        "experiences": [
            {
                "title": e.get("role"),
                "company": e.get("orgName"),
                "start": None if e.get("from") is None else _date(e["from"]),
                "end": None if e.get("to") is None else _date(e["to"]),
            }
            for e in jobs
        ]
        if isinstance(jobs, list)
        else None,
        "skills": [None if x.get("label") is None else _lower(x["label"]) for x in tags] if isinstance(tags, list) else None,
        "updated_at": None if updated is None else _date(updated),
    }


def hand_routing(p):
    """The routing fields, read by hand from a parsed payload."""
    email, phone, domain = _routing(p.get("contact") or {}, p.get("org") or {})
    return {"email": email, "phone": phone, "company.domain": domain}


# --- payloads -------------------------------------------------------------------------------------

def rng(seed):
    """person.js `rng`. JavaScript does this in doubles, so the product rounds past 2**53; so must we."""
    s = float(seed)

    def r():
        nonlocal s
        s = math.fmod(s * 1103515245.0 + 12345.0, 2147483648.0)
        return s / 2147483648.0

    return r


WORDS = ["alpha", "beta", "gamma", "delta", "omega", "sigma", "kappa", "lambda", "theta", "zeta"]


def pick(r, xs):
    return xs[math.floor(r() * len(xs))]


def word(r):
    return pick(r, WORDS)


def noise(r, n):
    """Keys a provider returns that the mapping ignores, to reach a realistic ~8 KB body."""
    o = {}
    for i in range(n):
        key = f"{word(r)}_{i}"  # the key before the value, as in the JavaScript
        o[key] = f"{word(r)} {word(r)} {word(r)}" if r() < 0.5 else math.floor(r() * 1e6)
    return o


def payload(i):
    r = rng(i + 1)
    return {
        "id": f"p_{i}",
        "profile": {"fullName": f"{word(r)} {word(r)}", "headline": f"  Head of {word(r)}  ", "level": pick(r, ["vp", "director", "ic"])},
        "enrichment": noise(r, 120),
        "contact": {
            "workEmail": f" {word(r)}.{i}@Example.com ",
            "emails": [{"address": f"{word(r)}@example.org", "type": "personal"}],
            "mobile": f"+1 (415) 555-{str(1000 + i)[-4:]}",
        },
        "social": {"li": f"https://www.linkedin.com/in/{word(r)}-{i}/", "x": f"@{word(r)}"},
        "geo": {"city": word(r), "countryCode": "US", "lat": r() * 90, "lng": r() * 180},
        "signals": noise(r, 60),
        "org": {"name": f"{word(r)} Inc", "website": f"https://www.{word(r)}{i}.com/about", "size": str(10 + i), "industry": "Software", "foundedOn": "2014-06-01"},
        "workHistory": [{"role": word(r), "orgName": f"{word(r)} Co", "from": f"201{k}-01", "to": f"201{k + 1}-06-30", "summary": f"{word(r)} " * 8} for k in range(5)],
        "tags": [{"label": word(r).upper(), "score": r()} for _ in range(10)],
        "_meta": {"updated": 1727600000, "source": "bench"},
    }


def js_number(x):
    """ECMAScript Number::toString for a finite double: the shortest round-trip digits, laid out
    the JavaScript way (no `.0`, plain notation from 1e-7 up to 1e21)."""
    if isinstance(x, int) or x.is_integer() and abs(x) < 1e21:
        return str(int(x))
    sign, digits, exp = Decimal(repr(x)).as_tuple()
    ds = "".join(map(str, digits)).rstrip("0")
    exp += len(digits) - len(ds)
    k, n = len(ds), len(ds) + exp
    if k <= n <= 21:
        s = ds + "0" * (n - k)
    elif 0 < n <= 21:
        s = f"{ds[:n]}.{ds[n:]}"
    elif -6 < n <= 0:
        s = "0." + "0" * -n + ds
    else:
        s = ds[0] + (f".{ds[1:]}" if k > 1 else "") + f"e{'+' if n > 0 else '-'}{abs(n - 1)}"
    return "-" + s if sign else s


def js_stringify(v):
    """`JSON.stringify` for the values these generators build."""
    if isinstance(v, dict):
        return "{" + ",".join(f"{json.dumps(k, ensure_ascii=False)}:{js_stringify(x)}" for k, x in v.items()) + "}"
    if isinstance(v, list):
        return "[" + ",".join(map(js_stringify, v)) + "]"
    if isinstance(v, (int, float)) and not isinstance(v, bool):
        return js_number(v)
    return json.dumps(v, ensure_ascii=False)


def raw(i):
    return js_stringify(payload(i))


if __name__ == "__main__":
    for x, want in [(1.0, "1"), (0.5, "0.5"), (1e-7, "1e-7"), (5e-05, "0.00005"), (1e21, "1e+21"), (123.456, "123.456"), (-2.5e-8, "-2.5e-8")]:
        assert js_number(x) == want, (x, js_number(x), want)
