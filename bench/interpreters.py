"""pycanonic against the Python mapping libraries: a port of canonic's bench/interpreters.js. The same
501-field nested mapping and the same payloads go to every library, and every library must produce
the same output as pycanonic before anything is timed.

    python bench/interpreters.py          prints the table
    python bench/interpreters.py --gate   exits 1 unless every mapping library is at least 10x slower
                                          than pycanonic, bytes in and dict in (reference rows are not gated)
    --exempt FILE                         libraries the gate reports but does not fail on, by
                                          distribution name, with the reason (default:
                                          bench/interpreters-exempt.json; the README names them)

`jmespath` and `jmespath-community` install the same `jmespath` module, and `jsonata-python` and
`jsonata-rs` the same `jsonata` module, so one environment holds one of each pair. requirements.txt
pins the first of each, requirements-alt.txt the second; the rows are labelled with whichever is installed.
"""

import argparse
import importlib.metadata as md
import json
import platform
import subprocess
import sys
from pathlib import Path
from typing import Annotated, Any

import glom
import jmespath
import jq
import jsonata
import jsonbender as jb
import jsonpath_ng
import orjson
from pydantic import AliasPath, BeforeValidator, Field, StringConstraints, create_model, model_validator

sys.path.insert(0, str(Path(__file__).parent))

import pytransforms  # noqa: E402
from measure import fmt, measure  # noqa: E402
from pycanonic import compile_spec  # noqa: E402

# ---------- the mapping, as neutral rules every library is built from (interpreters.js) ----------
# [canonical group, source base, field count]; company.hq fields sit on org itself as hq<k>.
GROUPS = [
    ("identity", ["person"], 40), ("contact", ["person", "contact"], 30), ("location", ["geo"], 20),
    ("company", ["org"], 60), ("company.funding", ["org", "financials"], 20), ("company.metrics", ["org", "kpis"], 30),
    ("company.hq", ["org"], 15), ("social", ["profiles"], 20), ("signals", ["intent", "signals"], 40),
    ("meta", ["_meta"], 20), ("scores", ["scoring"], 40),
]
# [canonical path, source, fields per item, items per payload]
ARRAYS = [("contact.emails", "emailAddresses", 4, 5), ("experiences", "workHistory", 12, 8), ("education", "schools", 10, 3)]
SKILLS = 20
TRANSFORMS = [None, "toNumber", "trimLower", "toBool"]

rules: list[dict[str, Any]] = []
for group, base, n in GROUPS:
    prefix = "hq" if group == "company.hq" else "s"
    rules += [{"from": [*base, f"{prefix}{k}"], "to": [*group.split("."), f"f{k}"], "t": TRANSFORMS[k % 4]} for k in range(n)]
for to, src, n, _ in ARRAYS:
    rules.append({"from": [src], "to": to.split("."), "item": [{"from": [f"s{k}"], "to": [f"f{k}"], "t": TRANSFORMS[k % 4]} for k in range(n)]})
rules.append({"from": ["tags"], "to": ["skills"], "pluck": "label"})


# ---------- payloads ----------
def value(t, seed):
    if t == "toNumber":
        return str(seed * 7919 % 100000)
    if t == "trimLower":
        return f"  Value {seed} MiXeD  "
    if t == "toBool":
        return "true" if seed % 2 else "false"
    return f"v{seed}"


def set_path(o, path, v):
    for k in path[:-1]:
        o = o.setdefault(k, {})
    o[path[-1]] = v


def gen_payload(seed):
    """genPayload in interpreters.js. Deterministic: every value comes from a counter started at `seed`."""
    p: dict[str, Any] = {}
    s = seed
    for r in rules:
        if "pluck" in r:
            tags = []
            for _ in range(SKILLS):
                s += 1
                tags.append({"label": f"skill{s - 1}", "weight": s % 10})  # JS reads `s` after `s++`
            set_path(p, r["from"], tags)
        elif "item" in r:
            count = next(c for to, _, _, c in ARRAYS if to == ".".join(r["to"]))
            items = []
            for _ in range(count):
                items.append({i["from"][0]: value(i["t"], s + k) for k, i in enumerate(r["item"])})
                s += len(r["item"])
            set_path(p, r["from"], items)
        else:
            set_path(p, r["from"], value(r["t"], s))
            s += 1
    return p


# 64 payloads rather than the JavaScript's 100, because `measure` cycles through indices 0..63.
payloads = [gen_payload(i + 1) for i in range(64)]
raws = [orjson.dumps(p) for p in payloads]

# ---------- pycanonic ----------
CANONIC_T = {"toNumber": {"cast": "number"}, "trimLower": ["trim", "lower"], "toBool": {"cast": "boolean"}}


def leaf(src, t, with_t):
    return {"from": src, "transform": CANONIC_T[t]} if with_t and t else {"from": src}


def canonic_spec(with_t):
    fields = {}
    for r in rules:
        if "pluck" in r:
            fields[".".join(r["to"]) + "[]"] = {"from": ".".join(r["from"]), "pluck": r["pluck"]}
        elif "item" in r:
            item = {i["to"][0]: leaf(i["from"][0], i["t"], with_t) for i in r["item"]}
            fields[".".join(r["to"]) + "[]"] = {"from": ".".join(r["from"]), "item": item}
        else:
            fields[".".join(r["to"])] = leaf(".".join(r["from"]), r["t"], with_t)
    return {"provider": "bench", "endpoint": "person.enrich", "version": 1, "status": "promoted", "fields": fields}


def canonical_schema():
    root: dict[str, Any] = {"type": "object", "properties": {}}
    for r in rules:
        parent = root
        for k in r["to"][:-1]:
            parent = parent["properties"].setdefault(k, {"type": "object", "properties": {}})
        name = r["to"][-1]
        if "pluck" in r:
            parent["properties"][name] = {"type": "array", "items": {"type": "string"}}
        elif "item" in r:
            parent["properties"][name] = {"type": "array", "items": {"type": "object", "properties": {i["to"][0]: {} for i in r["item"]}}}
        else:
            parent["properties"][name] = {}
    return root


SCHEMA = canonical_schema()
CANONIC = compile_spec(canonic_spec(True), SCHEMA)
CANONIC_PLAIN = compile_spec(canonic_spec(False), SCHEMA)  # no transforms, checks the JMESPath row


# ---------- the same mapping in each library ----------
def to_number(v):
    """JavaScript `Number(v)` for the bench values, returning an int when integral as canonic's cast does."""
    n = float(v)
    return int(n) if n.is_integer() else n


T = {"toNumber": to_number, "trimLower": lambda v: v.strip().lower(), "toBool": lambda v: v == "true"}
array_rules = [r for r in rules if "item" in r]
pluck_rule = next(r for r in rules if "pluck" in r)


def tree(leaf_of, array_of, skills):
    """The canonical output shape with `leaf_of(rule)` at each scalar, `array_of(rule)` at each array."""
    t: dict[str, Any] = {}
    for r in rules:
        if "pluck" in r:
            continue
        n = t
        for k in r["to"][:-1]:
            n = n.setdefault(k, {})
        n[r["to"][-1]] = array_of(r) if "item" in r else leaf_of(r)
    t["skills"] = skills
    return t


def emit(n, key, sep=": "):
    """An object literal in a query language, `key` quoting each name."""
    return n if isinstance(n, str) else "{" + ",".join(f"{key(k)}{sep}{emit(v, key, sep)}" for k, v in n.items()) + "}"


def version(dist):
    return md.version(dist)


def dist_of(module):
    return md.packages_distributions()[module][0]


# glom: a spec tree; (path, fn) applies a transform, (path, [subspec]) maps a list.
def gleaf(r):
    return (".".join(r["from"]), T[r["t"]]) if r["t"] else ".".join(r["from"])


glom_spec = tree(gleaf, lambda a: (a["from"][0], [{i["to"][0]: gleaf(i) for i in a["item"]}]), ("tags", ["label"]))


def glom_map(p):
    return glom.glom(p, glom_spec)


# JMESPath: one compiled expression. Plain jmespath has no string functions for these transforms,
# so it runs without them, as in interpreters.js. jmespath-community has lower, trim and to_number.
JMES_COMMUNITY = dist_of("jmespath") == "jmespath-community"


def mleaf(path, t):
    if not JMES_COMMUNITY or not t:
        return path
    return {"toNumber": f"to_number({path})", "trimLower": f"lower(trim({path}))", "toBool": f"{path} == 'true'"}[t]


jmes = jmespath.compile(emit(tree(
    lambda r: mleaf(".".join(r["from"]), r["t"]),
    lambda a: f"{a['from'][0]}[].{{{','.join(f'{i['to'][0]}: {mleaf(i['from'][0], i['t'])}' for i in a['item'])}}}",
    "tags[].label",
), str))


# jsonpath-ng: one compiled path per field; the output tree is built around them.
def jp_leaf(r):
    return ("leaf", jsonpath_ng.parse(".".join(r["from"])), T.get(r["t"]))


jp_tree = tree(
    jp_leaf,
    lambda a: ("array", jsonpath_ng.parse(f"{a['from'][0]}[*]"), {i["to"][0]: jp_leaf(i) for i in a["item"]}),
    ("pluck", jsonpath_ng.parse("tags[*].label"), None),
)


def jp_build(n, doc):
    if isinstance(n, dict):
        return {k: jp_build(v, doc) for k, v in n.items()}
    kind, path, x = n
    if kind == "leaf":
        v = path.find(doc)[0].value
        return x(v) if x else v
    if kind == "array":
        return [jp_build(x, m.value) for m in path.find(doc)]
    return [m.value for m in path.find(doc)]


def jsonpath_map(p):
    return jp_build(jp_tree, p)


# jsonbender: S selects, >> F(fn) transforms, Forall maps a list.
def bleaf(r):
    s = jb.S(*r["from"])
    return s >> jb.F(T[r["t"]]) if r["t"] else s


bender = tree(
    bleaf,
    lambda a: jb.S(a["from"][0]) >> jb.Forall.bend({i["to"][0]: bleaf(i) for i in a["item"]}),
    jb.S("tags") >> jb.Forall(lambda e: e["label"]),
)


def bender_map(p):
    return jb.bend(bender, p)


# pydantic: one model per output object, fields read by AliasPath; transforms are pydantic's own
# types (lax int | float and bool parse the strings, StringConstraints trims and lowers).
PYD_TYPES = {
    None: str,
    "toNumber": int | float,
    "trimLower": Annotated[str, StringConstraints(strip_whitespace=True, to_lower=True)],
    "toBool": bool,
}


def whole_body(groups):
    """Nested output groups read absolute source paths, so each one is handed the whole body."""
    def hook(cls, d):
        return {**d, **dict.fromkeys(groups, d)}

    return model_validator(mode="before")(classmethod(hook))


def pyd_model(name, node):
    fields, groups = {}, []
    for k, v in node.items():
        if isinstance(v, dict):
            fields[k] = (pyd_model(f"{name}_{k}", v), ...)
            groups.append(k)
        else:
            fields[k] = v
    return create_model(name, __validators__={"whole_body": whole_body(tuple(groups))}, **fields)


def pyd_field(r, relative=False):
    src = r["from"][-1] if relative else AliasPath(*r["from"])
    return (PYD_TYPES[r["t"]], Field(validation_alias=src))


Person = pyd_model("Person", tree(
    pyd_field,
    lambda a: (list[create_model(f"Item_{a['from'][0]}", **{i["to"][0]: pyd_field(i, True) for i in a["item"]})], Field(validation_alias=a["from"][0])),
    (Annotated[list[str], BeforeValidator(lambda v: [e["label"] for e in v])], Field(validation_alias="tags")),
))


def pydantic_map(p):
    return Person.model_validate(p).model_dump()


# JSONata: the expression from interpreters.js.
def ja_name(k):
    return f"`{k}`"


def jsleaf(x, t):
    return {"toNumber": f"$number({x})", "trimLower": f"$lowercase($trim({x}))", "toBool": f'({x} = "true" ? true : false)'}.get(t, x)


def jpath(path):
    return ".".join(map(ja_name, path))


jsonata_expr = jsonata.Jsonata(emit(tree(
    lambda r: jsleaf(jpath(r["from"]), r["t"]),
    lambda a: f"{ja_name(a['from'][0])}.{{{','.join(f'\"{i['to'][0]}\": {jsleaf(ja_name(i['from'][0]), i['t'])}' for i in a['item'])}}}",
    "`tags`.`label`",
), lambda k: f'"{k}"'))


def jsonata_map(p):
    return jsonata_expr.evaluate(p)


# jq (C library via jq.py): one compiled program.
def qleaf(path, t):
    x = "." + ".".join(f'"{k}"' for k in path)
    return {"toNumber": f"({x}|tonumber)", "trimLower": f"({x}|trim|ascii_downcase)", "toBool": f'({x} == "true")'}.get(t, x)


jq_prog = jq.compile(emit(tree(
    lambda r: qleaf(r["from"], r["t"]),
    lambda a: f"[.\"{a['from'][0]}\"[] | {{{','.join(f'\"{i['to'][0]}\": {qleaf(i['from'], i['t'])}' for i in a['item'])}}}]",
    '[.tags[].label]',
), lambda k: f'"{k}"'))


# Hand-written glue, generated from the rules only because 501 fields is too many to type (as the
# JavaScript's inlineMapper): .get chains, a nested dict literal, list comprehensions, no per-field calls.
def hand_written_source():
    def acc(base, path):
        return base + "".join(f'.get("{k}", {{}})' for k in path[:-1]) + f'.get("{path[-1]}")'

    def val(e, t):
        return {"toNumber": f"num({e})", "trimLower": f"tl({e})", "toBool": f'({e} == "true")'}.get(t, e)

    body = emit(tree(
        lambda r: val(acc("p", r["from"]), r["t"]),
        lambda a: f"[{emit({i['to'][0]: val(acc('e', i['from']), i['t']) for i in a['item']}, lambda k: f'\"{k}\"')} for e in p.get(\"{a['from'][0]}\", ())]",
        '[e.get("label") for e in p.get("tags", ())]',
    ), lambda k: f'"{k}"')
    return f"def hand_written(p):\n    return {body}\n"


ns: dict[str, Any] = {"num": to_number, "tl": lambda v: pytransforms.lower(pytransforms.trim(v))}
exec(hand_written_source(), ns)
hand_written = ns["hand_written"]

loads = orjson.loads


def jmes_label():
    d = dist_of("jmespath")
    return f"{d} {version(d)}" + ("" if JMES_COMMUNITY else ", without transforms")


def jsonata_label():
    d = dist_of("jsonata")
    return f"{d} {version(d)}"


# (name, applies the transforms?, map of a parsed body). Bytes in is orjson.loads then the map for
# every row. pydantic's own model_validate_json and jq's input_text were tried and are not faster.
LIBRARIES = [
    (f"glom {version('glom')}", True, glom_map),
    (jmes_label(), JMES_COMMUNITY, jmes.search),
    (f"jsonpath-ng {version('jsonpath-ng')}", True, jsonpath_map),
    (f"jsonbender {version('jsonbender')}", True, bender_map),
    (f"pydantic {version('pydantic')}", True, pydantic_map),
    (jsonata_label(), True, jsonata_map),
    (f"jq {version('jq')} (jq.py)", True, lambda p: jq_prog.input_value(p).first()),
]
# Not a mapper, never gated: plain Python glue, for context.
REFERENCES = [("hand-written py (pure-Python transforms)", True, hand_written)]


def norm(o):
    """The JavaScript's JSON round trip: compare serialized output, keys sorted, so 1 and 1.0 or 1 and True differ."""
    return orjson.dumps(o, option=orjson.OPT_SORT_KEYS)


def main(gate, exempt):
    for k in range(64):
        want = norm(CANONIC.map(payloads[k]))
        if norm(CANONIC.map(raws[k])) != want or orjson.loads(CANONIC.map_json(raws[k])) != CANONIC.map(payloads[k]):
            raise SystemExit(f"pycanonic bytes in differs from dict in on payload {k}")
    ok = []
    for row in LIBRARIES + REFERENCES:
        name, transforms, fn = row
        ref = CANONIC if transforms else CANONIC_PLAIN
        bad = [k for k in range(64) if norm(fn(payloads[k])) != norm(ref.map(payloads[k]))]
        if bad:
            print(f"EXCLUDED {name}: output differs from pycanonic on payload {bad[0]}")
        else:
            ok.append(row)
    libs = [r for r in ok if r in LIBRARIES]

    cases = [
        ("bytes in", [("pycanonic map(bytes)", lambda i: CANONIC.map(raws[i])), ("pycanonic map_json(bytes), JSON out", lambda i: CANONIC.map_json(raws[i]))]
         + [(n, lambda i, f=f: f(loads(raws[i]))) for n, _, f in ok] + [("orjson.loads, parse only", lambda i: loads(raws[i]))]),
        ("dict in", [("pycanonic map(dict)", lambda i: CANONIC.map(payloads[i]))] + [(n, lambda i, f=f: f(payloads[i])) for n, _, f in ok]),
    ]
    v = sys.version_info
    head = subprocess.run(["git", "-C", str(Path(__file__).parent), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    print(f"Python {v.major}.{v.minor}.{v.micro} {sys.platform}-{platform.machine()}, pycanonic {head}, 501-field nested payload, same mapping for all")
    print("correctness: every row below produced pycanonic's output on all 64 payloads before timing\n")
    print(f"{'case':<10}{'implementation':<46}{'per map':>12}{'maps/sec':>12}{'vs pycanonic':>14}")
    failed = []
    for case, fns in cases:
        times = measure(fns)
        for (name, _), t in zip(fns, times):
            ratio = t / times[0]
            mapper = any(name == r[0] for r in libs)
            note = "" if name.startswith("pycanonic map(") else f"{ratio:.1f}x"
            if mapper and ratio < 10:
                if name.split()[0] in exempt:
                    note += " exempt"
                else:
                    failed.append(f"{case}: {name} is {ratio:.1f}x")
                    note += " <10x"
            print(f"{case:<10}{name:<46}{fmt(t):>12}{round(1e9 / t):>12,}{note:>14}")
    print("\n'vs pycanonic' = time / pycanonic map time in the same case (above 1x is slower).")
    print("Not mappers, never gated: orjson.loads, hand-written py, pycanonic map_json.")
    for lib, why in exempt.items():
        print(f"Exempt from the 10x gate: {lib}: {why}")
    if gate:
        sys.stdout.flush()
        for f in failed:
            print(f"FAIL {f}", file=sys.stderr)
        if failed or len(libs) < len(LIBRARIES):
            sys.exit(1)
        print("gate: every mapping library not exempt is at least 10x slower than pycanonic, bytes in and dict in")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--gate", action="store_true")
    ap.add_argument("--exempt", type=Path, default=Path(__file__).parent / "interpreters-exempt.json")
    args = ap.parse_args()
    main(args.gate, json.loads(args.exempt.read_text()))
