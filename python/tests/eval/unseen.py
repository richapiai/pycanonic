"""Providers written after the heuristics were tuned on providers.py, to measure how well the tuning
generalizes; ported from the TypeScript CLI's test/eval/unseen.ts. Add new providers here rather than
tuning against these."""

from __future__ import annotations

from providers import Provider, iso

UNSEEN: list[Provider] = [
    Provider(
        name="graph-style",
        payload=lambda p, i: {
            "@odata.id": f"users/{i}",
            "displayName": f"{p.first} {p.last}",
            "givenName": p.first,
            "surname": p.last,
            "mail": p.email,
            "mobilePhone": p.phone,
            "jobTitle": p.title,
            "city": p.city,
            "country": p.country,
            "companyName": p.company,
            "lastModifiedDateTime": iso(p.updated),
        },
        truth={
            "first_name": "givenName",
            "last_name": "surname",
            "full_name": "displayName",
            "email": {"from": "mail", "transform": ["trim", "lower"]},
            "phone": {"from": "mobilePhone", "transform": "normalize_phone"},
            "title": "jobTitle",
            "location.city": "city",
            "location.country": "country",
            "company.name": "companyName",
            "updated_at": {"from": "lastModifiedDateTime", "transform": "parse_date"},
        },
    ),
    Provider(
        name="flat-prefixed",
        payload=lambda p, i: {
            "person": {
                "name": {"fullName": f"{p.first} {p.last}", "givenName": p.first, "familyName": p.last},
                "email": p.email,
                "employment": {"name": p.company, "title": p.title, "seniority": p.level, "domain": p.domain},
                "geo": {"city": p.city, "countryCode": p.country},
                "linkedin": {"handle": f"in/{p.li}"},
            },
            "company": {"name": p.company, "domain": p.domain, "metrics": {"employees": p.headcount}, "category": {"industry": p.industry}, "foundedYear": int(p.founded[:4])},
        },
        truth={
            "first_name": "person.name.givenName",
            "last_name": "person.name.familyName",
            "full_name": "person.name.fullName",
            "email": {"from": "person.email", "transform": ["trim", "lower"]},
            "title": "person.employment.title",
            "seniority": "person.employment.seniority",
            "location.city": "person.geo.city",
            "location.country": "person.geo.countryCode",
            "company.name": "company.name",
            "company.domain": {"from": "company.domain", "transform": "to_domain"},
            "company.headcount": "company.metrics.employees",
            "company.industry": "company.category.industry",
            "company.founded": {"from": "company.foundedYear", "transform": {"cast": "string"}},
        },
    ),
    Provider(
        name="list-heavy",
        payload=lambda p, i: {
            "id": i,
            "name": {"first": p.first, "last": p.last},
            "emails": [{"value": p.email, "kind": "work"}, {"value": p.personal, "kind": "personal"}],
            "phones": [{"number": p.phone, "kind": "mobile"}],
            "title": p.title,
            "organization": {"legal_name": p.company, "domain": p.domain, "size": f"{p.headcount}"},
            "positions": [{"title": j.title, "organization": j.company, "started": j.start, "ended": j.end} for j in p.jobs],
            "skills": [{"name": s, "endorsements": i % 7} for s in p.skills],
        },
        truth={
            "first_name": "name.first",
            "last_name": "name.last",
            "full_name": {"from": ["name.first", "name.last"], "transform": "join"},
            "email": {"from": "emails[0].value", "transform": ["trim", "lower"]},
            "phone": {"from": "phones[0].number", "transform": "normalize_phone"},
            "title": "title",
            "company.name": "organization.legal_name",
            "company.domain": {"from": "organization.domain", "transform": "to_domain"},
            "company.headcount": {"from": "organization.size", "transform": {"cast": "integer"}},
            "experiences[]": {"from": "positions", "item": {"title": "title", "company": "organization", "start": {"from": "started", "transform": "parse_date"}, "end": {"from": "ended", "transform": "parse_date"}}},
            "skills[]": {"from": "skills", "pluck": "name"},
        },
    ),
]
