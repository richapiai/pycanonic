# Contributing to pycanonic

This file covers how to set up the repo, the commands CI runs, how to add a transform, and how a release is cut. [docs/how-it-works.md](docs/how-it-works.md) walks through the compiler, the parser and the executor. By taking part you agree to the [code of conduct](CODE_OF_CONDUCT.md).

## Setup

You need Rust stable, Python 3.10 or later, and [uv](https://docs.astral.sh/uv/) or plain `venv` and `pip`.

```sh
git clone https://github.com/richapiai/pycanonic.git
cd pycanonic
python3 -m venv .venv
.venv/bin/pip install maturin pytest mypy
.venv/bin/maturin develop
.venv/bin/pytest python/tests -q
```

The repo has two crates and one Python package:

- `crates/canonic-core` is the Rust core: spec grammar, resolver, plan, executor, parser, extractor, transforms and drift. It has no Python in it, and it is published to crates.io.
- `crates/pycanonic-ext` is the PyO3 extension, `pycanonic._core`. It builds as one abi3 wheel per platform.
- `python/pycanonic` is the Python API, the registry, `routing_tap` and the `canonic` CLI.

## Commands

| Command | What it does |
|---|---|
| `cargo test --workspace` | the Rust unit, property and conformance tests |
| `cargo clippy --workspace --all-targets -- -D warnings` | lints, with every warning an error |
| `cargo fmt --check` | formatting |
| `.venv/bin/maturin develop` | builds the extension into the venv. Add `--release` before you time anything. |
| `.venv/bin/pytest python/tests bench/test_payloads.py -q` | the Python tests, the conformance corpus and the bench payload pins |
| `cargo +nightly fuzz run <target>` | in `crates/canonic-core`, runs one fuzz target: `extract`, `stream` or `map_bytes` |
| `cargo audit` | checks `Cargo.lock` against the RustSec advisory database |
| `scripts/leak-scan.sh` | fails on any tracked or untracked file with a word from `scripts/forbidden-words.txt` |
| `scripts/leak-scan.sh --history` | the same check over every commit message and diff on the current branch |
| `conformance/sync.sh` | refreshes the conformance corpus from a `canonic` checkout next to this repo |
| `.venv/bin/python bench/run.py` | the benchmark table. `--gate`, `--selftest` and `--vs-ts` are in [bench/README.md](bench/README.md). |
| `.venv/bin/python bench/interpreters.py` | pycanonic against the Python mapping libraries |

CI runs the Rust job on Linux, macOS and Windows, and the Python job on 3.10, 3.12 and 3.14. It also runs the leak scan and the corpus age check. The bench job is not blocking, because timing on shared runners is noisy. The nightly schedule in `release.yml` runs `cargo audit` and each fuzz target for 10 minutes.

## Adding a transform

pycanonic follows the TypeScript package. A new default transform lands in canonic first. Then it comes here through the conformance corpus.

1. Port the function into `crates/canonic-core/src/transforms.rs`. It returns null for any input it cannot handle and never panics.
2. Add its name to `Builtin` and to `default_transform_names`. An argument check goes next to the existing ones, so a bad argument fails the compile.
3. Add its name to `_DEFAULT_NAMES` in `python/pycanonic/_api.py`.
4. Add cases to the transform tests in `crates/canonic-core/tests/` and `python/tests/test_transforms.py`, with the inputs it must reject.
5. Run `conformance/sync.sh` to pull the TypeScript cases for it, then the full test suite.

A transform that only one deployment needs belongs in that deployment. Pass it through `compile_spec(spec, schema, transforms=...)`. See [docs/custom-transforms.md](docs/custom-transforms.md).

## Pull request checklist

- CI is green: Rust tests on three platforms, clippy, fmt, Python tests, leak scan, corpus age.
- New behavior has a test that fails without the change.
- The only `unsafe` code lives in `crates/canonic-core/src/json/simd.rs`.
- `map`, `map_json`, `extract` and the view never raise on bad data.
- A divergence from the TypeScript package goes into `conformance/EXCLUDED.md` and [docs/known-limits.md](docs/known-limits.md) in the same PR.
- No interface with one implementation, no configuration for a value that never changes, no abstraction for a later PR.
- A performance number changes in the README only with the script that produced it, the machine and the commit.
- Prose in the README and `docs/` follows the same rules as canonic. The first sentence says what changed, and every speed claim has a number and a script. No filler words, no emoji.
- Never commit a real provider's payload or response shape. The example specs use fictional providers.

## Releasing

A release is a `v*` tag. `.github/workflows/release.yml` does the rest:

1. It checks that the tag, both crate versions and `__version__` name the same version.
2. It builds abi3 wheels for manylinux x86_64 and aarch64, musllinux x86_64, macOS universal2 and Windows x86_64, plus an sdist.
3. It installs each wheel in a clean venv on Python 3.10 and 3.14, runs the README quick start and `canonic --help`.
4. It runs `cargo audit`.
5. It creates a GitHub release for the tag, with the wheels and the sdist attached, and the `CHANGELOG.md` section as notes.
6. If the repository variable `PUBLISH_PUBLIC` is `true`, it publishes to PyPI with trusted publishing, then runs `cargo publish -p canonic-core`. Without the variable, nothing leaves GitHub.

Before the first public release, set up these once:

- On PyPI, add a trusted publisher for this repository, workflow `release.yml`, environment `pypi`.
- In the repository options, create the environments `pypi` and `crates-io`. Add a `CARGO_REGISTRY_TOKEN` secret to `crates-io`.

To cut a release:

1. Set the version in `crates/canonic-core/Cargo.toml`, `crates/pycanonic-ext/Cargo.toml` and `python/pycanonic/__init__.py`, and add the entry to `CHANGELOG.md`.
2. On the Hetzner bench box, run the blocking bench gate and keep the output:

   ```sh
   .venv/bin/maturin develop --release
   .venv/bin/python bench/run.py --gate
   .venv/bin/python bench/run.py --vs-ts ../canonic
   .venv/bin/python bench/interpreters.py --gate
   ```

   `interpreters.py --gate` skips the three libraries in `bench/interpreters-exempt.json` (jmespath, jsonbender and pydantic) and fails on any other mapping library under 10x. The README says why the three are exempt.

3. Run the local checks: `cargo test --workspace`, `.venv/bin/pytest python/tests -q`, `cargo audit`, `scripts/leak-scan.sh` and `scripts/leak-scan.sh --history`.
4. Tag and push:

   ```sh
   git tag -a v0.1.0 -m "pycanonic 0.1.0"
   git push origin main
   git push origin v0.1.0
   ```

5. Watch the `release` workflow. The GitHub release, PyPI and crates.io jobs wait for the wheel smoke test and the audit.

To publish by hand instead, for example after a workflow failure past the wheel stage:

```sh
# the wheels and the sdist from the workflow run, in dist/
gh run download <run-id> --dir dist --pattern 'wheel-*' --pattern sdist
python3 -m pip install twine
python3 -m twine upload dist/*/*.whl dist/sdist/*.tar.gz
cargo publish -p canonic-core
```
