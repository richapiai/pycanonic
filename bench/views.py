"""Port of canonic's bench/views.js: view cost against schema size. Constructing a view should
cost the same at 20 fields as at 2,000; map() writes every field and grows with the schema.
Reported, not gated.    python bench/views.py"""

import platform
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from measure import fmt, measure  # noqa: E402
from pycanonic import compile_spec  # noqa: E402

SIZES = [20, 150, 500, 2000]
PER_GROUP = 25


def build(n):
    """n string fields in groups of 25, each group one nested object on both sides."""
    properties, fields = {}, {}
    for k in range(n):
        g = f"g{k // PER_GROUP}"
        properties.setdefault(g, {"type": "object", "properties": {}})["properties"][f"f{k}"] = {"type": "string"}
        fields[f"{g}.f{k}"] = f"src_{g}.v{k}"
    properties["email"] = {"type": "string", "semantic": "email", "routing": True}
    fields["email"] = "contact.email"
    spec = {"provider": "bench", "endpoint": "views", "version": 1, "status": "promoted", "fields": fields}
    artifact = compile_spec(spec, {"type": "object", "properties": properties})
    payloads = []
    for i in range(64):
        p = {"contact": {"email": f"user{i}@example.com"}}
        for k in range(n):
            p.setdefault(f"src_g{k // PER_GROUP}", {})[f"v{k}"] = f"value {i}.{k}"
        payloads.append(p)
    return artifact, payloads


v = sys.version_info
print(f"Python {v.major}.{v.minor}.{v.micro} {sys.platform}-{platform.machine()}\n")
print(f"{'fields':>7}{'view':>12}{'view + 1 read':>16}{'map':>12}")
for n in SIZES:
    a, payloads = build(n)
    for p in payloads:
        if a.view(p).to_dict() != a.map(p):
            raise SystemExit(f"output mismatch: view and map at {n} fields")
    construct, read, full = measure([
        ("construct", lambda i: a.view(payloads[i])),
        ("read", lambda i: a.view(payloads[i]).email),
        ("map", lambda i: a.map(payloads[i])),
    ])
    print(f"{n:>7}{fmt(construct):>12}{fmt(read):>16}{fmt(full):>12}")
