"""Six synthetic providers for the same people, each with its own naming, nesting and value formats, and
the spec a person would write for it. Ported from the TypeScript CLI's test/eval/providers.ts: `person(n)`
gives the same values as the TypeScript one, since the thresholds were tuned on those."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Callable

FIRST = ["Ada", "Alan", "Grace", "Edsger", "Barbara", "Donald", "Frances", "John", "Radia", "Ken", "Margaret", "Tim"]
LAST = ["Lovelace", "Turing", "Hopper", "Dijkstra", "Liskov", "Knuth", "Allen", "Backus", "Perlman", "Thompson", "Hamilton", "Berners-Lee"]
TITLES = ["VP Engineering", "Head of Data", "Staff Engineer", "CTO", "Director of Sales", "Product Manager"]
LEVELS = ["vp", "head", "senior", "cxo", "director", "manager"]
CITIES = ["London", "Austin", "Berlin", "Toronto", "Lisbon", "Denver"]
INDUSTRIES = ["Software", "Fintech", "Healthcare", "Logistics", "Retail", "Energy"]
SKILLS = ["sql", "go", "rust", "sales", "python", "design", "ml", "kotlin"]
COS = ["Acme", "Globex", "Initech", "Umbrella", "Hooli", "Stark"]


@dataclass
class Job:
    title: str
    company: str
    start: str
    end: str


@dataclass
class Person:
    first: str
    last: str
    email: str
    personal: str
    phone: str  # +1 415 555 01xx
    li: str  # slug
    title: str
    level: str
    city: str
    country: str
    company: str
    domain: str
    headcount: int
    industry: str
    founded: str  # YYYY-MM-DD
    jobs: list[Job]
    skills: list[str]
    updated: int  # epoch seconds


def person(i: int) -> Person:
    f = FIRST[i % 12]
    l = LAST[(i * 5) % 12]
    co = COS[(i * 7) % 6]
    domain = f"{co.lower()}{i}.com"
    return Person(
        first=f,
        last=l,
        email=f"{f.lower()}.{l.lower()}@{domain}",
        personal=f"{f.lower()}{i}@gmail.com",
        phone=f"+1 415 555 {100 + (i % 900):04d}",
        li=f"{f.lower()}-{l.lower()}-{i}",
        title=TITLES[i % 6],
        level=LEVELS[i % 6],
        city=CITIES[(i * 3) % 6],
        country=["GB", "US", "DE", "CA", "PT", "US"][(i * 3) % 6],
        company=f"{co} Inc",
        domain=domain,
        headcount=10 + ((i * 37) % 5000),
        industry=INDUSTRIES[(i * 11) % 6],
        founded=f"{1990 + (i % 30)}-0{1 + (i % 9)}-1{i % 9}",
        jobs=[Job(TITLES[(i + k) % 6], f"{COS[(i + k + 1) % 6]} Ltd", f"20{10 + k}-0{1 + k}", f"20{12 + k}-1{k}") for k in range(1 + (i % 3))],
        skills=[SKILLS[i % 8], SKILLS[(i + 3) % 8]],
        updated=1_700_000_000 + i * 86_400,
    )


@dataclass
class Provider:
    name: str
    payload: Callable[[Person, int], Any]
    truth: dict[str, Any]


def noise(i: int, n: int, tag: str) -> dict[str, int]:
    return {f"{tag}{k}": (i * 31 + k) % 997 for k in range(n)}


def iso(seconds: int) -> str:
    """`new Date(seconds * 1000).toISOString()`"""
    return datetime.fromtimestamp(seconds, timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z")


PROVIDERS: list[Provider] = [
    Provider(
        name="nested camelCase",
        payload=lambda p, i: {
            "id": f"n_{i}",
            "profile": {"fullName": f"{p.first} {p.last}", "headline": f" {p.title} ", "level": p.level},
            "contact": {"workEmail": p.email if i % 4 else None, "emails": [{"address": p.email, "type": "work"}], "mobile": p.phone},
            "social": {"li": f"https://www.linkedin.com/in/{p.li}/"},
            "geo": {"city": p.city, "countryCode": p.country},
            "org": {"name": p.company, "website": f"https://www.{p.domain}/about", "size": str(p.headcount), "industry": p.industry, "foundedOn": p.founded},
            "workHistory": [{"role": j.title, "orgName": j.company, "from": j.start, "to": j.end} for j in p.jobs],
            "tags": [{"label": s.upper()} for s in p.skills],
            "_meta": {"updated": p.updated},
            "stats": noise(i, 10, "s"),
        },
        truth={
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
            "company.industry": "org.industry",
            "company.founded": {"from": "org.foundedOn", "transform": "parse_date"},
            "experiences[]": {"from": "workHistory", "item": {"title": "role", "company": "orgName", "start": {"from": "from", "transform": "parse_date"}, "end": {"from": "to", "transform": "parse_date"}}},
            "skills[]": {"from": "tags", "pluck": "label"},
            "updated_at": {"from": "_meta.updated", "transform": "parse_date"},
        },
    ),
    Provider(
        name="flat snake_case",
        payload=lambda p, i: {
            "person_id": i,
            "fname": p.first,
            "lname": p.last,
            "e_mail": p.email,
            "tel": p.phone.replace(" ", "-"),
            "li_url": f"linkedin.com/in/{p.li}",
            "job_title": p.title,
            "org_name": p.company,
            "org_url": f"http://{p.domain}",
            "num_employees": p.headcount,
            "sector": p.industry,
            "city": p.city,
            "country_code": p.country,
            "jobs": [{"job_title": j.title, "employer": j.company, "start_date": j.start, "end_date": j.end} for j in p.jobs],
            "skill_list": [{"name": s} for s in p.skills],
            "last_updated": p.updated * 1000,
            **noise(i, 8, "attr_"),
        },
        truth={
            "first_name": "fname",
            "last_name": "lname",
            "full_name": {"from": ["fname", "lname"], "transform": "join"},
            "email": {"from": "e_mail", "transform": ["trim", "lower"]},
            "phone": {"from": "tel", "transform": "normalize_phone"},
            "title": "job_title",
            "linkedin_url": {"from": "li_url", "transform": "normalize_linkedin"},
            "location.city": "city",
            "location.country": "country_code",
            "company.name": "org_name",
            "company.domain": {"from": "org_url", "transform": "to_domain"},
            "company.headcount": "num_employees",
            "company.industry": "sector",
            "experiences[]": {"from": "jobs", "item": {"title": "job_title", "company": "employer", "start": {"from": "start_date", "transform": "parse_date"}, "end": {"from": "end_date", "transform": "parse_date"}}},
            "skills[]": {"from": "skill_list", "pluck": "name"},
            "updated_at": {"from": "last_updated", "transform": "parse_date"},
        },
    ),
    Provider(
        name="kebab-case",
        payload=lambda p, i: {
            "full-name": f"{p.first} {p.last}",
            "work-email": p.email,
            "phone-number": p.phone,
            "linkedin": f"https://linkedin.com/in/{p.li}",
            "position": p.title,
            "company": {"company-name": p.company, "domain": p.domain, "employee-count": p.headcount, "industry": p.industry, "year-founded": p.founded[:4]},
            "location": {"city": p.city, "country": p.country},
            "positions": [{"title": j.title, "company-name": j.company, "start-date": j.start, "end-date": j.end} for j in p.jobs],
            "expertise": [{"label": s} for s in p.skills],
            "updated-at": iso(p.updated),
            "meta": noise(i, 6, "m"),
        },
        truth={
            "first_name": {"from": '["full-name"]', "transform": {"split_name": "first"}},
            "last_name": {"from": '["full-name"]', "transform": {"split_name": "last"}},
            "full_name": '["full-name"]',
            "email": {"from": '["work-email"]', "transform": ["trim", "lower"]},
            "phone": {"from": '["phone-number"]', "transform": "normalize_phone"},
            "title": "position",
            "linkedin_url": {"from": "linkedin", "transform": "normalize_linkedin"},
            "location.city": "location.city",
            "location.country": "location.country",
            "company.name": 'company["company-name"]',
            "company.domain": {"from": "company.domain", "transform": "to_domain"},
            "company.headcount": 'company["employee-count"]',
            "company.industry": "company.industry",
            "company.founded": {"from": 'company["year-founded"]', "transform": "parse_date"},
            "experiences[]": {"from": "positions", "item": {"title": "title", "company": '["company-name"]', "start": {"from": '["start-date"]', "transform": "parse_date"}, "end": {"from": '["end-date"]', "transform": "parse_date"}}},
            "skills[]": {"from": "expertise", "pluck": "label"},
            "updated_at": {"from": '["updated-at"]', "transform": "parse_date"},
        },
    ),
    Provider(
        name="deep nesting",
        payload=lambda p, i: {
            "data": {
                "person": {
                    "names": {"given": p.first, "family": p.last, "display": f"{p.first} {p.last}"},
                    "contact": {"emails": {"work": p.email, "personal": p.personal}, "phones": {"mobile": p.phone}},
                    "profiles": {"linkedin": {"url": f"https://www.linkedin.com/in/{p.li}"}},
                    "employment": {"title": p.title, "seniority": p.level},
                    "address": {"city": p.city, "country": p.country},
                },
                "company": {"name": p.company, "website": f"https://{p.domain}", "headcount": {"value": p.headcount}, "industry": {"name": p.industry}, "founded": p.founded},
                "history": [{"role": j.title, "employer": {"name": j.company}, "dates": {"start": j.start, "end": j.end}} for j in p.jobs],
            },
            "meta": {"updated": p.updated},
        },
        truth={
            "first_name": "data.person.names.given",
            "last_name": "data.person.names.family",
            "full_name": "data.person.names.display",
            "email": {"from": "data.person.contact.emails.work", "transform": ["trim", "lower"]},
            "phone": {"from": "data.person.contact.phones.mobile", "transform": "normalize_phone"},
            "title": "data.person.employment.title",
            "seniority": "data.person.employment.seniority",
            "linkedin_url": {"from": "data.person.profiles.linkedin.url", "transform": "normalize_linkedin"},
            "location.city": "data.person.address.city",
            "location.country": "data.person.address.country",
            "company.name": "data.company.name",
            "company.domain": {"from": "data.company.website", "transform": "to_domain"},
            "company.headcount": "data.company.headcount.value",
            "company.industry": "data.company.industry.name",
            "company.founded": {"from": "data.company.founded", "transform": "parse_date"},
            "experiences[]": {"from": "data.history", "item": {"title": "role", "company": "employer.name", "start": {"from": "dates.start", "transform": "parse_date"}, "end": {"from": "dates.end", "transform": "parse_date"}}},
            "updated_at": {"from": "meta.updated", "transform": "parse_date"},
        },
    ),
    Provider(
        name="PascalCase",
        payload=lambda p, i: {
            "FirstName": p.first,
            "LastName": p.last,
            "EmailAddress": p.email,
            "MobilePhone": p.phone,
            "LinkedInUrl": f"https://www.linkedin.com/in/{p.li}",
            "JobTitle": p.title,
            "Seniority": p.level,
            "City": p.city,
            "Country": p.country,
            "CompanyName": p.company,
            "CompanyWebsite": f"www.{p.domain}",
            "NumberOfEmployees": str(p.headcount),
            "Industry": p.industry,
            "WorkHistory": [{"JobTitle": j.title, "CompanyName": j.company, "StartDate": j.start, "EndDate": j.end} for j in p.jobs],
            "Skills": [{"Name": s} for s in p.skills],
            "LastModified": iso(p.updated),
            "RecordId": f"R{i}",
        },
        truth={
            "first_name": "FirstName",
            "last_name": "LastName",
            "full_name": {"from": ["FirstName", "LastName"], "transform": "join"},
            "email": {"from": "EmailAddress", "transform": ["trim", "lower"]},
            "phone": {"from": "MobilePhone", "transform": "normalize_phone"},
            "title": "JobTitle",
            "seniority": "Seniority",
            "linkedin_url": {"from": "LinkedInUrl", "transform": "normalize_linkedin"},
            "location.city": "City",
            "location.country": "Country",
            "company.name": "CompanyName",
            "company.domain": {"from": "CompanyWebsite", "transform": "to_domain"},
            "company.headcount": {"from": "NumberOfEmployees", "transform": {"cast": "integer"}},
            "company.industry": "Industry",
            "experiences[]": {"from": "WorkHistory", "item": {"title": "JobTitle", "company": "CompanyName", "start": {"from": "StartDate", "transform": "parse_date"}, "end": {"from": "EndDate", "transform": "parse_date"}}},
            "skills[]": {"from": "Skills", "pluck": "Name"},
            "updated_at": {"from": "LastModified", "transform": "parse_date"},
        },
    ),
    Provider(
        name="prefixed codes",
        payload=lambda p, i: {
            "rec": i,
            "p_fn": p.first,
            "p_ln": p.last,
            "p_eml": p.email,
            "p_mob": p.phone,
            "p_lnkd": f"https://linkedin.com/in/{p.li}",
            "p_ttl": p.title,
            "c_nm": p.company,
            "c_dom": p.domain,
            "c_emp": p.headcount,
            "c_ind": p.industry,
            "c_fnd": p.founded,
            "loc_city": p.city,
            "loc_ctry": p.country,
            "upd_ts": p.updated,
        },
        truth={
            "first_name": "p_fn",
            "last_name": "p_ln",
            "full_name": {"from": ["p_fn", "p_ln"], "transform": "join"},
            "email": {"from": "p_eml", "transform": ["trim", "lower"]},
            "phone": {"from": "p_mob", "transform": "normalize_phone"},
            "title": "p_ttl",
            "linkedin_url": {"from": "p_lnkd", "transform": "normalize_linkedin"},
            "location.city": "loc_city",
            "location.country": "loc_ctry",
            "company.name": "c_nm",
            "company.domain": {"from": "c_dom", "transform": "to_domain"},
            "company.headcount": "c_emp",
            "company.industry": "c_ind",
            "company.founded": {"from": "c_fnd", "transform": "parse_date"},
            "updated_at": {"from": "upd_ts", "transform": "parse_date"},
        },
    ),
]
