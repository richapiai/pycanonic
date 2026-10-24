"""Invented people and the companies they work for. Every value comes from a seeded generator, so a
seed always gives the same entities.

The world has the traps that make fit guess wrong on fresh data: values that are nearly constant
(`currency`, `plan`, `region`), a value that is nearly always null
(`nickname`, set only on the rare statuses), enums with rare values (`seniority`, `status`, `industry`), partial dates,
optional sections (one that is never empty in 15 pairs), and job histories with repeats and gaps."""

from __future__ import annotations

import random
from typing import Any

FIRST = ["Ilsa", "Bram", "Corin", "Dagny", "Edrik", "Fenna", "Gorm", "Hesper", "Ivo", "Jorun", "Kaspar", "Liesl", "Mirek", "Nadja", "Orrin", "Petra"]
LAST = ["Achterberg", "Brightwell", "Cazimir", "Dunmore", "Elsworth", "Falkner", "Gravel", "Holloway", "Ingram", "Juniper", "Kettle", "Larkspur"]
TITLES = ["Data Engineer", "Account Lead", "Platform Architect", "Support Analyst", "Growth Manager", "Field Technician", "Research Scientist", "Ops Coordinator"]
CITIES = [("Velmora", "VM"), ("Ostrand", "OS"), ("Pellwick", "PW"), ("Quarrow", "QR"), ("Rendlesham", "RD"), ("Sallow Bay", "SB")]
COMPANIES = ["Bluefen Works", "Cobblestone Labs", "Driftmark", "Emberline", "Foxglove Systems", "Gullwing Freight", "Hollowpine", "Ironquill"]
SKILLS = ["sql", "kotlin", "forecasting", "welding", "negotiation", "cartography", "statistics", "rust", "pottery", "logistics"]
# Weighted so that the rare values seldom show in 15 pairs and do show in 60.
SENIORITY = [("junior", 20), ("mid", 25), ("senior", 22), ("lead", 12), ("principal", 6), ("manager", 8), ("director", 4), ("vp", 1.5), ("cxo", 1), ("intern", 0.5)]
STATUS = [("active", 80), ("inactive", 14), ("pending", 5), ("suspended", 1)]
INDUSTRY = [("software", 30), ("freight", 20), ("farming", 15), ("textiles", 12), ("mining", 10), ("brewing", 8), ("shipbuilding", 4), ("perfumery", 1)]
CURRENCY = [("USD", 95), ("EUR", 4), ("GBP", 1)]
PLAN = [("free", 94), ("team", 5), ("enterprise", 1)]
REGION = [("na", 97), ("emea", 2), ("apac", 1)]


def _pick(r: random.Random, table: list[tuple[str, float]]) -> str:
    return r.choices([v for v, _ in table], [w for _, w in table])[0]


def _slug(first: str, last: str, n: int) -> str:
    return f"{first.lower()}-{last.lower()}-{n}"


def _jobs(r: random.Random) -> list[dict[str, Any]]:
    """0 to 5 stints, oldest first. A stint can stay at the last company (a promotion) or return to an
    earlier one after another job (a repeat), and there can be months between stints (a gap). The
    last stint is current (no end) in about half the histories."""
    out: list[dict[str, Any]] = []
    y, m = r.randint(2004, 2012), r.randint(1, 12)
    for k in range(r.choice([0, 1, 2, 2, 3, 3, 4, 5])):
        if out and r.random() < 0.3:
            company = out[-1]["company"]  # promotion: same company, next role
        elif len(out) >= 2 and r.random() < 0.35:
            company = out[-2]["company"]  # repeat after another job
        else:
            company = r.choice([c for c in COMPANIES if not out or c != out[-1]["company"]])
        start = (y, m)
        y, m = y + r.randint(0, 3), r.randint(1, 12)
        out.append({"company": company, "company_id": COMPANIES.index(company) + 101, "title": r.choice(TITLES), "start": start, "end": (y, m)})
        if r.random() < 0.4:  # a gap before the next stint
            y += 1
    if out and r.random() < 0.5:
        out[-1]["end"] = None
    return out


def entities(seed: int, n: int) -> list[dict[str, Any]]:
    """`n` invented people. Dates are tuples: `joined` is (year, month, day), `birth` has None for the
    parts that are not known, job `start` and `end` are (year, month)."""
    r = random.Random(seed)
    out: list[dict[str, Any]] = []
    for i in range(n):
        first, last = r.choice(FIRST), r.choice(LAST)
        company = r.choice(COMPANIES)
        domain = company.lower().replace(" ", "") + ".example"
        birth_year = r.randint(1960, 2001)
        birth_month = r.randint(1, 12) if r.random() < 0.7 else None
        birth_day = r.randint(1, 28) if birth_month and r.random() < 0.7 else None
        city = CITIES[r.randrange(len(CITIES))]
        uid = r.randint(10_000, 99_999)
        status = _pick(r, STATUS)
        out.append(
            {
                "id": uid,
                "first": first,
                "last": last,
                "email": f"{first.lower()}.{last.lower()}@{domain}" if r.random() < 0.8 else None,
                "title": r.choice(TITLES),
                "seniority": _pick(r, SENIORITY),
                "status": status,
                "nickname": f"{first[:3]}{last[:2]}".lower() if status in ("pending", "suspended") else None,  # rare: none in 15 pairs
                "verified": r.random() < 0.6,
                "joined": (r.randint(2015, 2024), r.randint(1, 12), r.randint(1, 28)),
                "birth": (birth_year, birth_month, birth_day),
                "score": round(r.uniform(1, 5), 1),
                "currency": _pick(r, CURRENCY),
                "plan": _pick(r, PLAN),
                "location": {"city": city[0], "country": city[1]} if r.random() < 0.75 else None,
                "company": {
                    "name": company,
                    "domain": domain,
                    "size": r.choice([4, 12, 40, 75, 180, 420, 950, 2400, 7000]) + r.randint(0, 3),
                    "industry": _pick(r, INDUSTRY),
                    "founded": r.randint(1950, 2020),
                },
                "jobs": _jobs(r),
                "skills": r.sample(SKILLS, r.choice([0, 0, 1, 2, 3, 4])),
                "region": _pick(r, REGION),
                "social": {"handle": _slug(first, last, i), "followers": r.randint(0, 50_000)} if r.random() < 0.65 else None,
            }
        )
        if status == "suspended":  # a suspended account loses its company section: none in 15 pairs
            out[-1]["company"] = None
    return out
