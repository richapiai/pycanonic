# Known limits

This page lists the places where pycanonic can give you a wrong or missing value without an error. It also lists every place where pycanonic and the TypeScript canonic differ. Each entry says what to do about it.

## The extractor trusts its input

`extract` and `stream` do not check that their input is JSON. Malformed or truncated input returns the routing fields read before the damage and `None` for the rest. It never raises. Use extracted values to route. Before you store, return or bill anything, map the full body with `map`. See [streaming.md](streaming.md#what-the-extractor-trusts).

## An unparseable body maps to nulls

`map(bytes)` and `map_json` never raise on a bad body. A body that is not JSON maps to the full canonical shape with every leaf `None`. Every field counts as a miss in `artifact.stats`, and nothing counts as an error. So a broken body looks like an empty one, except in the stats. If you need to tell them apart, parse the body yourself first.

## Duplicate keys

A JSON object can repeat a key. pycanonic reads such an object in three ways:

- A path read (`from`, `coalesce`, `pluck`) takes the last occurrence, as `JSON.parse` does.
- An object passed through as a whole keeps the key at its first position, with the last value.
- `extract` and `stream` take the first occurrence.

So on `{"e_mail":"first@a.com","e_mail":"last@a.com"}`, `extract` and `map` disagree. The TypeScript package behaves the same way. Running the shadow diff on providers you route with `extract` shows whether any of them send duplicates.

## Key order on passthrough

JavaScript puts integer-like keys (`"1"`, `"42"`) first in every object, in numeric order. pycanonic keeps the source order. The canonical fields always come out in schema order on both sides. The difference shows only inside an object that a field passes through unchanged, such as an array element with no `item` map.

## Integers beyond 2^53 stay exact

JavaScript numbers are doubles, so the TypeScript package rounds an integer above 2^53. pycanonic keeps it exact. `{"id": 9007199254740993}` maps to `9007199254740993` here and to `9007199254740992` in TypeScript. The conformance runners allow one double step of difference past 2^53 for this reason.

## Very long integers become floats

CPython refuses to convert an integer string longer than 4,300 digits. pycanonic returns such an integer as a `float` instead of raising. It is usually `inf`. For numbers that long, raise the limit with `sys.set_int_max_str_digits`.

## Lone surrogates become U+FFFD

A JSON string can hold an escape such as `\ud800` with no partner. A UTF-8 string cannot hold that, so pycanonic turns it into U+FFFD. This has three effects:

- Two keys that differ only in a lone surrogate become the same key. The conformance case `hostile/spec/21` is excluded for this reason, see `conformance/EXCLUDED.md`.
- `fingerprint` of a response with such a key differs from the TypeScript fingerprint, because `JSON.stringify` writes the escape.
- On `map(dict)`, a lone surrogate inside a container that a field passes through stays as it is. pycanonic returns the same Python object and does not rewrite it.

## Custom transforms

- For a spec that names a transform alone, the call is `f(value)`. For a spec that gives an argument, the call is `f(value, arg)`.
- A transform that raises an `Exception` subclass is caught. The field gets `None` or its default, and the error is counted. `KeyboardInterrupt` and `SystemExit` are not swallowed. They are raised from `map` once it returns.
- A result other than `str`, `int`, `float`, `bool` or `None` counts as a rejection. The TypeScript package keeps any value a transform returns.
- A dict or list value reaches a transform as a JSON copy, not the original object. The copy stops at 512 levels of nesting or 1,000,000 values. Past either cap the field is `None` and one error is counted.
- A transform that mutates the input dict during a `map` call can see stale reads. pycanonic reads a shared path prefix once per call, so later fields read the sub-dict as it was first seen.

## Limits of shape and fingerprint

On a Python object, `shape` and `fingerprint` raise `ValueError` past 512 levels of nesting, 1,000,000 containers or 64 MiB of path text. A cyclic object, or one that shares its children many times, hits these caps. On `bytes` input the response is parsed first, and the result is never cut.

## A view inside its own source

A view keeps its source alive. If you store a view inside the dict it reads, the two form a cycle. The garbage collector cannot free it, because the view does not take part in cycle collection. Do not store a view in its own source.

## The GIL

`map(bytes)` releases the GIL while it parses the body. It holds the GIL while it builds the Python output. `map_json`, `extract` and `stream` release the GIL for the whole call. A custom Python transform runs with the GIL. So two threads can parse in parallel, but they build output objects one at a time.

## Anchors depend on the samples

`canonic infer` proposes an anchor only for a key token that occurs exactly once in every sample. A provider that later sends the key twice breaks that. The anchored reader skips a token after an odd run of backslashes, so a token inside a JSON string does not match. A field whose token is not found falls back to the scanner.

The anchored reader does not check where a token sits. If the same key shows up at another level (`manager.e_mail` next to `e_mail`), it reads whichever comes first. That new path changes the response fingerprint. Re-run inference after a fingerprint change. A key repeated inside the same object leaves the fingerprint unchanged.

## Transforms are lossy

- `split_name` splits on spaces. `"Ada Lovelace Jr."` gives last name `"Jr."`, and multi-word surnames lose all but their last word.
- `normalize_phone` keeps digits and a leading `+`. It has no country rules and rejects extensions.
- `to_domain` rejects domains in Unicode form.

If your data needs more, register your own transform under a new name or over the default. See [custom-transforms.md](custom-transforms.md).

## A default skips transforms

A field's `default` is returned as written, without its transforms. Routing fields cannot have a default. `compile_spec` rejects one. A default skips the validator, so every response looks like a hit.

## Auto-merge can be fooled

`merge_gate` lets a non-routing field change on up to 1% of compared responses (`max_change`). A provider change that keeps 99% or more of a field's values the same and alters the rest passes. Fill and error counters keep their watch after promotion. Every promotion is a spec change you can revert.

## Sparse arrays are judged late

Fields inside `item` maps are counted per element, under the spec's field name (`"experiences[].title"`). `FillMonitor` judges one only in a window that holds at least `min_responses` elements of its array. A window with fewer is skipped for that field, not merged into the next. So an array that rarely has elements can lose a key for a long time before a window is big enough.

The response fingerprint includes the keys of array elements. A fingerprint check on sampled responses catches the rename sooner, and so does `canonic diff`. Arrays mapped with `pluck` have no per-element counts.

## No single-element array collapsing

pycanonic never turns a one-element array into its element, or a single object into an array. For anything other than an array, an array field is `None`. A scalar field without a transform that points at an array gets the whole array. Address elements by index (`emails[0].address`).

## Inference

- `canonic infer --ask` covers scalar fields only. Array fields, and fields mapped by `coalesce`, `split` or `join`, are never sent to the command.
- The `--ask` question lists at most 200 scalar source paths, the first 200 found in the samples. When the samples have more, the right path can be missing from the question.
- Inference reads at most 5,000 source paths across the samples. For single-valued fields it looks inside arrays at the first element only (`emails[0].address`, `tags[0]`).
- A field that is `None` in every sample cannot be found. Use 5 to 20 samples that differ.
- Accuracy is measured on six synthetic providers (`python/tests/eval/`), not on real provider responses. See [inference.md](inference.md#evaluation).

## CLI differences from the TypeScript command

- `--ask` keeps the TypeScript convention. The command runs through the shell once per field, with one JSON question on stdin, and its last non-empty stdout line is the answer. An empty `--ask ""` means no ask. A reply that is not valid UTF-8 is decoded with replacement characters, so it names no known path and the field is skipped.
- `--min-score` accepts what the core's `cast number` accepts. It rejects `"0x1A"`, which JavaScript reads as 26. It also rejects a blank value, which JavaScript reads as 0.
- An unknown flag prints `canonic: unrecognized arguments: --bogus` and exits 2. The TypeScript command prints different text with the same exit code. A missing file prints Python's own error text.
- `canonic build` writes a plan, `spec.plan.json`, that `pycanonic.load` reads. It does not write a JavaScript module.
