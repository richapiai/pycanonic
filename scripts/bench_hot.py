"""The hot-path table of task 24b over the 64 bench bodies (6 KB each), median of `measure`.
Run after `maturin develop --release`:
    .venv/bin/python scripts/bench_hot.py
"""

import json
from pathlib import Path

from pycanonic import _core
from pycanonic.cli.measure import fmt, measure

FIXTURES = Path(__file__).resolve().parents[1] / "crates/canonic-core/tests/fixtures"


def load(name):
    return json.loads((FIXTURES / name).read_text())


spec, schema = load("specs/bench.json"), load("specs/canonical/bench.json")
a = _core.compile(spec, schema, None)
# canonic/bench/person.js `anchoredSpec`, as benches/scan.rs builds it.
spec["anchors"] = {"email": '"workEmail":', "phone": '"mobile":', "company.domain": '"website":'}
spec["fields"]["email"] = {"from": "contact.workEmail", "transform": ["trim", "lower"]}
anchored = _core.compile(spec, schema, None)

raws = load("bench_bodies.json")["raws"]
bodies = [r.encode() for r in raws]
dicts = [json.loads(r) for r in raws]


def reads(v):
    return v.email, v.phone, v.company.domain


cases = [
    ("map(bytes)", lambda i: a.map(bodies[i])),
    ("map(dict)", lambda i: a.map(dicts[i])),
    ("map_json(bytes)", lambda i: a.map_json(bodies[i])),
    ("extract scanner", lambda i: a.extract(bodies[i])),
    ("extract anchored", lambda i: anchored.extract(bodies[i])),
    ("view(dict) + 3 reads", lambda i: reads(a.view(dicts[i]))),
    ("view(bytes) + 3 reads", lambda i: reads(a.view(bodies[i]))),
]

for (name, _), ns in zip(cases, measure(cases)):
    print(f"{name:24} {fmt(ns):>10}")
