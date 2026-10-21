"""The benchmark suite, a port of canonic's bench/run.js.

    python bench/run.py                     prints the table
    python bench/run.py --gate              fails if a canonic/reference ratio is >15% worse than baseline.json for this platform
    python bench/run.py --selftest          checks that an injected 20% slowdown trips the gate
    python bench/run.py --update-baseline   records this machine's ratios under its platform key
    python bench/run.py --vs-ts ../canonic  runs the TypeScript bench in that checkout and fails if a gated row is slower
"""

import json
import platform as _platform
import re
import subprocess
import sys
from pathlib import Path

import orjson

sys.path.insert(0, str(Path(__file__).parent))

import person  # noqa: E402
import routing  # noqa: E402
from measure import fmt, measure  # noqa: E402
from pycanonic import compile_spec  # noqa: E402

TOLERANCE = 1.15
HERE = Path(__file__).parent
BASELINE = HERE / "baseline.json"
PLATFORM = f"{sys.platform}-{_platform.machine()}"
TS = json.loads((HERE / "ts_outputs.json").read_text())

artifact = compile_spec(person.spec, person.canonical)
anchored = compile_spec(person.anchored_spec, person.canonical)
raws = [person.raw(i).encode() for i in range(64)]
parsed = [orjson.loads(r) for r in raws]

scan5 = compile_spec(routing.spec, routing.canonical)
anchored5 = compile_spec(routing.anchored_spec, routing.canonical)
big_raws = [routing.raw(i).encode() for i in range(64)]
big_parsed = [orjson.loads(r) for r in big_raws]
chunks = [[r[k : k + 1024] for k in range(0, len(r), 1024)] for r in big_raws]

hand_written, hand_routing, hand_read = person.hand_written, person.hand_routing, routing.hand_read


def view_reads(v):
    return v.email, v.phone, v.company.domain


def stream(a, pieces):
    s = a.stream()
    for c in pieces:
        if s.push(c):
            break
    return s.result()


# Identical output first; a fast wrong answer is not a benchmark.
for i in range(64):
    want = hand_written(parsed[i])
    assert want == TS["expected"][i]
    assert artifact.map(raws[i]) == want
    assert artifact.map(parsed[i]) == want
    assert artifact.map_json(raws[i]) == orjson.dumps(want)
    assert artifact.view(parsed[i]).to_dict() == want
    assert view_reads(artifact.view(parsed[i])) == tuple(hand_routing(parsed[i]).values())
    assert artifact.extract(raws[i]) == hand_routing(parsed[i]) == TS["routing"][i]
    assert anchored.extract(raws[i]) == hand_routing(parsed[i]) == TS["anchored"][i]
    assert scan5.extract(big_raws[i]) == hand_read(big_parsed[i]) == TS["hand_read"][i]
    assert anchored5.extract(big_raws[i]) == hand_read(big_parsed[i])
    assert stream(scan5, chunks[i]) == hand_read(big_parsed[i])

loads = orjson.loads
CASES = [
    ("map, bytes in", lambda i: artifact.map(raws[i]), {
        "orjson.loads + hand-written py": lambda i: hand_written(loads(raws[i])),
        "orjson.loads (input)": lambda i: loads(raws[i]),
        "json.loads (input)": lambda i: json.loads(raws[i]),
    }),
    ("map, JSON out", lambda i: artifact.map_json(raws[i]), {
        "orjson.loads + hand-written py + orjson.dumps": lambda i: orjson.dumps(hand_written(loads(raws[i]))),
    }),
    ("map, parsed in", lambda i: artifact.map(parsed[i]), {
        "hand-written py": lambda i: hand_written(parsed[i]),
    }),
    ("view, construct", lambda i: artifact.view(parsed[i]), {
        "dict literal": lambda i: {"s": parsed[i]},
    }),
    ("view, 3 reads", lambda i: view_reads(artifact.view(parsed[i])), {
        "hand-written py routing": lambda i: hand_routing(parsed[i]),
    }),
    ("extract routing", lambda i: artifact.extract(raws[i]), {
        "orjson.loads + read": lambda i: hand_routing(loads(raws[i])),
        "orjson.loads (input)": lambda i: loads(raws[i]),
    }),
    ("extract, anchored", lambda i: anchored.extract(raws[i]), {
        "orjson.loads + read": lambda i: hand_routing(loads(raws[i])),
    }),
    ("extract 5, 10.7 KB", lambda i: scan5.extract(big_raws[i]), {
        "orjson.loads + read": lambda i: hand_read(loads(big_raws[i])),
    }),
    ("extract 5, anchored", lambda i: anchored5.extract(big_raws[i]), {
        "orjson.loads + read": lambda i: hand_read(loads(big_raws[i])),
    }),
    ("stream, 1 KB chunks", lambda i: stream(scan5, chunks[i]), {
        "join + orjson.loads + read": lambda i: hand_read(loads(b"".join(chunks[i]))),
    }),
]


def slowed(fn, extra):
    """Adds `extra` of the function's own work: at 0.2, every fifth call runs twice."""
    acc = 0.0

    def run(i):
        nonlocal acc
        out = fn(i)
        acc += extra
        if acc >= 1:
            acc -= 1
            fn(i)
        return out

    return run


def run(extra=None):
    """`extra=None` times the functions bare; a number wraps canonic in `slowed` (the self-test
    wraps both of its runs, so the wrapper's own call cost cancels out)."""
    ratios, rows = {}, []
    for name, canonic, refs in CASES:
        fns = [("canonic", canonic if extra is None else slowed(canonic, extra)), *refs.items()]
        ns = measure(fns)
        rows += [(name, label, ns[k], ns[k] / ns[0]) for k, (label, _) in enumerate(fns)]
        for k in range(1, len(fns)):
            ratios[f"{name} / {fns[k][0]}"] = ns[0] / ns[k]
    return ratios, rows


def commit(path):
    out = subprocess.run(["git", "-C", str(path), "rev-parse", "--short", "HEAD"], capture_output=True, text=True)
    return out.stdout.strip() or "unknown"


def kb(xs):
    return f"{sum(map(len, xs)) / len(xs) / 1024:.1f}"


def table(rows):
    v = sys.version_info
    print(f"Python {v.major}.{v.minor}.{v.micro} {PLATFORM}, pycanonic {commit(HERE)}, payloads from canonic {TS['ts_commit']}")
    print(f"person.enrich: {kb(raws)} KB payloads, {len(person.spec['fields'])} spec fields; extract 5: {kb(big_raws)} KB, fields at 10% to 90% of the body\n")
    print(f"{'case':<22}{'implementation':<48}{'per op':>11}{'ops/sec':>13}{'canonic is':>13}")
    for name, label, ns, rel in rows:
        vs = "" if label == "canonic" else f"{1 / rel:.2f}x"
        print(f"{name:<22}{label:<48}{fmt(ns):>11}{round(1e9 / ns):>13,}{vs:>13}")
    print("\n'canonic is' = canonic time / reference time (below 1.00x is faster).")


def gate(ratios, baseline):
    worse = [(k, r) for k, r in ratios.items() if k in baseline and r > baseline[k] * TOLERANCE]
    for k, r in worse:
        print(f"FAIL {k}: {r:.3f}, baseline {baseline[k]:.3f} (limit {baseline[k] * TOLERANCE:.3f})", file=sys.stderr)
    return not worse


# --- --vs-ts --------------------------------------------------------------------------------------

# Python row -> (TypeScript rows whose times add up to the same work, gated?). TypeScript has no
# bytes-in path, so bytes in and JSON out are compared with JSON.parse plus map on the parsed object.
VS_TS = {
    "map, bytes in": ([("map", "canonic"), ("map", "JSON.parse (input)")], True),
    "map, JSON out": ([("map", "canonic"), ("map", "JSON.parse (input)")], True),
    "map, parsed in": ([("map", "canonic")], False),
    "view, construct": ([("view, construct", "canonic")], False),
    "view, 3 reads": ([("view, 3 reads", "canonic")], True),
    "extract routing": ([("extract routing", "canonic")], True),
    "extract, anchored": ([("extract, anchored", "canonic")], True),
    "extract 5, 10.7 KB": ([("extract 5, 10.7 KB", "canonic")], True),
    "extract 5, anchored": ([("extract 5, anchored", "canonic")], True),
    "stream, 1 KB chunks": ([("1 KB chunks", "canonic stream")], True),
}
ROW = re.compile(r"^(\S.*?)\s{2,}(\S.*?)\s{2,}[\d.]+ (?:µs|ns)\s+([\d,]+)")


def ts_rows(checkout, script):
    """{(case, implementation): ns} from a TypeScript bench table; ns comes from ops/sec, which
    carries more digits than the per-op column."""
    out = subprocess.run(["node", f"bench/{script}"], cwd=checkout, capture_output=True, text=True, check=True).stdout
    sys.stdout.write(out + "\n")
    return {(m[1], m[2]): 1e9 / int(m[3].replace(",", "")) for m in map(ROW.match, out.splitlines()) if m}


def vs_ts(checkout, rows):
    ts = ts_rows(checkout, "run.js") | ts_rows(checkout, "streaming.js")
    py = {name: ns for name, label, ns, _ in rows if label == "canonic"}
    print(f"pycanonic {commit(HERE)} against canonic {commit(checkout)}, same session\n")
    print(f"{'case':<22}{'TypeScript rows':<38}{'TypeScript':>12}{'pycanonic':>12}{'py / ts':>10}  gate")
    ok = True
    for name, (refs, gated) in VS_TS.items():
        t = sum(ts[r] for r in refs)
        ratio = py[name] / t
        verdict = ("PASS" if ratio < 1 else "FAIL") if gated else "reported"
        ok &= verdict != "FAIL"
        label = " + ".join(c if impl == "canonic" else impl for c, impl in refs)
        print(f"{name:<22}{label:<38}{fmt(t):>12}{fmt(py[name]):>12}{ratio:>9.2f}x  {verdict}")
    print("\n'py / ts' = pycanonic time / TypeScript time (below 1.00x is faster). Gated rows fail at 1.00x or above.")
    return ok


def main(argv):
    if "--selftest" in argv:
        normal = run(0)[0]
        slow = run(0.2)[0]
        print("injected 20% slowdown, measured against this run's own ratios:")
        for k in normal:
            print(f"  {k}: {slow[k] / normal[k]:.3f}x")
        tripped = not gate(slow, normal)
        print("selftest ok: the gate fails on a 20% slowdown" if tripped else "selftest FAILED: a 20% slowdown passed the gate")
        return 0 if tripped else 1

    ratios, rows = run()
    table(rows)

    if "--update-baseline" in argv:
        all_ = json.loads(BASELINE.read_text()) if BASELINE.exists() else {}
        all_[PLATFORM] = {k: round(r, 3) for k, r in ratios.items()}
        BASELINE.write_text(json.dumps(all_, indent=2) + "\n")
        print(f"\nbaseline for {PLATFORM} written")
    elif "--gate" in argv:
        baseline = json.loads(BASELINE.read_text()).get(PLATFORM)
        if not baseline:
            # An unrecorded platform must not pass: CI would gate nothing. Paste this run's ratios into baseline.json.
            print(f"\nno baseline for {PLATFORM}; add this to bench/baseline.json (or run --update-baseline on that platform):", file=sys.stderr)
            print(json.dumps({PLATFORM: {k: round(r, 3) for k, r in ratios.items()}}, indent=2), file=sys.stderr)
            return 1
        if not gate(ratios, baseline):
            return 1
        print(f"\nbench gate: {len(baseline)} ratios within {round((TOLERANCE - 1) * 100)}% of the {PLATFORM} baseline")

    if "--vs-ts" in argv:
        print()
        if not vs_ts(Path(argv[argv.index("--vs-ts") + 1]).resolve(), rows):
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
