# Why not JSONata

pycanonic compiles each spec to a plan once and runs it in Rust. On a 501-field nested mapping it runs 143 times faster than jsonata-python and 122 times faster than glom. Six of the eight interpreted mapping libraries in the comparison are at least 10 times slower than pycanonic. Two are not, and pydantic is a different kind of tool. The numbers below come from `bench/interpreters.py`, the port of the TypeScript `bench/interpreters.js`.

## The benchmark

Every library gets the same mapping and the same 64 generated payloads:

- 335 scalar fields in 11 nested groups (`identity`, `contact`, `company.funding`, and others),
- three arrays of objects mapped per element (4, 12 and 10 fields per item, with 5, 8 and 3 items per payload),
- one array of 20 plucked values (`tags[].label` into `skills`).

That comes to 501 leaves in the output. Three fields in four carry a transform (`toNumber`, `trim` then `lower`, or `toBool`), and the fourth is a plain copy. The payloads are the same bytes as the JavaScript `genPayload(1..64)`, and `bench/test_interpreters.py` checks the hash.

Before it times anything, the script checks that every library produces the same output as pycanonic on all 64 payloads. A library that does not match is excluded and not timed. JMESPath has no string functions for these transforms. So plain `jmespath` runs without them, which gives it less work than the others. `jmespath-community` has the functions and runs with them.

"Bytes in" is `orjson.loads` and then the library, which is what a Python user writes. pycanonic gets the bytes directly. "Dict in" maps an already parsed dict.

## Results

Apple M3 Max (36 GB, macOS 27.0), Python 3.14.7, release build, pycanonic at `68197d1`. The machine was under load (load average 3.5 to 5). This is one run.

| Library | Bytes in | vs pycanonic | Dict in | vs pycanonic |
|---|---:|---:|---:|---:|
| pycanonic `map` | 38.99 µs | 1.0x | 38.02 µs | 1.0x |
| pycanonic `map_json`, JSON out | 30.95 µs | 0.8x | | |
| glom 25.12.0 | 4757.68 µs | 122.0x | 4705.15 µs | 123.7x |
| jsonata-python 0.7.1 | 5585.88 µs | 143.2x | 5579.45 µs | 146.7x |
| jq 1.13.0 (jq.py) | 2841.09 µs | 72.9x | 2818.44 µs | 74.1x |
| jmespath-community 1.1.3 | 881.77 µs | 22.7x | 850.34 µs | 22.3x |
| jsonpath-ng 1.12.0 | 806.43 µs | 20.7x | 783.02 µs | 20.6x |
| jsonata-rs 0.1.4 | 593.25 µs | 15.2x | 570.19 µs | 15.0x |
| jmespath 1.1.0, without transforms | 372.41 µs | 9.6x | 348.98 µs | 9.2x |
| jsonbender 0.9.3 | 313.35 µs | 8.0x | 285.34 µs | 7.5x |
| pydantic 2.14.1 | 118.23 µs | 3.0x | 85.73 µs | 2.3x |
| hand-written Python, not a mapper | 88.80 µs | 2.3x | 60.19 µs | 1.6x |
| `orjson.loads` alone, not a mapper | 20.68 µs | 0.5x | | |

"vs pycanonic" is the time divided by pycanonic's `map` time in the same column. Above 1.0x is slower.

The hand-written row is the same mapping as a nested dict literal of `.get` chains and list comprehensions. That is the way you write glue code by hand. It is generated from the rules only because 501 fields is too many to type.

## Where the gap comes from

The interpreters read their mapping description while they map: a JSONata expression tree, a glom spec, a JMESPath AST, a jsonbender object graph. That work repeats on every request, in Python. pycanonic does it once, in `compile_spec`, and then runs a plan in Rust.

- On bytes, the parser builds only what the plan reads and skips the rest with a SIMD bracket scan.
- Every path segment is an interned key, so a read is one lookup with no path parsing.
- Default transforms are Rust functions with their arguments bound at compile time.
- Stats count misses, not fills, so a field that maps normally writes nothing.

## The three that miss 10x

- pydantic compiles its models once into a Rust validator tree (pydantic-core). On a parsed dict it does the same kind of work as pycanonic: it walks a Python dict from Rust and builds Python objects. It is not an interpreter in the JSONata sense. So 10x over it is not a target.
- jsonbender (8.0x) and jmespath without transforms (9.6x) are light pure-Python interpreters. They spend under 1 µs per field. pycanonic spends about 76 ns per field, so both land just under 10x.
- On bytes, pycanonic takes 39 µs: about 12 µs to parse, 16 µs to run the plan and 10 µs to build the Python objects. `orjson.loads` of the 7.3 KB output alone takes 19 µs, so object building has little left in it.

A plan compiled into per-field kernels is the next step to clear the last two. It is not in 0.1.0.

For context, the TypeScript `node bench/interpreters.js` on the same machine in the same session gives canonic 9.2 µs and jsonata 2.2.2 1112.6 µs (121.1x). The TypeScript side also has one library under 10x: @munesoft/api-normalizer at 5.8x.

## Reproduce

```
.venv/bin/maturin develop --release
uv pip install -p .venv/bin/python -r bench/requirements.txt
.venv/bin/python bench/interpreters.py --gate
```

`jmespath` and `jmespath-community` install the same module, and so do `jsonata-python` and `jsonata-rs`. The second of each pair runs in a separate environment from `bench/requirements-alt.txt`. [bench/README.md](../bench/README.md) has the commands, the earlier run and the libraries that were tried and dropped.

These numbers come from one laptop under load. The Hetzner server run, on the machine behind the TypeScript numbers, is still to come.
