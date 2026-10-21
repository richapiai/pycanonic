"""Port of canonic's bench/streaming.js: read the 5 routing fields from ~10.7 KB bodies
(routing.py), whole and in chunks, every output checked against the hand-written read first.
The npm streaming parsers there have no counterpart here yet; the references are orjson and json.
Reported, not gated.    python bench/streaming.py"""

import json
import platform
import sys
from pathlib import Path

import orjson

sys.path.insert(0, str(Path(__file__).parent))

import routing  # noqa: E402
from measure import fmt, measure  # noqa: E402
from pycanonic import compile_spec  # noqa: E402

scan = compile_spec(routing.spec, routing.canonical)
anchored = compile_spec(routing.anchored_spec, routing.canonical)
bodies = [routing.raw(i).encode() for i in range(64)]
read = routing.hand_read


def split(b, size):
    return [b[k : k + size] for k in range(0, len(b), size)]


def canonic_stream(pieces):
    s = scan.stream()
    for c in pieces:
        if s.push(c):
            break
    return s.result()


CASES = [("whole body", {
    "canonic extract": lambda i: scan.extract(bodies[i]),
    "canonic extract, anchored": lambda i: anchored.extract(bodies[i]),
    "orjson.loads + read": lambda i: read(orjson.loads(bodies[i])),
    "json.loads + read": lambda i: read(json.loads(bodies[i])),
})]
for size in (256, 1024, 4096):
    pieces = [split(b, size) for b in bodies]
    CASES.append((f"{size} B chunks" if size < 1024 else f"{size // 1024} KB chunks", {
        "canonic stream": lambda i, p=pieces: canonic_stream(p[i]),
        "join + orjson.loads + read": lambda i, p=pieces: read(orjson.loads(b"".join(p[i]))),
    }))

for name, fns in CASES:
    for label, fn in fns.items():
        for i in range(64):
            if fn(i) != read(orjson.loads(bodies[i])):
                raise SystemExit(f"output mismatch: {name}: {label}, body {i}")

v = sys.version_info
kb = sum(map(len, bodies)) / len(bodies) / 1024
print(f"Python {v.major}.{v.minor}.{v.micro} {sys.platform}-{platform.machine()}")
print(f"5 routing fields from {kb:.1f} KB bodies, fields at 10% to 90% of the body; identical output checked first\n")
print(f"{'case':<15}{'implementation':<30}{'per op':>11}{'ops/sec':>12}{'vs canonic':>12}")
for name, fns in CASES:
    ns = measure(list(fns.items()))
    for k, label in enumerate(fns):
        vs = "" if k == 0 else f"{ns[k] / ns[0]:.1f}x"
        print(f"{name:<15}{label:<30}{fmt(ns[k]):>11}{round(1e9 / ns[k]):>12,}{vs:>12}")
print("\n'vs canonic' = time / the first row of the case (above 1.0x is slower).")
