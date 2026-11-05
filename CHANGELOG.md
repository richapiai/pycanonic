# Changelog

## 0.2.0

### Added

- Language 2 conditions in `when`, `filter` and `reject` gain `matches`, `starts_with`, `ends_with`, `contains`, the compares `lt`, `lte`, `gt` and `gte`, and the combinators `not`, `all` and `any`. A language 1 spec that uses one fails the grammar check as before.
- Language 2 gets `len`, the number transforms `add`, `sub`, `mul`, `div`, `ceil_div`, `floor_div`, `min` and `max`, and the compares `eq`, `lt`, `lte`, `gt` and `gte`. On one source they take a number, as in `{"mul": 100}`. On a `from` list they take no argument and fold the sources left to right. A language 1 spec that names one fails with `unknown transform` as before.
- Language 2 gets computed sources. An entry of a `from` list can be `{"from": <path or list>, "transform": <transform>}`, nested to any depth, so `(page + 1) * size >= total` is one field. A computed source that gives null counts as a missing source for the outer transform. A plan that uses one is format 2.
- Language 2 gets `format`, which builds a string from a template: `{"format": "urn:example:item:{}"}` on one source, or `{0}/{1}` on a `from` list. It is the inverse of `strip_prefix`. A null, missing or container source gives null. A template with no placeholder, `{}` mixed with `{n}`, an unclosed brace, `{}` on a list or an index past the last source fails the compile. A plan that uses it is format 2.
- Language 2 gets `url_part`, which reads `host`, `path`, `{"path_segment": n}` or `{"query": "name"}` from a `scheme://` URL, with percent-decoding and no new dependency. A value without `://` or with a bad escape gives null. `split_name` takes `{"part": "first" | "last", "mode": "last_word" | "first_word"}` in language 2: `first_word` gives the first word as the first name and the other words, joined by single spaces, as the last name. The string form is unchanged. A plan that uses either is format 2.
- Language 2 gets a conditional `coalesce` entry, `{"from": <path>, "when": <condition>}`, which is taken when its value is present and the condition holds, and `coalesce` on array fields in place of `from`. The first entry that is present, not null and not `[]` is the source, and every array option acts on it. A plan that uses either is format 2. A routing field cannot have a conditional entry.
- Language 2 `group_by` takes a list of 2 or more paths, such as `["org.id", "org.url", "org.name"]`. Each element uses the first path whose value is present and not null, and an element with no key is a group of its own. A single string keeps its form. A plan that uses a list is format 2.
- Language 2 gets `"on_missing": "transform"` on a scalar rule with one `from` path or on a container rule. It calls the first transform with `None` when the source is missing or null, so a custom function can fill a value. A null result is a miss, not an error, and `default` still applies. It needs one `from` path, and a routing field cannot have it. A plan that uses it is format 2.

## 0.1.1

### Changed

- `canonic fit` no longer proposes a rule that memorizes one sample pair. A lookup table keeps an entry only if two sample pairs show it. A learned text argument (a prefix, a suffix, a pattern) must change the value in two sample pairs. A reject condition must hit only bodies that lack at least half of the fitted sources, and conditions that reject the same pairs collapse to one. On the 13 invented providers of `python/tests/fit_eval/`, three learned tables that memorized the samples are now manual. The auto share stays at 75%, with no wrong auto field.
- `canonic schema`, `fit` and `verify` are marked experimental in the README and in `docs/fit.md`.
- `docs/known-limits.md` lists three more limits: aggregates take no transform, `group_by` reads one path, and the cost of a custom transform.

## 0.1.0

The first release. pycanonic is the Python version of the TypeScript canonic, with a Rust core. It reads the same spec files and gives the same output on the 145 cases of the shared conformance corpus, with one documented divergence. It also adds spec language 2 and three commands that onboard a provider from paired samples.

### Language 1, shared with TypeScript

- `compile_spec` and `load` compile a spec into a plan, or load a plan that `canonic build` wrote.
- `Artifact.map` takes `bytes`, `str` or a parsed object. `map_json` takes the raw body and returns canonical JSON bytes with no Python objects built in between.
- `Artifact.view` maps each field on first read. `extract` and `stream` read the routing fields from raw text, with anchors.
- `pycanonic.stream.routing_tap` reads the routing fields from an async body while it passes through.
- `Registry`, per-field `stats`, `FillMonitor`, `shape`, `fingerprint`, `ShadowDiff` and `merge_gate` for drift and the learning loop.
- Nine default transforms and seven routing validators. Custom transforms are plain Python functions.
- The `canonic` CLI: `infer` (with `--ask`), `test`, `diff`, `build` and `bench`.

### Language 2, pycanonic only

- Spec language 2, turned on with `"language": 2`. It adds constants, empty values for arrays and optional objects. It adds the list options `wrap`, `filter`, `unique`, `sort_by` and `limit`, and flatten paths. It also adds `group_by` with aggregates, `$input` with `when` and `omit`, `reject`, `definitions` and `"keys": "case_insensitive"`. See [docs/spec-format.md](docs/spec-format.md#language-2).
- `map` and `map_json` take `context=`, the request inputs that `$input` paths read.
- `pycanonic.Rejected`. If a `reject` condition holds, `map` and `map_json` raise it. Its `reason` is the condition as JSON text. `stats.snapshot()` counts rejections under `"rejected"`.
- Thirteen new transforms. The string transforms are `regex`, `strip_prefix`, `strip_suffix`, `replace`, `lookup`, `null_if`, `upper`, `title`, `split` and `to_string`. The others are `format_date`, `parse_number` and `parse_range`. `parse_date` takes `{"formats": [...]}`.
- A custom transform on an array or object field can return a whole list or dict. The result is checked against the canonical node.
- Language 2 canonical schemas accept any key name, `x-open` maps, arrays of arrays, `x-empty` and an array root. A spec maps an array root with the field key `"[]"`, and `map` returns a list.
- Plan format 2 for plans that use a language 2 rule. A plan with no language 2 rule is written as format 1.

Language 2 is pycanonic-only, and `view()` supports language 1 only. [docs/known-limits.md](docs/known-limits.md) lists these limits and the rest.

### Onboarding a provider

- `canonic schema` derives a canonical schema from the default provider's samples.
- `canonic fit` learns a language 2 spec from paired samples and sorts every field into auto, review or manual. On the 13 invented providers of `python/tests/fit_eval/`, 75% of the fields are auto and no auto field is wrong. The 80% target is not met yet. See [docs/fit.md](docs/fit.md).
- `canonic verify` maps every pair through a spec. It exits 1 on a shape failure, a field below `--min-match`, a reject that throws away a correct answer, or too many rejects. It lists the fields that the spec does not map, and `--fail-unmapped` fails on them.
- `canonic test` fixtures take `"context"` and `"expected": {"$rejected": ...}`.

### Packages and benchmarks

- abi3 wheels for CPython 3.10 and later on manylinux x86_64 and aarch64, musllinux x86_64, macOS universal2 and Windows x86_64.
- The `canonic-core` Rust crate on crates.io.
- On the Hetzner server behind the TypeScript numbers, `map(bytes)` on the 5.9 KB benchmark body takes 8.94 µs. TypeScript needs 23.07 µs for `JSON.parse` plus `map`. `view, 3 reads` measures 1.08x TypeScript there, and it is reported, not gated.
- `bench/run.py` has a row for a grouped language 2 spec on bytes. It is gated on darwin-arm64 only, until the Linux bench box records its baseline.
