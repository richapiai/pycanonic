"""Fourteen invented providers for the entities in world.py. Each one renders the default provider's
body (`render_default`) and its own body (`render`) from the same entity, so the true mapping is
known: on any pair, the right spec maps `render` to exactly `render_default`.

Shapes covered: renamed keys, snake and camel case, nested and flat bodies, string numbers, partial
dates, prefixed ids, URL slugs, enum renames with a consistent table, grouped histories, dropped
sections, `[]` against null against a missing key, bare-array bodies, a page envelope, case-drifting
keys, and one provider whose values genuinely disagree with the default's."""

from __future__ import annotations

import random
from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

E = dict[str, Any]


def iso(d: tuple[Any, ...]) -> str:
    """A date tuple as ISO text with the parts that are known: `2019`, `2019-03` or `2019-03-14`."""
    out = f"{d[0]:04d}"
    for part in d[1:]:
        if part is None:
            break
        out += f"-{part:02d}"
    return out


def ym(d: tuple[int, int] | None) -> str | None:
    return None if d is None else f"{d[0]:04d}-{d[1]:02d}"


def _social_url(handle: str) -> str:
    return f"https://social.example/@{handle}"


def default(e: E) -> E:
    """The default provider's body: the contract the customers see."""
    s = e["social"]
    return {
        "id": e["id"],
        "name": {"first": e["first"], "last": e["last"], "full": f"{e['first']} {e['last']}"},
        "email": e["email"],
        "nickname": e["nickname"],
        "title": e["title"],
        "seniority": e["seniority"],
        "status": e["status"],
        "verified": e["verified"],
        "joined": iso(e["joined"]),
        "birth": iso(e["birth"]),
        "score": e["score"],
        "currency": e["currency"],
        "plan": e["plan"],
        "region": e["region"],
        "location": dict(e["location"]) if e["location"] else None,
        "company": dict(e["company"]) if e["company"] else None,
        "jobs": [{"company": j["company"], "title": j["title"], "start": ym(j["start"]), "end": ym(j["end"])} for j in e["jobs"]],
        "skills": list(e["skills"]),
        "social": {"handle": s["handle"], "url": _social_url(s["handle"]), "followers": s["followers"]} if s else None,
    }


@dataclass
class Provider:
    name: str
    render_default: Callable[[E], Any]
    render: Callable[[E], Any]


def quillon(e: E) -> E:
    """Renamed keys in camel case, and a billing block whose currency and region never change."""
    s, loc, c = e["social"], e["location"], e["company"]
    return {
        "personId": e["id"],
        "givenName": e["first"],
        "familyName": e["last"],
        "displayName": f"{e['first']} {e['last']}",
        "emailAddress": e["email"],
        "nickName": e["nickname"],
        "jobTitle": e["title"],
        "seniorityLevel": e["seniority"],
        "accountStatus": e["status"],
        "isVerified": e["verified"],
        "joinedOn": iso(e["joined"]),
        "birthDate": iso(e["birth"]),
        "rating": e["score"],
        "billing": {"currency": "USD", "cycle": "monthly", "region": "na"},
        "currencyCode": e["currency"],
        "planName": e["plan"],
        "regionCode": e["region"],
        "address": {"cityName": loc["city"], "countryCode": loc["country"]} if loc else None,
        "employer": {"orgName": c["name"], "webDomain": c["domain"], "employeeCount": c["size"], "sector": c["industry"], "yearFounded": c["founded"]} if c else None,
        "workHistory": [{"orgName": j["company"], "roleTitle": j["title"], "startedOn": ym(j["start"]), "endedOn": ym(j["end"])} for j in e["jobs"]],
        "skillTags": list(e["skills"]),
        "socialProfile": {"handle": s["handle"], "profileUrl": _social_url(s["handle"]), "followerCount": s["followers"]} if s else None,
    }


def brask(e: E) -> E:
    """A flat body in snake case, numbers as strings, US dates."""
    s, loc, c = e["social"], e["location"], e["company"]
    y, m, d = e["joined"]
    return {
        "person_id": str(e["id"]),
        "first_name": e["first"],
        "last_name": e["last"],
        "full_name": f"{e['first']} {e['last']}",
        "email": e["email"],
        "nickname": e["nickname"],
        "job_title": e["title"],
        "seniority": e["seniority"],
        "status": e["status"],
        "verified": e["verified"],
        "joined_date": f"{m:02d}/{d:02d}/{y:04d}",
        "birth_date": iso(e["birth"]),
        "score": str(e["score"]),
        "currency": e["currency"],
        "plan": e["plan"],
        "region": e["region"],
        "city": loc["city"] if loc else None,
        "country": loc["country"] if loc else None,
        "company_name": c["name"] if c else None,
        "company_domain": c["domain"] if c else None,
        "company_size": str(c["size"]) if c else None,
        "company_industry": c["industry"] if c else None,
        "company_founded": str(c["founded"]) if c else None,
        "jobs": [{"company_name": j["company"], "job_title": j["title"], "start_month": ym(j["start"]), "end_month": ym(j["end"])} for j in e["jobs"]],
        "skills": list(e["skills"]),
        "social_handle": s["handle"] if s else None,
        "social_url": _social_url(s["handle"]) if s else None,
        "social_followers": str(s["followers"]) if s else None,
    }


def _epoch(d: tuple[int, int, int]) -> int:
    return int(datetime(*d, tzinfo=timezone.utc).timestamp())


def tavio(e: E) -> E:
    """Deeply nested, the join date as epoch seconds, no full name (it is first and last joined)."""
    s, loc, c = e["social"], e["location"], e["company"]
    return {
        "data": {
            "person": {
                "identity": {"names": {"given": e["first"], "family": e["last"], "nick": e["nickname"]}, "contact": {"email": e["email"]}},
                "role": {"title": e["title"], "level": e["seniority"]},
                "account": {"state": e["status"], "verified": e["verified"], "created": _epoch(e["joined"]), "plan": e["plan"], "currency": e["currency"], "region": e["region"]},
                "dob": iso(e["birth"]),
                "score": e["score"],
                "geo": dict(loc) if loc else None,
                "org": {"name": c["name"], "domain": c["domain"], "size": c["size"], "industry": c["industry"], "founded": c["founded"]} if c else None,
                "career": [{"org": j["company"], "title": j["title"], "from": ym(j["start"]), "to": ym(j["end"])} for j in e["jobs"]],
                "skills": list(e["skills"]),
                "social": {"handle": s["handle"], "link": _social_url(s["handle"]), "followers": s["followers"]} if s else None,
            }
        },
        "id": e["id"],
    }


def merrow(e: E) -> E:
    """Prefixed ids and enum codes, URL slugs, a datetime join date, and the location as one string."""
    s, loc, c = e["social"], e["location"], e["company"]
    return {
        "uid": f"usr_{e['id']}",
        "first": e["first"],
        "last": e["last"],
        "name": f"{e['first']} {e['last']}",
        "mail": e["email"],
        "alias": e["nickname"],
        "headline": e["title"],
        "level": e["seniority"],
        "state": e["status"],
        "verified": e["verified"],
        "joined": iso(e["joined"]) + "T09:30:00Z",
        "dob": iso(e["birth"]),
        "score": e["score"],
        "currency": e["currency"],
        "plan": e["plan"],
        "region": e["region"],
        "location": f"{loc['city']}, {loc['country']}" if loc else None,
        "company": {"name": c["name"], "url": f"https://www.{c['domain']}/about", "size": c["size"], "industry": f"ind_{c['industry']}", "founded": c["founded"]} if c else None,
        "positions": [{"org": j["company"], "title": j["title"], "start": ym(j["start"]), "end": ym(j["end"])} for j in e["jobs"]],
        "skills": list(e["skills"]),
        "profile_url": _social_url(s["handle"]) if s else None,
        "followers": s["followers"] if s else None,
    }


SENIORITY_CODES = {"junior": "J1", "mid": "J2", "senior": "S1", "lead": "S2", "principal": "S3", "manager": "M1", "director": "M2", "vp": "E1", "cxo": "E2", "intern": "J0"}
STATUS_CODES = {"active": "A", "inactive": "I", "pending": "P", "suspended": "X"}
INDUSTRY_CODES = {"software": "SW", "freight": "FR", "farming": "AG", "textiles": "TX", "mining": "MN", "brewing": "BR", "shipbuilding": "SB", "perfumery": "PF"}


def zentel(e: E) -> E:
    """Enum renames through fixed tables, and case changes on currency and plan."""
    body = brask(e)
    body.update(
        seniority=SENIORITY_CODES[e["seniority"]],
        status=STATUS_CODES[e["status"]],
        company_industry=INDUSTRY_CODES[e["company"]["industry"]] if e["company"] else None,
        currency=e["currency"].lower(),
        plan=e["plan"].upper(),
        region=e["region"].upper(),
        score=e["score"],
        person_id=e["id"],
        joined_date=iso(e["joined"]),
    )
    return body


def _runs(jobs: list[E]) -> list[list[E]]:
    runs: list[list[E]] = []
    for j in jobs:
        if runs and runs[-1][0]["company_id"] == j["company_id"]:
            runs[-1].append(j)
        else:
            runs.append([j])
    return runs


def obrik_default(e: E) -> E:
    """The default with the job history grouped by employer, in a row (a repeat is a new group)."""
    body = default(e)
    del body["jobs"]
    body["employers"] = [
        {"company": r[0]["company"], "start": ym(r[0]["start"]), "end": ym(r[-1]["end"]), "roles": [{"title": j["title"], "start": ym(j["start"])} for j in r]}
        for r in _runs(e["jobs"])
    ]
    return body


def obrik(e: E) -> E:
    """A flat list of positions with an employer id, which the default groups."""
    body = quillon(e)
    del body["workHistory"]
    body["positions"] = [{"employer_id": j["company_id"], "employer": j["company"], "role": j["title"], "from": ym(j["start"]), "until": ym(j["end"])} for j in e["jobs"]]
    return body


def fennix(e: E) -> E:
    """Drops whole sections: no location, no birth date, no skills, no social profile, no plan."""
    body = quillon(e)
    for k in ("address", "birthDate", "skillTags", "socialProfile", "planName"):
        del body[k]
    return body


def lumari(e: E) -> E:
    """Empty values in other forms: skills null or left out, social `{}`, location and email left out."""
    body = tavio(e)["data"]["person"]
    if not e["skills"]:
        if e["id"] % 2:
            body["skills"] = None
        else:
            del body["skills"]
    if not e["social"]:
        body["social"] = {}
    if not e["location"]:
        del body["geo"]
    if e["email"] is None:
        del body["identity"]["contact"]["email"]
    body["id"] = e["id"]
    return body


def corvane(e: E) -> list[E]:
    """A bare-array body: the record is the only element."""
    return [brask(e)]


def pellis_default(e: E) -> E:
    """A page envelope over the person's jobs."""
    return {"total": len(e["jobs"]), "page": 1, "items": default(e)["jobs"]}


def pellis(e: E) -> E:
    return {"data": quillon(e)["workHistory"], "meta": {"count": len(e["jobs"]), "page": 1, "per_page": 50}}


def _drift(key: str, style: int) -> str:
    return [key, key[0].upper() + key[1:], key.lower()][style]


def drovo(e: E) -> E:
    """Camel case keys that drift: in about one body in four, the top-level keys come capitalized
    or all lowercase."""
    r = random.Random(e["id"])
    style = 0 if r.random() < 0.75 else r.choice([1, 2])
    return {_drift(k, style): v for k, v in quillon(e).items()}


SECTORS = {"software": "technology", "freight": "transport", "farming": "agriculture", "textiles": "manufacturing", "mining": "resources", "brewing": "manufacturing", "shipbuilding": "manufacturing", "perfumery": "consumer"}
SIZES: list[tuple[int, int | None]] = [(1, 9), (10, 99), (100, 999), (1000, None)]  # invented bands, None: no top


def _band(n: int) -> str:
    lo, hi = next(b for b in SIZES if b[0] <= n and (b[1] is None or n <= b[1]))
    return f"{lo}+" if hi is None else f"{lo}-{hi}"


def kestrin(e: E) -> E:
    """Values that genuinely disagree: another industry vocabulary (several industries share a
    sector), head counts as bands, and the score on a 0 to 100 scale."""
    body = quillon(e)
    if e["company"]:
        body["employer"].update(sector=SECTORS[e["company"]["industry"]], employeeCount=_band(e["company"]["size"]))
    body["rating"] = round(e["score"] * 20)
    return body


MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]


def tessaly(e: E) -> E:
    """Strings for numbers and booleans, month names in job dates, follower counts with commas."""
    s, loc, c = e["social"], e["location"], e["company"]
    mon = lambda d: None if d is None else f"{MONTHS[d[1] - 1]} {d[0]}"
    return {
        "profile": {
            "id": str(e["id"]),
            "first": e["first"],
            "last": e["last"],
            "full": f"{e['first']} {e['last']}",
            "email": e["email"],
            "nickname": e["nickname"],
            "title": e["title"],
            "seniority": e["seniority"],
            "status": e["status"],
            "verified": "yes" if e["verified"] else "no",
            "joined": iso(e["joined"]),
            "birth": iso(e["birth"]),
            "score": f"{e['score']:.1f}",
            "currency": e["currency"],
            "plan": e["plan"],
            "region": e["region"],
        },
        "location": {"city": loc["city"], "country": loc["country"]} if loc else None,
        "company": {"name": c["name"], "domain": c["domain"], "size": str(c["size"]), "industry": c["industry"], "founded": str(c["founded"])} if c else None,
        "jobs": [{"company": j["company"], "title": j["title"], "start": mon(j["start"]), "end": mon(j["end"])} for j in e["jobs"]],
        "skills": list(e["skills"]),
        "social": {"handle": s["handle"], "url": _social_url(s["handle"]), "followers": f"{s['followers']:,}"} if s else None,
    }


def ordina_default(e: E) -> E:
    return {"ref": f"item-{e['id']}", "first": e["first"], "status": e["status"]}


def ordina(e: E) -> E:
    """The default's ref is a fixed prefix on the provider's id, and a small integer sits beside it as a decoy."""
    return {"itemId": e["id"], "rank": e["id"] % 4, "givenName": e["first"], "state": e["status"]}


PROVIDERS: list[Provider] = [
    Provider("quillon", default, quillon),
    Provider("brask", default, brask),
    Provider("tavio", default, tavio),
    Provider("merrow", default, merrow),
    Provider("zentel", default, zentel),
    Provider("obrik", obrik_default, obrik),
    Provider("fennix", default, fennix),
    Provider("lumari", default, lumari),
    Provider("corvane", default, corvane),
    Provider("pellis", pellis_default, pellis),
    Provider("drovo", default, drovo),
    Provider("kestrin", default, kestrin),
    Provider("tessaly", default, tessaly),
    Provider("ordina", ordina_default, ordina),
]
