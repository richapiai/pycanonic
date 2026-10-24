"""The benchmark suite, a port of canonic's bench/run.js.

    python bench/run.py                     prints the table
    python bench/run.py --gate              fails if a canonic/reference ratio is >15% worse than baseline.json for this platform
    python bench/run.py --selftest          checks that an injected 20% slowdown trips the gate
    python bench/run.py --update-baseline   records this machine's ratios under its platform key
    python bench/run.py --vs-ts ../canonic  runs the TypeScript bench in that checkout and fails if a gated row is slower
"""

import itertools
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
hand_written_pure = person.hand_written_pure

# Language 2: the work history grouped by company, as a provider adapter does it. TypeScript has no
# language 2, so this row is not in VS_TS.
_jobs = {"type": "object", "properties": {"company": {"type": "string"}, "start": {"type": "string"}, "end": {"type": "string"}, "roles": {"type": "array", "items": {"type": "string"}}}}
grouped = compile_spec(
    {
        "language": 2, "provider": "bench", "endpoint": "person.jobs", "version": 1, "status": "promoted",
        "fields": {"jobs[]": {"from": "workHistory", "group_by": "orgName", "item": {
            "company": "orgName",
            "start": {"aggregate": "min", "from": "$group[].from"},
            "end": {"aggregate": "max", "from": "$group[].to", "nulls": "propagate"},
            "roles[]": {"from": "$group", "pluck": "role"},
        }}},
    },
    {"type": "object", "properties": {"jobs": {"type": "array", "items": _jobs}}},
)


def hand_grouped(p):
    """The grouped spec by hand: consecutive runs of one orgName, min start, max end (null if any end is null)."""
    src = p.get("workHistory")
    if not isinstance(src, list):
        return {"jobs": None}
    jobs = []
    for _, run_ in itertools.groupby(src, key=lambda e: e.get("orgName")):
        run_ = list(run_)
        starts = [e["from"] for e in run_ if e.get("from") is not None]
        ends = [e.get("to") for e in run_]
        end = None if None in ends else max(ends)
        jobs.append({"company": run_[0].get("orgName"), "start": min(starts) if starts else None, "end": end, "roles": [e.get("role") for e in run_]})
    return {"jobs": jobs}


def view_reads(v):
    return v.email, v.phone, v.company.domain


def stream(a, pieces):
    s = a.stream()
    for c in pieces:
        if s.push(c):
            break
    return s.result()


def check(ok, what):
    """Raises rather than asserts, so `python -O` cannot skip the check."""
    if not ok:
        raise SystemExit(f"output mismatch: {what}")


# Identical output first; a fast wrong answer is not a benchmark.
for i in range(64):
    want, routed, read = hand_written(parsed[i]), hand_routing(parsed[i]), hand_read(big_parsed[i])
    check(want == TS["expected"][i], f"hand-written py, body {i}")
    check(person.hand_written_pure(parsed[i]) == want, f"hand-written py (pure-Python transforms), body {i}")
    check(artifact.map(raws[i]) == want, f"map, bytes in, body {i}")
    check(artifact.map(parsed[i]) == want, f"map, parsed in, body {i}")
    check(artifact.map_json(raws[i]) == orjson.dumps(want), f"map, JSON out, body {i}")
    check(artifact.view(parsed[i]).to_dict() == want, f"view, body {i}")
    check(view_reads(artifact.view(parsed[i])) == tuple(routed.values()), f"view, 3 reads, body {i}")
    check(artifact.extract(raws[i]) == routed == TS["routing"][i], f"extract routing, body {i}")
    check(anchored.extract(raws[i]) == routed == TS["anchored"][i], f"extract, anchored, body {i}")
    check(scan5.extract(big_raws[i]) == read == TS["hand_read"][i], f"extract 5, body {i}")
    check(anchored5.extract(big_raws[i]) == read, f"extract 5, anchored, body {i}")
    check(stream(scan5, chunks[i]) == read, f"stream, body {i}")
    check(grouped.map(raws[i]) == hand_grouped(parsed[i]), f"map grouped, body {i}")
check(any(len(hand_grouped(p)["jobs"]) < len(p["workHistory"]) for p in parsed), "map grouped: no body has a group of two")

loads = orjson.loads
CASES = [
    ("map, bytes in", lambda i: artifact.map(raws[i]), {
        "orjson.loads + hand-written py": lambda i: hand_written(loads(raws[i])),
        "orjson.loads + hand-written py (pure-Python transforms)": lambda i: hand_written_pure(loads(raws[i])),
        "orjson.loads (input)": lambda i: loads(raws[i]),
        "json.loads (input)": lambda i: json.loads(raws[i]),
    }),
    ("map, JSON out", lambda i: artifact.map_json(raws[i]), {
        "orjson.loads + hand-written py + orjson.dumps": lambda i: orjson.dumps(hand_written(loads(raws[i]))),
    }),
    ("map, parsed in", lambda i: artifact.map(parsed[i]), {
        "hand-written py": lambda i: hand_written(parsed[i]),
        "hand-written py (pure-Python transforms)": lambda i: hand_written_pure(parsed[i]),
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
    # gate() checks only the ratios a platform's baseline has, so this row is reported but not gated
    # on linux-x86_64 until the bench box records its baseline.
    ("map grouped, bytes in", lambda i: grouped.map(raws[i]), {
        "orjson.loads + hand-written py": lambda i: hand_grouped(loads(raws[i])),
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


def git(path, *args):
    return subprocess.run(["git", "-C", str(path), *args], capture_output=True, text=True).stdout.strip()


def commit(path, dirty=True):
    """The short HEAD hash, with `-dirty` when the working tree has uncommitted changes."""
    head = git(path, "rev-parse", "--short", "HEAD") or "unknown"
    return head + ("-dirty" if dirty and git(path, "status", "--porcelain") else "")


def kb(xs):
    return f"{sum(map(len, xs)) / len(xs) / 1024:.1f}"


def table(rows):
    v = sys.version_info
    print(f"Python {v.major}.{v.minor}.{v.micro} {PLATFORM}, pycanonic {commit(HERE)}, payloads from canonic {TS['ts_commit']}")
    print(f"person.enrich: {kb(raws)} KB payloads, {len(person.spec['fields'])} spec fields; extract 5: {kb(big_raws)} KB, fields at 10% to 90% of the body\n")
    print(f"{'case':<22}{'implementation':<58}{'per op':>11}{'ops/sec':>13}{'canonic is':>13}")
    for name, label, ns, rel in rows:
        vs = "" if label == "canonic" else f"{1 / rel:.2f}x"
        print(f"{name:<22}{label:<58}{fmt(ns):>11}{round(1e9 / ns):>13,}{vs:>13}")
    print("\n'canonic is' = canonic time / reference time (below 1.00x is faster).")


def gate(ratios, baseline):
    worse = [(k, r) for k, r in ratios.items() if k in baseline and r > baseline[k] * TOLERANCE]
    for k, r in worse:
        print(f"FAIL {k}: {r:.3f}, baseline {baseline[k]:.3f} (limit {baseline[k] * TOLERANCE:.3f})", file=sys.stderr)
    return not worse


# --- --vs-ts --------------------------------------------------------------------------------------

# Python row -> (TypeScript rows whose times add up to the same work, gated or not). TypeScript has no
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


def ts_preflight(checkout):
    """The TypeScript side must be the pinned commit (the one ts_outputs.json came from) and its
    dist must be built from the current sources. Returns an error message, or None."""
    head = commit(checkout, dirty=False)
    if head != TS["ts_commit"]:
        return (f"--vs-ts: {checkout} is at {head}, but bench/ts_outputs.json pins canonic {TS['ts_commit']}. "
                f"Check out {TS['ts_commit']} there, or regenerate with `node bench/dump.mjs {checkout}`.")
    pkg = checkout / "packages/canonic"
    dist = pkg / "dist/index.js"
    if not dist.exists():
        return f"--vs-ts: {dist} is missing; run `pnpm build` in {checkout}."
    newest = max((f for f in (pkg / "src").rglob("*") if f.is_file()), key=lambda f: f.stat().st_mtime)
    if newest.stat().st_mtime > dist.stat().st_mtime:
        return f"--vs-ts: {dist} is older than {newest}; run `pnpm build` in {checkout}."
    return None


def vs_ts(checkout, rows):
    ts = ts_rows(checkout, "run.js") | ts_rows(checkout, "streaming.js")
    py = {name: ns for name, label, ns, _ in rows if label == "canonic"}
    print(f"pycanonic {commit(HERE)} against canonic {commit(checkout)}, same session\n")
    print(f"{'case':<22}{'TypeScript rows':<38}{'TypeScript':>12}{'pycanonic':>12}{'py / ts':>10}  gate")
    ok = True
    for name, (refs, gated) in VS_TS.items():
        missing = [r for r in refs if r not in ts]
        if missing:
            print(f"--vs-ts: no TypeScript row {missing} in the bench output. If the TypeScript table format changed, update ROW in bench/run.py.", file=sys.stderr)
            return False
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

    checkout = None
    if "--vs-ts" in argv:
        at = argv.index("--vs-ts") + 1
        if at >= len(argv) or argv[at].startswith("--"):
            print("--vs-ts needs the path to a canonic checkout: python bench/run.py --vs-ts ../canonic", file=sys.stderr)
            return 2
        checkout = Path(argv[at]).resolve()
        error = ts_preflight(checkout)
        if error:
            print(error, file=sys.stderr)
            return 1

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
            # An unrecorded platform must not pass, or CI gates nothing. Paste this run's ratios into baseline.json.
            print(f"\nno baseline for {PLATFORM}; add this to bench/baseline.json (or run --update-baseline on that platform):", file=sys.stderr)
            print(json.dumps({PLATFORM: {k: round(r, 3) for k, r in ratios.items()}}, indent=2), file=sys.stderr)
            return 1
        if not gate(ratios, baseline):
            return 1
        print(f"\nbench gate: {len(baseline)} ratios within {round((TOLERANCE - 1) * 100)}% of the {PLATFORM} baseline")

    if checkout:
        print()
        if not vs_ts(checkout, rows):
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
