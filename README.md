# pycanonic

[![CI](https://github.com/richapiai/pycanonic/actions/workflows/ci.yml/badge.svg)](https://github.com/richapiai/pycanonic/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/pycanonic)](https://pypi.org/project/pycanonic/)
[![license](https://img.shields.io/badge/license-MIT-blue)](https://github.com/richapiai/pycanonic/blob/main/LICENSE)

pycanonic maps a data provider's JSON response to your own canonical shape. It compiles a JSON mapping spec into a plan and runs it in Rust. The mapping stays a JSON file that you review in a pull request. pycanonic is the Python version of [canonic](https://github.com/richapiai/canonic), the TypeScript package, and it reads the same spec files.

On the 5.9 KB benchmark body with 18 spec fields, `artifact.map(bytes)` takes 8.91 µs on the server that produced the TypeScript numbers. That is the parse and the mapping together. The TypeScript canonic needs 23.26 µs for `JSON.parse` plus `map` on the same server, and `orjson.loads` of the same body alone takes 20.73 µs.

## Why

A product that gets the same kind of data from several providers ends up with one block of glue code per provider. The data can be people, companies, prices or shipments. Each block turns that provider's response into the product's own shape. That code is tedious to write and easy to get subtly wrong. It also breaks without an error the day a provider renames a key. pycanonic turns it into data you can review, test and monitor.

- A provider is a JSON file. `canonic infer` drafts the spec from 5 to 20 sample responses and scores every field. A person reviews a diff. A second provider for an endpoint adds a file, not a code path.
- The mapper is faster than glue code. On bytes, `map` beats `orjson.loads` plus hand-written Python by 3.2 times. It is 17 to 142 times faster than six of the eight interpreted mapping libraries we measured. The numbers and the exceptions are below.
- Fill and error counts per field show a renamed key. Every `map` counts them, fields inside list items included. `FillMonitor` raises an alert for a field whose fill drops. `canonic diff` proposes the fixed spec. `merge_gate` decides whether it can ship without a person.
- A routing value that fails its format check becomes `None`. A routing field (email, phone, domain, URL) carries a validator. A provider that answers `"N/A"` gives you `None` and a counted error. The check is on format only, so it cannot tell a personal address from a work one.
- One contract, any number of providers. Callers get your canonical keys, in a fixed order, whichever provider answered.
- Bytes in, bytes out. `map_json` takes the raw body and returns canonical JSON without building a Python object. It runs with the GIL (the CPython global interpreter lock) released.
- Specs live in git. A bad spec fails at compile time, with the field named. `artifact.source` lists the compiled plan, one line per field.

pycanonic is the wrong tool for one provider whose shape you control. It is also the wrong tool for queries or aggregation over JSON (use jq or JSONata). And it does not reject invalid input (use a schema validator). pycanonic maps what it can and counts the rest.

## Install

```sh
pip install pycanonic
```

This installs the library and the `canonic` command. Wheels are abi3 builds, so one wheel per platform covers CPython 3.10 and later. They cover manylinux x86_64 and aarch64, musllinux x86_64, macOS universal2 and Windows x86_64. The package has no runtime dependencies.

## Quick start

A canonical schema is JSON Schema with three extra keywords. `semantic` names the kind of value. `routing: true` marks a field the extractor returns. `aliases` gives inference more names to match. A spec maps one provider's shape onto the schema.

```python
import pycanonic

canonical = {
    "type": "object",
    "properties": {
        "full_name": {"type": "string"},
        "email": {"type": "string", "semantic": "email", "routing": True},
        "company": {
            "type": "object",
            "properties": {
                "domain": {"type": "string", "semantic": "domain", "routing": True},
                "headcount": {"type": "integer"},
            },
        },
    },
}

spec = {
    "provider": "acme",
    "endpoint": "person.enrich",
    "version": 1,
    "status": "promoted",
    "fields": {
        "full_name": "profile.fullName",
        "email": {"coalesce": ["contact.workEmail", "contact.emails[0].address"], "transform": ["trim", "lower"]},
        "company.domain": {"from": "org.website", "transform": "to_domain"},
        "company.headcount": {"from": "org.size", "transform": {"cast": "integer"}},
    },
}

acme = pycanonic.compile_spec(spec, canonical)

body = b'{"profile":{"fullName":"Ada Lovelace"},"contact":{"workEmail":" Ada@Acme.com "},"org":{"website":"https://www.acme.com/about","size":"120"}}'

print(acme.map(body))
# {'full_name': 'Ada Lovelace', 'email': 'ada@acme.com', 'company': {'domain': 'acme.com', 'headcount': 120}}

print(acme.map_json(body))
# b'{"full_name":"Ada Lovelace","email":"ada@acme.com","company":{"domain":"acme.com","headcount":120}}'

view = acme.view(body)
print(view.email, view.company.domain)  # ada@acme.com acme.com

print(acme.extract(body))  # {'email': 'ada@acme.com', 'company.domain': 'acme.com'}
print(acme.routing)        # ['email', 'company.domain']

print(acme.stats.snapshot())
# {'responses': 2, 'fields': {'full_name': {'fill': 2, 'errors': 0}, 'email': {'fill': 2, 'errors': 0}, ...}}
```

`map` takes `bytes`, `str` or an already parsed object. `compile_spec` rejects a bad spec with the field that caused it, so a typo fails at deploy time and not on a request:

```
acme person.enrich v1: fields["emial"]: canonical schema has no field "emial"
acme person.enrich v1: fields["email"]: unknown transform "shout"
```

`map` never raises on bad data. A missing source value becomes `None`. A transform that cannot handle its input returns `None` and adds one to that field's `errors` count. A routing field whose value fails its validator does the same. Only `map` and `map_json` count stats. The view and `extract` do not count. Call `snapshot()` and `reset()` once per period to get per-window counts.

The default transforms are `cast`, `lower`, `trim`, `split_name`, `join`, `parse_date`, `normalize_phone`, `normalize_linkedin` and `to_domain`. Pass your own with `compile_spec(spec, canonical, transforms={"my_transform": fn})`. pycanonic catches an `Exception` from your function and counts it as a field error. [docs/transforms.md](https://github.com/richapiai/pycanonic/blob/main/docs/transforms.md) covers the defaults. [docs/custom-transforms.md](https://github.com/richapiai/pycanonic/blob/main/docs/custom-transforms.md) covers your own. [docs/spec-format.md](https://github.com/richapiai/pycanonic/blob/main/docs/spec-format.md) has the spec rules.

A shadow artifact is a candidate version that you run next to the promoted one to compare their output. The registry holds one promoted and at most one shadow artifact per provider and endpoint:

```python
registry = pycanonic.Registry()
registry.put("acme", "person.enrich", 1, acme)
registry.get("acme", "person.enrich")         # the promoted artifact
registry.get_shadow("acme", "person.enrich")  # None until a shadow version is put
```

`put` refuses draft and deprecated artifacts. It also refuses an artifact whose provider, endpoint or version differ from its arguments. It swaps a reference, so a request that already holds the old artifact finishes on it.

## Benchmarks

All numbers on this page come from the Hetzner server that produced the TypeScript README numbers: AMD EPYC-Genoa, 4 vCPUs, 7.6 GB, Ubuntu 24.04.5, Python 3.14.7 and Node 26.10.0, release build. The load average was 0.6 at the start and about 1.2 to 1.5 during the run. Each number is the median of three runs of pycanonic 1f9d115 against canonic 936d6fd. That commit comes before spec language 2. Language 2 rules cost only the specs that use them, and `bench/run.py --gate` checks that the language 1 numbers stay within 15% of the baseline. [bench/README.md](https://github.com/richapiai/pycanonic/blob/main/bench/README.md) has the method, the scripts and the full output.

### Against TypeScript canonic

`bench/run.py --vs-ts` runs both packages on the same bodies in one session. It checks that the TypeScript checkout is at the pinned commit before it starts. "py / ts" is pycanonic time over TypeScript time, so below 1.00x is faster.

| Case | TypeScript | pycanonic | py / ts |
|---|---:|---:|---:|
| map, bytes in (TypeScript: `JSON.parse` + map) | 23.07 µs | 8.94 µs | 0.39x |
| map, JSON out (TypeScript: `JSON.parse` + map) | 23.07 µs | 7.42 µs | 0.32x |
| map, parsed in (reported, not gated) | 5.25 µs | 6.46 µs | 1.23x |
| view, construct (reported, not gated) | 11 ns | 143 ns | 13.0x |
| view, 3 reads (reported, not gated) | 1.08 µs | 1.17 µs | 1.08x |
| extract 3 routing fields, 5.9 KB | 10.41 µs | 2.16 µs | 0.21x |
| same, anchored | 1.84 µs | 863 ns | 0.47x |
| extract 5 fields, 10.7 KB | 19.52 µs | 3.31 µs | 0.17x |
| same, anchored | 4.50 µs | 1.35 µs | 0.30x |
| stream, 1 KB chunks | 23.79 µs | 10.04 µs | 0.42x |

TypeScript has no bytes-in path, so the bytes rows compare with `JSON.parse` plus `map`. Three rows are slower, and all three read an already parsed dict. On a parsed object, V8's JIT reads JavaScript objects faster than Rust reads Python dicts. `map, parsed in`, `view, construct` and `view, 3 reads` are reported, not gated. `view, 3 reads` measured 1.03x to 1.14x on this server in the three runs, 1.08x at the median. On an Apple M3 Max it measured 0.98x. A Python call that returns an extension object costs about 100 ns before Rust runs, which is most of the view construct time.

### The 6 KB rows against Python references

This table runs the same 5.9 KB bodies against what a Python user writes by hand. The hand-written reference calls the same Rust transforms as pycanonic, which makes it the harder bar. "pycanonic is" is pycanonic time over the reference time.

| Case | pycanonic | Reference | pycanonic is |
|---|---:|---|---:|
| map, bytes in | 8.94 µs | `orjson.loads` + hand-written, 28.60 µs | 0.31x |
| map, JSON out | 7.42 µs | `orjson.loads` + hand-written + `orjson.dumps`, 29.14 µs | 0.25x |
| map, parsed in | 6.46 µs | hand-written, 7.31 µs | 0.88x |
| view, 3 reads | 1.17 µs | hand-written routing reads, 1.26 µs | 0.93x |
| extract routing | 2.16 µs | `orjson.loads` + read, 22.28 µs | 0.10x |
| extract, anchored | 863 ns | `orjson.loads` + read, 22.22 µs | 0.04x |

### Against mapping libraries

`bench/interpreters.py` maps a 501-field nested payload with the same mapping in every library. It checks that all of them produce the same output before it times anything. "vs pycanonic" is the library's time over pycanonic's `map` time.

| Library | Bytes in | vs pycanonic | Dict in | vs pycanonic |
|---|---:|---:|---:|---:|
| pycanonic `map` | 61.53 µs | 1.0x | 55.23 µs | 1.0x |
| jsonnet 0.22.0 | 1.19 s | 19,316.2x | 1.19 s | 21,545.6x |
| json-e 4.8.4 | 11300.08 µs | 183.7x | 11171.51 µs | 202.3x |
| jsonata-python 0.7.1 | 7848.93 µs | 127.6x | 7751.55 µs | 140.4x |
| glom 25.12.0 | 6439.02 µs | 104.6x | 6330.87 µs | 114.6x |
| jq 1.13.0 (jq.py) | 4599.19 µs | 74.7x | 4562.62 µs | 82.6x |
| jmespath-community 1.1.3 | 1236.23 µs | 20.0x | 1196.28 µs | 21.6x |
| jsonpath-ng 1.12.0 | 1136.74 µs | 18.5x | 1106.02 µs | 20.0x |
| jsonata-rs 0.1.4 | 1052.52 µs | 17.1x | 1027.50 µs | 18.6x |
| jmespath 1.1.0, without transforms | 484.24 µs | 7.9x | 451.42 µs | 8.2x |
| jsonbender 0.9.3 | 434.88 µs | 7.1x | 387.66 µs | 7.0x |
| pydantic 2.14.1 | 175.85 µs | 2.9x | 129.23 µs | 2.3x |
| hand-written py (pure-Python transforms), reference | 114.74 µs | 1.9x | 77.24 µs | 1.4x |
| msgspec 0.22.0 Structs + hand-written py, reference | 43.53 µs | 0.7x | 37.90 µs | 0.7x |

Eight of the ten interpreted libraries are at least 10 times slower than pycanonic. Two are not. jmespath runs without transforms because it has no string functions, and lands at 7.9x. jsonbender lands at 7.1x. Both are light interpreters that spend under 1 µs per field. pydantic is the exception by design. It compiles its models into a Rust validator tree, so it does the same kind of work as pycanonic and is no interpreter. The jsonnet binding has no compiled form, so it parses the Jsonnet program again on every call. msgspec with hand-written glue is faster than pycanonic on this payload, at 0.7x. It is a reference row and not a mapper, because msgspec only decodes the body into typed Structs and hand-written code moves the fields. jmespath-community and jsonata-rs install the same modules as jmespath and jsonata-python, so their rows come from a second environment, against that environment's pycanonic run. [docs/why-not-jsonata.md](https://github.com/richapiai/pycanonic/blob/main/docs/why-not-jsonata.md) covers the method and where the time goes.

## CLI

`pip install pycanonic` installs the `canonic` command. Every subcommand has `--help`. Exit code 0 is success, and 1 is a verdict (`test` found a failing case, `diff` found drift). Exit code 2 means the command cannot run: a missing file, an unknown flag, a spec that does not compile. The commands match the TypeScript CLI and write the same output on the shared fixtures.

### canonic infer

This command drafts a spec from 5 to 20 sample responses. The spec goes to stdout (or `--out`) with status `draft`. A per-field report goes to stderr, with a score and a `<- check` mark on anything that needs a look.

```sh
canonic infer --schema canonical/person.enrich.json --provider globex --endpoint person.enrich samples/ > globex.json
```

```
email                       match    0.87  {"from":"e_mail","transform":["trim","lower"]}
full_name                   join     0.60  {"from":["fname","lname"],"transform":"join"}
experiences[]               item     0.43  {"from":"jobs","item":{...}}  <- check
seniority                   unmapped  best: fname (0.35)
anchor: email "e_mail":
```

`--known`, `--synonyms`, `--learn` and `--min-score` tune the scores. `--ask <command>` is an optional model pass. It sends each doubtful field to that shell command as a JSON question on stdin, and the command prints one source path or `null`. See [docs/inference.md](https://github.com/richapiai/pycanonic/blob/main/docs/inference.md).

### canonic test

This command maps fixtures through a spec and checks the fields that each fixture's `expected` names. It exits 1 for any failing case.

```sh
canonic test --schema canonical/person.enrich.json acme.json fixtures/acme.json
# 3 cases, 0 failed
```

### canonic diff

This command compares two sets of samples from one provider. With `--spec`, it also finds the spec fields whose fill dropped. It re-infers only those from the new samples and writes the next version with status `shadow`. Drift makes it exit 1.

```sh
canonic diff --schema canonical/person.enrich.json --spec globex.json --out globex.next.json old/ new/
```

### canonic build

This command compiles each spec to a plan next to it (`acme.json` becomes `acme.plan.json`). `pycanonic.load("acme.plan.json")` reads the plan without a new compile. `--transform <name>` declares a private transform that the plan gets at load time. `--transforms <module.py>` declares every public function in a file.

### canonic bench

This command times a spec on your own samples, single core, median of 15 rounds. It times `map` from parsed objects and from bytes, `map_json`, a view, and `extract`, next to `json.loads`.

```sh
canonic bench --schema canonical/person.enrich.json globex.json samples/
```

## Onboarding a provider

Adding a provider to an endpoint that already has a default provider takes one command and a review. Send the same requests to both providers and save the answers in two folders under the same file names. Then:

```sh
canonic schema samples/default/ --out canonical.json
canonic fit --default samples/default/ --provider samples/new/ --schema canonical.json \
  --provider-name globex --endpoint person.enrich --out globex.json --report report
canonic verify globex.json --schema canonical.json --default samples/default/ --provider samples/new/
```

`canonic schema` records the default's shape, key order and empty values. `canonic fit` learns a spec from the pairs and sorts every field into auto, review or manual. Each field in the report has its evidence and what can break on live traffic. `canonic verify` maps every pair and exits 1 on a shape failure or a field below the match bar, so CI can gate the provider. A reviewer locks the fields they checked with `--keep`, and `fit` refits the rest.

On 13 invented providers, 75% of the fields land in auto, and no auto field is wrong on fresh pairs. The target is 80%. The specs use spec language 2 (`"language": 2`), which only pycanonic runs. See [docs/fit.md](https://github.com/richapiai/pycanonic/blob/main/docs/fit.md) and the [Language 2](https://github.com/richapiai/pycanonic/blob/main/docs/spec-format.md#language-2) rules.

## Same behavior as TypeScript

`conformance/cases/` holds 145 cases exported from the TypeScript package. Each case has a spec, its schema, raw inputs, and what the TypeScript artifact returns for them, or the exact compile error. `cargo test` and `pytest python/tests` run every case through `map(bytes)`, `map(dict)`, `map_json`, `extract` and the stats. One case is excluded as a documented divergence: two keys that differ only in a lone surrogate. CI fails once the copy is more than 30 days old. See [conformance/README.md](conformance/README.md).

## Known limits

`extract` does not check that its input is JSON. On a duplicate key it returns the first occurrence, where `map` keeps the last. A body that does not parse maps to all `None` and counts every field as a miss, with no exception. Integers beyond 2^53 stay exact, where TypeScript rounds them. [docs/known-limits.md](https://github.com/richapiai/pycanonic/blob/main/docs/known-limits.md) has the full list, with every place pycanonic differs from TypeScript.

## What is in the box and what you bring

pycanonic ships these parts:

- `compile_spec`, `load`, `map`, `map_json`, `view`, `extract`, `stream` and `pycanonic.stream.routing_tap`
- `validate_spec` and `validate_canonical`
- `Registry` for promoted and shadow artifacts
- per-field `stats`, `FillMonitor`, `fingerprint` and `shape`
- `ShadowDiff` and `merge_gate`
- nine default transforms and seven routing validators, and thirteen more transforms for language 2
- the CLI: `infer`, `test`, `diff`, `build`, `bench`, `schema`, `fit` and `verify`

Your deployment wires them together:

- the canonical schemas and where specs live
- request routing, fallbacks between providers and caching
- the scheduler that snapshots stats and feeds `FillMonitor`
- response sampling for fingerprints and `canonic diff`
- running shadow artifacts next to promoted ones
- the bot that opens a pull request from `canonic diff` output, and merges it on a `merge_gate` pass
- the model command behind `infer --ask`, an optional part

## Docs

- [How it works](https://github.com/richapiai/pycanonic/blob/main/docs/how-it-works.md)
- [API reference](https://github.com/richapiai/pycanonic/blob/main/docs/api.md)
- [Spec format](https://github.com/richapiai/pycanonic/blob/main/docs/spec-format.md), language 2 included
- [Onboarding a provider](https://github.com/richapiai/pycanonic/blob/main/docs/fit.md)
- [Transforms](https://github.com/richapiai/pycanonic/blob/main/docs/transforms.md)
- [Custom transforms](https://github.com/richapiai/pycanonic/blob/main/docs/custom-transforms.md)
- [Streaming](https://github.com/richapiai/pycanonic/blob/main/docs/streaming.md)
- [Inference](https://github.com/richapiai/pycanonic/blob/main/docs/inference.md)
- [Drift and the learning loop](https://github.com/richapiai/pycanonic/blob/main/docs/drift-and-learning-loop.md)
- [Why not JSONata](https://github.com/richapiai/pycanonic/blob/main/docs/why-not-jsonata.md)
- [Known limits](https://github.com/richapiai/pycanonic/blob/main/docs/known-limits.md)

## Status

pycanonic is at 0.1. The spec format follows the TypeScript package, and the Python API can still change before 1.0. A plan from `canonic build` is tied to the version that built it. Rebuild your plans after an upgrade.

## Contributing

[CONTRIBUTING.md](https://github.com/richapiai/pycanonic/blob/main/CONTRIBUTING.md) covers setup, the test, fuzz and bench commands, and how a release is cut. The project follows a [code of conduct](https://github.com/richapiai/pycanonic/blob/main/CODE_OF_CONDUCT.md). Report a vulnerability through the repository's Security tab, as [SECURITY.md](https://github.com/richapiai/pycanonic/blob/main/SECURITY.md) describes.

## License

MIT, copyright RichAPI AI (TexAu OÜ). See [LICENSE](https://github.com/richapiai/pycanonic/blob/main/LICENSE).
