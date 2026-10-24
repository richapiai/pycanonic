# Why not JSONata

pycanonic compiles each spec to a plan once and runs it in Rust. On a 501-field nested mapping it runs 128 times faster than jsonata-python and 105 times faster than glom. Eight of the ten interpreted mapping libraries in the comparison are at least 10 times slower than pycanonic. Two are not, and pydantic is a different kind of tool. The numbers below come from `bench/interpreters.py`, the port of the TypeScript `bench/interpreters.js`.

## The benchmark

Every library gets the same mapping and the same 64 generated payloads:

- 335 scalar fields in 11 nested groups (`identity`, `contact`, `company.funding`, and others),
- three arrays of objects mapped per element (4, 12 and 10 fields per item, with 5, 8 and 3 items per payload),
- one array of 20 plucked values (`tags[].label` into `skills`).

That comes to 501 leaves in the output. Three fields in four carry a transform (`toNumber`, `trim` then `lower`, or `toBool`), and the fourth is a plain copy. The payloads are the same bytes as the JavaScript `genPayload(1..64)`, and `bench/test_interpreters.py` checks the hash.

Before it times anything, the script checks that every library produces the same output as pycanonic on all 64 payloads. A library that does not match is excluded and not timed. JMESPath has no string functions for these transforms. So plain `jmespath` runs without them, which gives it less work than the others. `jmespath-community` has the functions and runs with them.

"Bytes in" is `orjson.loads` and then the library, which is what a Python user writes. pycanonic gets the bytes directly. "Dict in" maps an already parsed dict.

## Results

The Hetzner box (AMD EPYC-Genoa, 4 vCPUs, 7.6 GB, Ubuntu 24.04.5), Python 3.14.7, release build, pycanonic at `1f9d115`. Each number is the median of three runs. The jmespath-community and jsonata-rs rows come from the second environment, against its own pycanonic run (61.66 µs bytes in, 55.31 µs dict in).

| Library | Bytes in | vs pycanonic | Dict in | vs pycanonic |
|---|---:|---:|---:|---:|
| pycanonic `map` | 61.53 µs | 1.0x | 55.23 µs | 1.0x |
| pycanonic `map_json`, JSON out | 49.34 µs | 0.8x | | |
| jsonnet 0.22.0 | 1.19 s | 19,316.2x | 1.19 s | 21,545.6x |
| json-e 4.8.4 | 11300.08 µs | 183.7x | 11171.51 µs | 202.3x |
| glom 25.12.0 | 6439.02 µs | 104.6x | 6330.87 µs | 114.6x |
| jsonata-python 0.7.1 | 7848.93 µs | 127.6x | 7751.55 µs | 140.4x |
| jq 1.13.0 (jq.py) | 4599.19 µs | 74.7x | 4562.62 µs | 82.6x |
| jmespath-community 1.1.3 | 1236.23 µs | 20.0x | 1196.28 µs | 21.6x |
| jsonpath-ng 1.12.0 | 1136.74 µs | 18.5x | 1106.02 µs | 20.0x |
| jsonata-rs 0.1.4 | 1052.52 µs | 17.1x | 1027.50 µs | 18.6x |
| jmespath 1.1.0, without transforms | 484.24 µs | 7.9x | 451.42 µs | 8.2x |
| jsonbender 0.9.3 | 434.88 µs | 7.1x | 387.66 µs | 7.0x |
| pydantic 2.14.1 | 175.85 µs | 2.9x | 129.23 µs | 2.3x |
| hand-written Python, not a mapper | 114.74 µs | 1.9x | 77.24 µs | 1.4x |
| msgspec 0.22.0 Structs + hand-written py, not a mapper | 43.53 µs | 0.7x | 37.90 µs | 0.7x |
| `orjson.loads` alone, not a mapper | 27.51 µs | 0.4x | | |

"vs pycanonic" is the time divided by pycanonic's `map` time in the same column. Above 1.0x is slower.

The hand-written row is the same mapping as a nested dict literal of `.get` chains and list comprehensions. That is the way you write glue code by hand. It is generated from the rules only because 501 fields is too many to type. The msgspec row decodes the body into typed Structs, and generated glue of the same kind moves the fields. It is faster than pycanonic here (0.7x), but msgspec does not map.

## Where the gap comes from

The interpreters read their mapping description while they map: a JSONata expression tree, a glom spec, a JMESPath AST, a jsonbender object graph. That work repeats on every request, in Python. pycanonic does it once, in `compile_spec`, and then runs a plan in Rust.

- On bytes, the parser builds only what the plan reads and skips the rest with a SIMD bracket scan.
- Every path segment is an interned key, so a read is one lookup with no path parsing.
- Default transforms are Rust functions with their arguments bound at compile time.
- Stats count misses, not fills, so a field that maps normally writes nothing.

## The three that miss 10x

- pydantic compiles its models once into a Rust validator tree (pydantic-core). On a parsed dict it does the same kind of work as pycanonic: it walks a Python dict from Rust and builds Python objects. It is not an interpreter in the JSONata sense. So 10x over it is not a target.
- jsonbender (7.1x) and jmespath without transforms (7.9x) are light pure-Python interpreters. They spend under 1 µs per field. pycanonic spends about 110 to 123 ns per field, so both land at 7.0x to 8.2x.
- On the M3 Max laptop, pycanonic takes 39 µs on bytes: about 12 µs to parse, 16 µs to run the plan and 10 µs to build the Python objects. `orjson.loads` of the 7.3 KB output alone takes 19 µs, so object building has little left in it.

A plan compiled into per-field kernels is the next step to clear the last two. It is not in 0.1.0.

For context, the TypeScript `node bench/interpreters.js` on the M3 Max laptop gives canonic 9.2 µs and jsonata 2.2.2 1112.6 µs (121.1x). The TypeScript side also has one library under 10x: @munesoft/api-normalizer at 5.8x.

## Reproduce

```
.venv/bin/maturin develop --release
uv pip install -p .venv/bin/python -r bench/requirements.txt
.venv/bin/python bench/interpreters.py --gate
```

`jmespath` and `jmespath-community` install the same module, and so do `jsonata-python` and `jsonata-rs`. The second of each pair runs in a separate environment from `bench/requirements-alt.txt`. [bench/README.md](../bench/README.md) has the commands, the earlier run and the libraries that were tried and dropped.

The table comes from the Hetzner box that produced the TypeScript README numbers: the median of three runs at pycanonic 1f9d115 against canonic 936d6fd. The breakdown of the 39 µs and the TypeScript context come from the M3 Max laptop, because the server run did not measure them.
