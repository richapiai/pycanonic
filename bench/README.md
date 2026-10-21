# Benchmarks

## The suite

`bench/run.py` is the port of canonic's `bench/run.js`. `bench/person.py` and `bench/routing.py` build the same bodies as `person.js` and `routing.js`, byte for byte. `bench/test_payloads.py` checks this against `bench/ts_outputs.json`, which `node bench/dump.mjs ../canonic` writes. Every case checks that canonic and its references give identical output before any timing starts.

How it measures (`measure`, shared with `canonic bench`): every function runs once to warm up and once more to find a batch size that takes at least 4 ms. Then 21 rounds run every function in the case in turn, starting from a different function each round, so drift hits all of them alike. The number is the median per-op time over the 21 batches. The loop is plain Python (`for i in ix: sink = fn(i)`), so each op includes a lambda call, an index and a list step. An empty lambda measures 18 ns this way, and one that indexes the input list measures 25 ns. canonic and every reference pay the same cost. The TypeScript bench uses the same loop shape with its own JIT costs. The tables below are one run, not the best of several.

### References

Every hand-written reference calls the same transforms canonic runs, as `person.js` does with `T` and `V`. In Python those are `pycanonic.default_transforms` and `pycanonic.validators`, and each call dispatches by name into Rust. That dispatch costs about 60 ns more than a plain method call: `T["lower"]("ABC")` measures 97 ns, while `"ABC".lower()` measures 35 ns. `hand_written` makes 35 such calls per payload, so roughly 2.5 to 3 µs of its 5.2 µs goes to transform calls (summed from per-transform timings, loop cost taken out).

To show what idiomatic glue would cost, `map, parsed in` and `map, bytes in` also have a `hand-written py (pure-Python transforms)` row: the same glue over pure-Python ports of the eight transforms and three validators the bench needs (`bench/pytransforms.py`). The trivial ports are faster than the Rust calls (trim 66 ns against 90 ns, lower 61 against 83), but the ones that do real work are slower: to_domain takes 1.27 µs against 141 ns, normalize_linkedin 708 against 142 ns, and parse_date on an epoch 1.65 µs against 120 ns. In total the pure-Python reference takes about 15 µs on a parsed dict, three times the shared-transforms reference. So the shared-transforms row is the harder bar for canonic, and it is the one the gate uses.

### Against TypeScript canonic

Measured on an Apple M3 Max (36 GB, macOS 27.0) with Python 3.14.7 and Node 26.10.0, release build, in one session: pycanonic first, then canonic's `run.js` and `streaming.js`. The machine was not idle (load average about 3.5), so treat the absolute numbers as rough. Both sides ran under the same load. `--vs-ts` refuses to run unless the checkout is at the canonic commit pinned in `bench/ts_outputs.json` and `packages/canonic/dist` is newer than its sources.

```
.venv/bin/maturin develop --release
.venv/bin/pip install -r bench/requirements.txt
.venv/bin/python bench/run.py --vs-ts ../canonic
```

```
pycanonic e425f48 against canonic 936d6fd, same session

case                  TypeScript rows                         TypeScript   pycanonic   py / ts  gate
map, bytes in         map + JSON.parse (input)                  15.25 µs     5.64 µs     0.37x  PASS
map, JSON out         map + JSON.parse (input)                  15.25 µs     4.68 µs     0.31x  PASS
map, parsed in        map                                        3.45 µs     3.86 µs     1.12x  reported
view, construct       view, construct                               7 ns      109 ns    14.77x  reported
view, 3 reads         view, 3 reads                               740 ns      726 ns     0.98x  PASS
extract routing       extract routing                            8.22 µs     1.34 µs     0.16x  PASS
extract, anchored     extract, anchored                          1.22 µs      705 ns     0.58x  PASS
extract 5, 10.7 KB    extract 5, 10.7 KB                        15.63 µs     1.88 µs     0.12x  PASS
extract 5, anchored   extract 5, anchored                        3.89 µs     1.12 µs     0.29x  PASS
stream, 1 KB chunks   canonic stream                            18.20 µs     8.85 µs     0.49x  PASS

'py / ts' = pycanonic time / TypeScript time (below 1.00x is faster). Gated rows fail at 1.00x or above.
```

TypeScript has no bytes-in path, so `map, bytes in` and `map, JSON out` are compared with `JSON.parse` plus `map` on the parsed object. The stream row comes from `streaming.js` (the 5-field 10.7 KB bodies in 1 KB chunks). `map, parsed in` and `view, construct` are reported and not gated (design section 8.1). A Python call that returns an extension object costs about 100 ns before Rust runs.

The table above is from pycanonic e425f48 (after the second hot-path pass). The Python table is from the earlier run at 74dae8f:

```
Python 3.14.7 darwin-arm64, pycanonic 74dae8f, payloads from canonic 936d6fd
person.enrich: 5.9 KB payloads, 18 spec fields; extract 5: 10.7 KB, fields at 10% to 90% of the body

case                  implementation                                                 per op      ops/sec   canonic is
map, bytes in         canonic                                                       5.68 µs      176,100             
map, bytes in         orjson.loads + hand-written py                               19.54 µs       51,183        0.29x
map, bytes in         orjson.loads + hand-written py (pure-Python transforms)      30.18 µs       33,139        0.19x
map, bytes in         orjson.loads (input)                                         13.78 µs       72,582        0.41x
map, bytes in         json.loads (input)                                           33.29 µs       30,040        0.17x
map, JSON out         canonic                                                       4.92 µs      203,092             
map, JSON out         orjson.loads + hand-written py + orjson.dumps                19.31 µs       51,783        0.25x
map, parsed in        canonic                                                       3.88 µs      257,483             
map, parsed in        hand-written py                                               5.21 µs      192,019        0.75x
map, parsed in        hand-written py (pure-Python transforms)                     15.10 µs       66,220        0.26x
view, construct       canonic                                                        100 ns    9,998,310             
view, construct       dict literal                                                    43 ns   23,443,739        2.34x
view, 3 reads         canonic                                                        678 ns    1,475,648             
view, 3 reads         hand-written py routing                                        907 ns    1,102,106        0.75x
extract routing       canonic                                                       1.33 µs      750,693             
extract routing       orjson.loads + read                                          14.61 µs       68,429        0.09x
extract routing       orjson.loads (input)                                         13.88 µs       72,035        0.10x
extract, anchored     canonic                                                        714 ns    1,400,252             
extract, anchored     orjson.loads + read                                          14.25 µs       70,199        0.05x
extract 5, 10.7 KB    canonic                                                       1.88 µs      532,905             
extract 5, 10.7 KB    orjson.loads + read                                          26.16 µs       38,220        0.07x
extract 5, anchored   canonic                                                       1.12 µs      891,048             
extract 5, anchored   orjson.loads + read                                          26.23 µs       38,121        0.04x
stream, 1 KB chunks   canonic                                                       8.37 µs      119,521             
stream, 1 KB chunks   join + orjson.loads + read                                   26.36 µs       37,943        0.32x

'canonic is' = canonic time / reference time (below 1.00x is faster).
```

### Gates

- `python bench/run.py --gate` fails if any canonic/reference ratio is more than 15% worse than `bench/baseline.json` for this platform (`sys.platform-machine`). An unrecorded platform fails and prints the JSON to paste. `--selftest` adds a 20% slowdown to canonic and checks that the gate trips.
- `python bench/run.py --vs-ts <canonic checkout>` fails if any gated row is not faster than TypeScript. Run it on the bench box before a release.

`baseline.json` has `darwin-arm64` only. Until it has `linux-x86_64`, the CI `bench` job runs `--selftest` and prints the table without gating. To record the Linux baseline, run this on the Hetzner box (or a runner), then commit `bench/baseline.json` and change the job's last step to `--gate`:

```
.venv/bin/maturin develop --release && .venv/bin/python bench/run.py --update-baseline
```

The two timing smoke tests in `bench/test_run.py` are opt-in: `PYCANONIC_BENCH_TESTS=1 .venv/bin/pytest bench`.

`bench/streaming.py` (chunk sizes against whole-body `extract`) and `bench/views.py` (view against map from 20 to 2,000 fields) are reported and not gated.

## Against mapping libraries

`bench/interpreters.py` is the port of canonic's `bench/interpreters.js`. It builds the same 501-field nested payload from the same rules (`GROUPS`, `ARRAYS`, `SKILLS`, and the transforms `null`, `toNumber`, `trimLower`, `toBool`) and gives the same mapping to pycanonic and to every library. `bench/test_interpreters.py` checks that the 64 generated payloads hash to the same bytes as `JSON.stringify` of `genPayload(1..64)` in the JavaScript (the hash was taken once in Node at canonic 936d6fd, because `interpreters.js` does not export its payloads). The JavaScript uses 100 payloads; this port uses 64 because `measure` cycles through indices 0 to 63.

The rule from the owner: every published Python mapping library must be at least 10x slower than pycanonic on this mapping, bytes in and dict in. `--gate` checks exactly that and exits 1 otherwise. orjson and the hand-written row are not mappers, so they are reported and never gated.

The rule does not hold today. Three libraries are less than 10x slower: pydantic (about 2.3x to 3x), jsonbender (about 7.5x to 8x) and plain jmespath without transforms (about 9.2x to 9.6x). The reasons are below the tables.

### Method

- Identical output first. Before timing, every library maps all 64 payloads and its output must serialize to the same bytes as pycanonic's (`orjson.dumps` with sorted keys, so `1` and `1.0` or `1` and `True` count as different). A library that does not match is printed as `EXCLUDED` and not timed. `pytest bench` runs the same check on three payloads.
- `bytes in` is `orjson.loads` and then the library, which is what a Python user would write. pycanonic gets the bytes directly. pydantic's `model_validate_json` and jq's `input_text` were tried and were not faster than `orjson.loads` plus the dict call.
- `dict in` maps an already parsed dict.
- Each mapping is built once, outside the timed loop: compiled JMESPath and jq programs, parsed JSONPath paths, a glom spec, a jsonbender mapping, pydantic models made with `create_model`, a JSONata expression. Timing uses `measure`, the same loop and settings as `run.py`.
- The transforms are the JavaScript's: `toNumber` is `Number(v)` (an int when the value is whole, as pycanonic's `cast: number` returns), `trimLower` is strip then lower, `toBool` is `v == "true"`. Where a library has these built in it uses its own (JSONata `$number`, `$trim`, `$lowercase`; jq `tonumber`, `trim`, `ascii_downcase`; jmespath-community `to_number`, `trim`, `lower`; pydantic `int | float`, `bool` and `StringConstraints`). The others call the same three Python functions.

How each library expresses it:

| library | the mapping |
|---|---|
| glom | one spec tree; `(path, fn)` for a transform, `(path, [subspec])` for a list |
| jmespath | one compiled multiselect-hash expression |
| jsonpath-ng | one compiled path per field, the output tree built around them |
| jsonbender | `S(...)`, `>> F(fn)`, `Forall.bend` for lists, `Forall` for the pluck |
| pydantic | one model per output object, fields read with `AliasPath`, `model_validate` then `model_dump`. The output nests differently from the input, so each nested model gets the whole body from a small `mode="before"` model validator. |
| jsonata-python, jsonata-rs | the expression from `interpreters.js` |
| jq (jq.py) | one compiled jq program, `input_value(dict).first()` |
| hand-written py | a nested dict literal of `.get` chains and list comprehensions, generated from the rules (as the JavaScript's `inlineMapper`), with `trim` and `lower` from `pytransforms.py` |

### Results

Apple M3 Max (36 GB, macOS 27.0), Python 3.14.7, release build, pycanonic at e425f48 (after the second hot-path pass), payloads from canonic 936d6fd. The machine was not idle (load average 3.5 to 5). One run each. The first run, at d330865 before any of the large-payload work, is in the second table.

`jmespath` and `jmespath-community` install the same `jmespath` module, and `jsonata-python` and `jsonata-rs` the same `jsonata` module, so they need separate environments. `requirements.txt` holds the first of each pair and `requirements-alt.txt` the second.

```
.venv/bin/maturin develop --release
uv pip install -p .venv/bin/python -r bench/requirements.txt
.venv/bin/python bench/interpreters.py --gate

uv venv -p .venv/bin/python .venv-alt
uv pip install -p .venv-alt/bin/python -r bench/requirements-alt.txt maturin pytest
VIRTUAL_ENV="$PWD/.venv-alt" .venv-alt/bin/maturin develop --release
.venv-alt/bin/python bench/interpreters.py --gate
```

`requirements.txt`:

```
orjson
glom==25.12.0
jmespath==1.1.0
jsonpath-ng==1.12.0
jsonbender==0.9.3
pydantic==2.14.1
jsonata-python==0.7.1
jq==1.13.0
```

`requirements-alt.txt` (only the rows that differ):

```
jmespath-community==1.1.3
jsonata-rs==0.1.4
```

`vs pycanonic` is time divided by pycanonic's `map` time in the same case. Rows marked `<10x` fail the gate. Rows in both environments are shown from the main run; the alt run agreed within 5%.

| implementation | bytes in | vs pycanonic | dict in | vs pycanonic |
|---|---:|---:|---:|---:|
| pycanonic map | 38.99 µs |  | 38.02 µs |  |
| pycanonic map_json(bytes), JSON out, reported | 30.95 µs | 0.8x |  |  |
| glom 25.12.0 | 4757.68 µs | 122.0x | 4705.15 µs | 123.7x |
| jmespath 1.1.0, without transforms | 372.41 µs | 9.6x **<10x** | 348.98 µs | 9.2x **<10x** |
| jsonpath-ng 1.12.0 | 806.43 µs | 20.7x | 783.02 µs | 20.6x |
| jsonbender 0.9.3 | 313.35 µs | 8.0x **<10x** | 285.34 µs | 7.5x **<10x** |
| pydantic 2.14.1 | 118.23 µs | 3.0x **<10x** | 85.73 µs | 2.3x **<10x** |
| jsonata-python 0.7.1 | 5585.88 µs | 143.2x | 5579.45 µs | 146.7x |
| jq 1.13.0 (jq.py) | 2841.09 µs | 72.9x | 2818.44 µs | 74.1x |
| hand-written py (pure-Python transforms), not a mapper | 88.80 µs | 2.3x | 60.19 µs | 1.6x |
| orjson.loads, parse only, not a mapper | 20.68 µs | 0.5x |  |  |
| jmespath-community 1.1.3 (alt env) | 881.77 µs | 22.7x | 850.34 µs | 22.3x |
| jsonata-rs 0.1.4 (alt env) | 593.25 µs | 15.2x | 570.19 µs | 15.0x |

At d330865, before the key-table fix:

| implementation | bytes in | vs pycanonic | dict in | vs pycanonic |
|---|---:|---:|---:|---:|
| pycanonic map | 57.44 µs |  | 45.25 µs |  |
| pycanonic map_json(bytes), JSON out, reported | 52.01 µs | 0.9x |  |  |
| glom 25.12.0 | 5077.59 µs | 88.4x | 4993.76 µs | 110.4x |
| jmespath 1.1.0, without transforms | 383.60 µs | 6.7x **<10x** | 358.30 µs | 7.9x **<10x** |
| jsonpath-ng 1.12.0 | 843.66 µs | 14.7x | 798.05 µs | 17.6x |
| jsonbender 0.9.3 | 321.37 µs | 5.6x **<10x** | 292.49 µs | 6.5x **<10x** |
| pydantic 2.14.1 | 124.49 µs | 2.2x **<10x** | 91.16 µs | 2.0x **<10x** |
| jsonata-python 0.7.1 | 6296.06 µs | 109.6x | 6066.95 µs | 134.1x |
| jq 1.13.0 (jq.py) | 2917.42 µs | 50.8x | 2862.35 µs | 63.3x |
| hand-written py (pure-Python transforms), not a mapper | 92.36 µs | 1.6x | 62.56 µs | 1.4x |
| orjson.loads, parse only, not a mapper | 21.67 µs | 0.4x |  |  |
| jmespath-community 1.1.3 (alt env) | 922.55 µs | 16.3x | 912.38 µs | 19.7x |
| jsonata-rs 0.1.4 (alt env) | 602.28 µs | 10.6x | 591.61 µs | 12.8x |

For context, `node bench/interpreters.js` in the canonic checkout on the same machine in the same session: canonic 9.2 µs, hand-written JS 7.6 µs, @munesoft/api-normalizer 53.5 µs (5.8x), jmespath 0.16.0 without transforms 143.5 µs (15.6x), object-mapper 219.0 µs (23.8x), morphism 391.4 µs (42.6x), jsonata 2.2.2 1112.6 µs (121.1x). The TypeScript side has its own library under 10x too.

### Why three libraries miss 10x

- **pydantic** compiles its models once into a Rust validator tree (pydantic-core). On a parsed dict it does the same kind of work pycanonic does: walk a Python dict from Rust and build Python objects. `model_validate` alone takes about 54 µs; `model_dump` adds about 30 µs to give back a dict. It is not an interpreter in the JSONata sense, so it is the hardest bar here.
- **jsonbender** and **jmespath** are pure-Python interpreters, but light ones: about 290 to 370 µs for 501 fields, under 1 µs a field. pycanonic takes about 38 µs on a dict and 39 µs on bytes, about 76 ns a field, so both land at 7.5x to 9.6x. The pure-Python hand-written row takes 60 µs on a dict. TypeScript canonic takes 9.2 µs for the same mapping on this machine.
- Where pycanonic's 39 µs on bytes goes (Rust benches in `benches/wide.rs`, per body): the parse takes 11.8 µs for 8.7 KB that is nearly all wanted, the plan 16 µs (about 6 µs of it in the transforms: trim, lower, number and boolean casts), and building the Python objects about 10 µs. `orjson.loads` of the 7.3 KB output alone takes 19 µs, so object building has little left in it.
- On a dict, a quarter of the time is CPython's `dict` lookup of each source key: the keys `json.loads` makes are not the interned strings pycanonic looks up with, so every hit ends in a string compare. Reading the dict in order (`PyDict_Next`) instead would need `unsafe` outside `simd.rs`.
- Task 24c (second hot-path pass) brought bytes in from 48 to 39 µs and dict in from 44 to 38 µs: shared path prefixes resolved once per document, a moving read hint on large parsed objects (the key table is built only when the hint misses), the parser trying the next listed key before hashing, and faster number and boolean casts.
- jmespath runs without transforms, so it does less work than the other rows, as in the TypeScript bench. jmespath-community has `trim`, `lower` and `to_number` and runs with them (about 16x and 20x).

### Searched on PyPI

Each candidate was installed with `uv pip install` into a scratch Python 3.14 environment.

| package | result |
|---|---|
| glom, jmespath, jsonpath-ng, jsonbender, pydantic | in the table (named in the task) |
| jsonata-python 0.7.1 | in the table: a JSONata implementation in pure Python |
| jsonata-rs 0.1.4 | in the table (alt environment): JSONata in Rust; same `jsonata` module as jsonata-python |
| jmespath-community 1.1.3 | in the table (alt environment): the JMESPath fork with string functions; same `jmespath` module as jmespath |
| jq 1.13.0 | in the table: Python bindings to the jq C library, a JSON-to-JSON language |
| python-json-mapper, jsontransform, json-mapping-engine, remap | not on PyPI, or no installable version |
| jsonmapping 0.1.x | does not install: needs PyICU, which needs a system ICU build |
| json2json 0.1.0 | does not import on Python 3.10+ (`from collections import Mapping`) |
| json-mapper 0.1.15 | Python 2 only (`dict.iteritems`) |
| json-transformer 0.0.1 | does not import (`from parser import ...`, Python 2 style) |
| jsonmorph 0.0.6 | reads and writes files, and turns every value into a string, so the output cannot match |
| objectpath 0.6.1 | a query language; returns lazy generators for arrays, no trim, emits SyntaxWarnings on import, last release 2017 |
| jsonmapper 0.2 | wraps a dict in a Python object (a 2011 ODM), not JSON to JSON |
| dpath, python-benedict, dictor, jsonpointer, python-jsonpath, jsonpath-python, jsonpath-rw, jsonpath-rust-bindings | path accessors, not mappers; jsonpath-ng covers the one-path-per-field style |
| jqpy 1.0.0 | runs the jq binary in a subprocess; jq.py covers jq |
| object-mapper, dataclass-mapper, pymapper, py-object-mapper | map between Python classes, not JSON documents |
| json-logic | evaluates rules to one value; it does not build objects |
| datamorph 2.4.0 | no metadata, and its template code raises `StopIteration` inside a generator, which is an error since Python 3.7 |
| kmap, morph-py | unrelated packages that share the name |

marshmallow and cattrs are in pydantic's category (schemas over classes); pydantic is the one in the table.

### Hetzner box (to be filled in)

The TypeScript README's numbers come from a Hetzner cloud server (AMD EPYC-Genoa, 4 vCPUs, Ubuntu 24.04). Run this there from a fresh clone of both repos, side by side, and paste both outputs here with `uname -a`, `python3 --version`, `node --version` and the two `git rev-parse --short HEAD` hashes:

```
cd pycanonic
uv venv -p 3.14 .venv
uv pip install -p .venv/bin/python maturin pytest -r bench/requirements.txt
.venv/bin/maturin develop --release
.venv/bin/pytest bench/test_interpreters.py -q
.venv/bin/python bench/interpreters.py --gate

uv venv -p .venv/bin/python .venv-alt
uv pip install -p .venv-alt/bin/python maturin pytest -r bench/requirements-alt.txt
VIRTUAL_ENV="$PWD/.venv-alt" .venv-alt/bin/maturin develop --release
.venv-alt/bin/python bench/interpreters.py --gate

cd ../canonic && pnpm install && pnpm bench:interp
```

| implementation | bytes in | vs pycanonic | dict in | vs pycanonic |
|---|---:|---:|---:|---:|
| (not yet run) | | | | |

## Hot path

Measured on an Apple M3 Max with Python 3.14.7 and a release build (`maturin develop --release`), on a machine that was not idle. Two payloads: the 64 bench bodies in `crates/canonic-core/tests/fixtures/bench_bodies.json` (6,189 bytes each, `specs/bench.json`, 18 fields), and the 64 bodies of `bench/interpreters.py` (8.7 KB each, 501 fields), which `scripts/export_wide.py` writes to `tests/fixtures/wide_bodies.json` for the Rust bench.

The Python rows are the median of `measure` (4 ms batches, 21 rounds, round-robin) over all 64 bodies: `scripts/bench_hot.py` for the 6 KB body, `bench/interpreters.py` for the 501-field one. "Start" is the code before task 24b (`f9b518e`) for the 6 KB rows and before task 26's fix (`1b52414`) for the 501-field rows. "Now" is `e425f48`.

| case | start | now | target | TypeScript here |
|---|---:|---:|---:|---:|
| 6 KB `map(bytes)` | 10.80 µs | 5.49 µs | under 5.5 µs | 15.3 µs (parse + map) |
| 6 KB `map(dict)` | 4.42 µs | 3.71 µs | 3.5 µs or less | 3.45 µs |
| 6 KB `map_json(bytes)` | 10.09 µs | 4.67 µs | under 4.5 µs | no such path |
| 6 KB `extract`, scanner | 4.25 µs | 1.31 µs | under 2.5 µs | 8.22 µs |
| 6 KB `extract`, anchored | 925 ns | 691 ns | under 500 ns | 1.22 µs |
| 6 KB `view(dict)` + 3 reads | 923 ns | 709 ns | under 750 ns | 740 ns |
| 6 KB `view(bytes)` + 3 reads | 7.46 µs | 3.16 µs | | |
| 501-field `map(bytes)` | 55.53 µs | 38.99 µs | under 22 µs | 9.2 µs (map) |
| 501-field `map(dict)` | 43.42 µs | 38.02 µs | under 25 µs | |
| 501-field `map_json(bytes)` | 49.96 µs | 30.95 µs | under 15 µs | |

## Rust only

`cargo bench -p canonic-core`. `scan` runs body 0 again and again. `hot` (6 KB) and `wide` (501 fields) cycle through all 64 bodies and report per body. The gap between them and the Python rows is the binding and the Python objects.

| divan case | start | now |
|---|---:|---:|
| `scan` parse with the bench filter | 5.87 µs | 2.1 to 2.3 µs |
| `scan` map_bytes | 9.37 µs | 4.2 to 4.7 µs |
| `hot` parse | 6.56 µs | 2.4 µs |
| `hot` plan execution into `Out` | 2.87 µs | 2.0 µs |
| `hot` map_json | 10.27 µs | 4.4 µs |
| `wide` parse | 17.4 µs (`1b52414`) | 11.8 µs |
| `wide` plan execution into `Out` | 20.8 µs (`7ce4d59`) | 16.1 µs |
| `wide` map_json | 41.6 µs (`1b52414`) | 29.7 µs |

The parser no longer builds the stage 1 index (`structurals`). It still has its bench and its tests, but it is off the hot path.

## Reproduce

```
.venv/bin/maturin develop --release
.venv/bin/python scripts/bench_hot.py
.venv/bin/python bench/interpreters.py
cargo bench -p canonic-core --bench hot
cargo bench -p canonic-core --bench wide
cargo bench -p canonic-core --bench scan
```
