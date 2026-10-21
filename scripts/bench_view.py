"""View timings over the 64 bench bodies, per body: construct, and construct plus the three routing
reads canonic's bench/run.js times (email, phone, company.domain). Run after `maturin develop --release`:
    .venv/bin/python scripts/bench_view.py
"""

import json
import timeit
from pathlib import Path

from pycanonic import _core

FIXTURES = Path(__file__).resolve().parents[1] / "crates/canonic-core/tests/fixtures"


def load(name):
    return json.loads((FIXTURES / name).read_text())


a = _core.compile(load("specs/bench.json"), load("specs/canonical/bench.json"), None)
raws = load("bench_bodies.json")["raws"]


def reads(v):
    v.email, v.phone, v.company.domain


for label, srcs in (("dict", [json.loads(r) for r in raws]), ("bytes", [r.encode() for r in raws])):
    cases = {
        "view, construct": lambda: [a.view(s) for s in srcs],
        "view, 3 reads": lambda: [reads(a.view(s)) for s in srcs],
        "map": lambda: [a.map(s) for s in srcs],
    }
    for name, f in cases.items():
        best = min(timeit.repeat(f, number=2000, repeat=7)) / 2000 / len(srcs)
        print(f"{label:5}  {name:16} {best * 1e9:7.0f} ns")
