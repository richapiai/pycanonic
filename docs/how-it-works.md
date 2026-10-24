# How pycanonic works

The TypeScript canonic turns a spec into JavaScript source. pycanonic turns the same spec into a plan, which is a data structure that a Rust executor runs. Nothing reads the spec on a request. This page follows one spec through the compiler, the parser and the executor. It is for people who read or change `crates/`.

## The pipeline

```
spec ──> validate_spec ──> resolve ──> Plan ──> Program ──> Artifact
          (spec.rs)       (resolve.rs) (plan.rs) (exec.rs)   (artifact.rs, pycanonic-ext)
canonical schema ──> load_schema ──┘
                     (canonical.rs)
```

| Step | File | In | Out | Fails on |
|---|---|---|---|---|
| Grammar | `spec.rs` | the spec | a list of issues | a key, path or transform step the grammar rejects. All issues are reported together. |
| Schema | `canonical.rs` | the canonical JSON Schema | a tree of objects, arrays and leaves | a name that is not an identifier, an array of arrays, a routing field inside an array |
| Resolve | `resolve.rs` | spec and tree | a `Plan`: one node per canonical field, with its source paths, transform steps and stat slots | an unknown field or transform, a bad transform argument, a routing field with a default |
| Link | `artifact.rs` | the plan, your transforms | a `Program` with every key interned, and a parse filter | a transform name nobody registered |

`compile_spec` runs all four steps. `canonic build` stops after "resolve" and writes the plan as JSON (`artifact.plan_json()`). `pycanonic.load` reads that JSON and runs "link". Both paths build the same `Program`, so a loaded plan and a compiled spec behave the same.

## The plan

This spec:

```json
{
  "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted",
  "fields": {
    "email": { "from": "contact.workEmail", "transform": ["trim", "lower"] },
    "company.domain": { "from": "org.website", "transform": "to_domain" },
    "jobs[]": { "from": "history", "item": { "title": "role" } }
  }
}
```

against a schema where `email` and `company.domain` are routing fields gives this `artifact.source`:

```
email <- contact.workEmail | trim | lower | validate email
company.domain <- org.website | to_domain | validate domain
jobs[] <- history[]
jobs[].title <- role
```

`artifact.source` prints one line per canonical leaf, in schema order. Each line shows the source, the transform steps and the routing validator. A `coalesce` prints as `coalesce(a, b)`, a default as `?? value`, and a pluck as `tags[].label`. An unmapped field prints as `<- null`. The listing is for people. `plan_json()` is the form a program reads.

Reading the plan:

- Fixed key order. Keys come out in schema order, mapped or not. So every record from every provider has the same keys in the same order, and `map_json` writes the same text.
- Transforms are direct calls. A default transform is a Rust enum variant with its argument bound at compile time. So `{ "cast": "integer" }` dispatches once, with no lookup by name. A routing field's validator is one more step at the end of its pipeline.
- Counters count misses. A field that comes out empty adds one to its miss slot. A transform that rejects a value adds one to its error slot. A filled field writes nothing. Fill is the total minus the misses.
- Keys are interned. Linking turns every path segment into a key id. Reads from a parsed body compare ids, and reads from a Python dict use one interned Python string per key.
- Shared prefixes are read once. Many fields often read below the same object (`org.website`, `org.size`). The executor resolves the shared prefix once per `map` call and reuses it.

## The bytes path

`map(bytes)`, `map_json` and `view(bytes)` start with the parser in `json/mod.rs`.

1. At link time the plan's source paths become a `Filter`, a tree of the keys the plan reads.
2. The parser walks the body once. Inside an object it checks each key against the filter. A wanted key is parsed. An unwanted value is skipped and left as `Node::Skipped`, with no allocation.
3. To skip a container, `json/simd.rs` classifies the body 64 bytes at a time into bitmasks for quotes, backslashes and brackets. It uses NEON on aarch64, AVX2 or SSE2 on x86_64, and plain bytes elsewhere. A prefix-XOR over the quotes gives the in-string mask, and the scan counts brackets outside strings until the depth returns to zero.
4. Strings and numbers borrow from the input where they can. The parser decodes only the strings the plan reads.
5. The executor (`exec.rs`) walks the program over the parsed tree and writes into a `Sink`.

There are two sinks:

- `PySink` (`pycanonic-ext/src/pysink.rs`) builds Python dicts and lists for `map`. The parse runs with the GIL released, and the sink builds objects with the GIL held.
- `JsonSink` writes JSON text straight into a buffer for `map_json`. It builds no Python objects, so the whole call runs with the GIL released. The bytes equal what `JSON.stringify` writes for the same value in the TypeScript package.

This is the bytes-in, bytes-out path. A service that receives JSON and returns JSON can call `map_json` and never build a Python object for the body.

The parser is lenient where `json.loads` is strict, because `map` never raises. Content after the root value is ignored. Skipped subtrees are bracket-matched and not checked. A body that does not parse maps to the all-null canonical shape, see [known-limits.md](known-limits.md#an-unparseable-body-maps-to-nulls).

## The dict path

`map(dict)` reads the Python object in place through `PyDoc` (`pycanonic-ext/src/pydoc.rs`). The executor is the same. Only the document type differs. A dict member is read by an interned key. A list or tuple element is read by index. Values that a field passes through unchanged are the same Python objects, not copies.

## The view

`artifact.view(src)` stores `src` and nothing else. Each canonical field is mapped the first time you read it and then kept. So reading three fields costs three field computations, however large the schema is. A nested canonical object is another view. `view(bytes)` parses the body once, with the GIL released, and keeps the parsed tree. `to_dict()` maps the whole level and equals `map(src)` there.

## The extractor

`extract(raw)` returns the routing fields from the response text without building the parsed tree. It lives in `extract.rs`.

1. At link time the routing fields' source paths become a trie.
2. The scanner walks the text once. Inside an object it compares each key with the wanted keys at that trie node. It skips the value of an unwanted key: strings with `memchr`, containers with the SIMD skip. No token is copied.
3. The scanner slices out and decodes a wanted value. Once every path is resolved the scanner stops, so it never reads the bytes after the last routing field.
4. The extractor then runs each routing field's transforms on those values. That is why `extract` and `map` agree.

For a spec with `anchors`, the extractor first finds each anchor token (`"workEmail":`) with a byte search. Then it reads the value after the token. A missing token falls back to the scanner.

The same state machine runs incrementally for `artifact.stream()` and `routing_tap`. `push(chunk)` resumes where the last chunk ended, also in the middle of a string or a number.

The extractor trusts its input. The fuzz targets in `crates/canonic-core/fuzz/` check it against a full parse, whole and in chunks.

## Where things are

| File | What it holds |
|---|---|
| `canonic-core/src/spec.rs` | spec types, the path grammar, the hand-written validator that must agree with `spec.schema.json` |
| `canonic-core/src/canonical.rs` | canonical schema loader, `validate_canonical` |
| `canonic-core/src/resolve.rs` | spec and schema to plan, and every semantic error |
| `canonic-core/src/plan.rs` | the plan and its `source()` listing |
| `canonic-core/src/exec.rs` | the executor over any `Doc` into any `Sink` |
| `canonic-core/src/json/` | the pruned parser and the SIMD skip |
| `canonic-core/src/write.rs` | JSON output, byte for byte as `JSON.stringify` |
| `canonic-core/src/transforms.rs` | default transforms and routing validators |
| `canonic-core/src/extract.rs` | the extractor and its stream state |
| `canonic-core/src/drift.rs`, `shadow.rs` | `shape`, `fingerprint`, `FillMonitor`, `ShadowDiff`, `merge_gate` |
| `pycanonic-ext/src/` | the PyO3 bindings: `Artifact`, `View`, `PyDoc`, `PySink`, Python transforms |
| `python/pycanonic/` | the Python API, `Registry`, `routing_tap` and the CLI |

The inference engine behind `canonic infer` is in `python/pycanonic/cli/infer/`. [inference.md](inference.md) describes it.
