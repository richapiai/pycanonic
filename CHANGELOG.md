# Changelog

## 0.1.0

The first release. pycanonic is the Python version of the TypeScript canonic, with a Rust core. It reads the same spec files and gives the same output on the 145 cases of the shared conformance corpus, with one documented divergence.

- `compile_spec` and `load` compile a spec into a plan, or load a plan that `canonic build` wrote.
- `Artifact.map` takes `bytes`, `str` or a parsed object. `map_json` takes the raw body and returns canonical JSON bytes with no Python objects built in between.
- `Artifact.view` maps each field on first read. `extract` and `stream` read the routing fields from raw text, with anchors.
- `pycanonic.stream.routing_tap` reads the routing fields from an async body while it passes through.
- `Registry`, per-field `stats`, `FillMonitor`, `shape`, `fingerprint`, `ShadowDiff` and `merge_gate` for drift and the learning loop.
- Nine default transforms and seven routing validators. Custom transforms are plain Python functions.
- The `canonic` CLI: `infer` (with `--ask`), `test`, `diff`, `build` and `bench`.
- abi3 wheels for CPython 3.10 and later on manylinux x86_64 and aarch64, musllinux x86_64, macOS universal2 and Windows x86_64.
- The `canonic-core` Rust crate on crates.io.

On one Apple M3 Max, `map(bytes)` on the 5.9 KB benchmark body takes 5.64 µs, against 15.25 µs for `JSON.parse` plus `map` in TypeScript. [docs/known-limits.md](docs/known-limits.md) lists every divergence from the TypeScript package.
