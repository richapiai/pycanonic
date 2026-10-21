# Benchmarks

Measured on an Apple M3 Max with Python 3.14.7 and a release build (`maturin develop --release`). Each body is one of the 64 bench bodies in `crates/canonic-core/tests/fixtures/bench_bodies.json`, 6,189 bytes each. The spec is `specs/bench.json`.

## Hot path

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
