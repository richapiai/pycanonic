# Benchmarks

## The suite

`bench/run.py` is the port of canonic's `bench/run.js`. `bench/person.py` and `bench/routing.py` build the same bodies as `person.js` and `routing.js`, byte for byte. `bench/test_payloads.py` checks this against `bench/ts_outputs.json`, which `node bench/dump.mjs ../canonic` writes. Every case checks that canonic and its references give identical output before any timing starts.

How it measures (`measure`, shared with `canonic bench`): every function runs once to warm up and once more to find a batch size that takes at least 4 ms. Then 21 rounds run every function in the case in turn, starting from a different function each round, so drift hits all of them alike. The number is the median per-op time over the 21 batches. The loop is plain Python (`for i in ix: sink = fn(i)`), so each op includes a lambda call, an index and a list step. An empty lambda measures 18 ns this way, and one that indexes the input list measures 25 ns. canonic and every reference pay the same cost. The TypeScript bench uses the same loop shape with its own JIT costs. The tables below are one run, not the best of several.

### Against TypeScript canonic

Measured on an Apple M3 Max (36 GB, macOS 27.0) with Python 3.14.7 and Node 26.10.0, release build, in one session: pycanonic first, then canonic's `run.js` and `streaming.js`. The machine was not idle (load average about 6), so treat the absolute numbers as rough. Both sides ran under the same load.

```
.venv/bin/maturin develop --release
.venv/bin/pip install -r bench/requirements.txt
.venv/bin/python bench/run.py --vs-ts ../canonic
```

```
pycanonic 787e9c6 against canonic 936d6fd, same session

case                  TypeScript rows                         TypeScript   pycanonic   py / ts  gate
map, bytes in         map + JSON.parse (input)                  16.25 µs     5.81 µs     0.36x  PASS
map, JSON out         map + JSON.parse (input)                  16.25 µs     5.08 µs     0.31x  PASS
map, parsed in        map                                        3.62 µs     3.95 µs     1.09x  reported
view, construct       view, construct                               8 ns      111 ns    14.48x  reported
view, 3 reads         view, 3 reads                               775 ns      696 ns     0.90x  PASS
extract routing       extract routing                            8.34 µs     1.35 µs     0.16x  PASS
extract, anchored     extract, anchored                          1.27 µs      707 ns     0.56x  PASS
extract 5, 10.7 KB    extract 5, 10.7 KB                        15.97 µs     1.92 µs     0.12x  PASS
extract 5, anchored   extract 5, anchored                        3.99 µs     1.13 µs     0.28x  PASS
stream, 1 KB chunks   canonic stream                            18.48 µs     9.11 µs     0.49x  PASS

'py / ts' = pycanonic time / TypeScript time (below 1.00x is faster). Gated rows fail at 1.00x or above.
```

TypeScript has no bytes-in path, so `map, bytes in` and `map, JSON out` are compared with `JSON.parse` plus `map` on the parsed object. The stream row comes from `streaming.js` (the 5-field 10.7 KB bodies in 1 KB chunks). `map, parsed in` and `view, construct` are reported and not gated (design section 8.1). A Python call that returns an extension object costs about 100 ns before Rust runs.

The Python table from the same run:

```
Python 3.14.7 darwin-arm64, pycanonic 787e9c6, payloads from canonic 936d6fd
person.enrich: 5.9 KB payloads, 18 spec fields; extract 5: 10.7 KB, fields at 10% to 90% of the body

case                  implementation                                       per op      ops/sec   canonic is
map, bytes in         canonic                                             5.81 µs      172,061             
map, bytes in         orjson.loads + hand-written py                     19.29 µs       51,843        0.30x
map, bytes in         orjson.loads (input)                               14.05 µs       71,156        0.41x
map, bytes in         json.loads (input)                                 33.64 µs       29,726        0.17x
map, JSON out         canonic                                             5.08 µs      196,832             
map, JSON out         orjson.loads + hand-written py + orjson.dumps      19.54 µs       51,169        0.26x
map, parsed in        canonic                                             3.95 µs      252,847             
map, parsed in        hand-written py                                     5.27 µs      189,911        0.75x
view, construct       canonic                                              111 ns    9,049,278             
view, construct       dict literal                                          48 ns   20,655,764        2.28x
view, 3 reads         canonic                                              696 ns    1,437,183             
view, 3 reads         hand-written py routing                              923 ns    1,083,956        0.75x
extract routing       canonic                                             1.35 µs      742,175             
extract routing       orjson.loads + read                                14.51 µs       68,902        0.09x
extract routing       orjson.loads (input)                               13.77 µs       72,604        0.10x
extract, anchored     canonic                                              707 ns    1,415,424             
extract, anchored     orjson.loads + read                                14.37 µs       69,599        0.05x
extract 5, 10.7 KB    canonic                                             1.92 µs      521,797             
extract 5, 10.7 KB    orjson.loads + read                                26.75 µs       37,378        0.07x
extract 5, anchored   canonic                                             1.13 µs      886,524             
extract 5, anchored   orjson.loads + read                                27.43 µs       36,461        0.04x
stream, 1 KB chunks   canonic                                             9.11 µs      109,736             
stream, 1 KB chunks   join + orjson.loads + read                         27.84 µs       35,918        0.33x

'canonic is' = canonic time / reference time (below 1.00x is faster).
```

### Gates

- `python bench/run.py --gate` fails if any canonic/reference ratio is more than 15% worse than `bench/baseline.json` for this platform (`sys.platform-machine`). An unrecorded platform fails and prints the JSON to paste. `--update-baseline` records it. `--selftest` adds a 20% slowdown to canonic and checks that the gate trips. In CI this runs as a non-blocking job, because shared runners are noisy.
- `python bench/run.py --vs-ts <canonic checkout>` fails if any gated row is not faster than TypeScript. Run it on the bench box before a release.

`bench/streaming.py` (chunk sizes against whole-body `extract`) and `bench/views.py` (view against map from 20 to 2,000 fields) are reported and not gated.

## Hot path

Measured on an Apple M3 Max with Python 3.14.7 and a release build (`maturin develop --release`). Each body is one of the 64 bench bodies in `crates/canonic-core/tests/fixtures/bench_bodies.json`, 6,189 bytes each. The spec is `specs/bench.json`.

`scripts/bench_hot.py` reports the median of `measure` (4 ms batches, 21 rounds, round-robin) over all 64 bodies. "Start" is the code before the performance pass (`f9b518e`). "Now" is the code after it. Both builds ran in the same session, alternating, three times each, on a loaded machine. The table shows the middle run.

| case | start | now | target | TypeScript here |
|---|---:|---:|---:|---:|
| `map(bytes)` | 10.80 µs | 5.64 µs | under 5.5 µs | 16.1 µs (parse + map) |
| `map(dict)` | 4.42 µs | 3.84 µs | 3.5 µs or less | 3.53 µs |
| `map_json(bytes)` | 10.09 µs | 5.11 µs | under 4.5 µs | no such path |
| `extract`, scanner | 4.25 µs | 1.37 µs | under 2.5 µs | 8.35 µs |
| `extract`, anchored | 925 ns | 690 ns | under 500 ns | 1.25 µs |
| `view(dict)` + 3 reads | 923 ns | 708 ns | under 750 ns | 756 ns |
| `view(bytes)` + 3 reads | 7.46 µs | 3.30 µs | | |

On a quieter run before the pass, the start numbers came out about 4% lower: 10.43, 4.29, 9.79, 3.86 µs, 884 ns, 882 ns and 7.17 µs.

## Rust only

`cargo bench -p canonic-core`. `scan` runs body 0 again and again. `hot` cycles through all 64 bodies and reports per body. The gap between `hot` and the Python table is the cost of the binding.

| divan case | start | now |
|---|---:|---:|
| `scan` parse with the bench filter | 5.87 µs | 2.1 to 2.3 µs |
| `scan` map_bytes | 9.37 µs | 4.2 to 4.7 µs |
| `scan` extract scanner | 4.12 µs | 1.1 to 1.2 µs |
| `scan` extract anchored | 749 ns | 460 to 500 ns |
| `hot` parse | 6.56 µs | 2.4 to 2.5 µs |
| `hot` plan execution into `Out` | 2.87 µs | 2.1 µs |
| `hot` map_json | 10.27 µs | 4.8 µs |

The parser no longer builds the stage 1 index (`structurals`). It still has its bench and its tests, but it is off the hot path.

## Reproduce

```
.venv/bin/maturin develop --release
.venv/bin/python scripts/bench_hot.py
cargo bench -p canonic-core --bench hot
cargo bench -p canonic-core --bench scan
```
