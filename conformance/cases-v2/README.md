# Conformance cases for spec language 2

This folder holds the cases for spec language 2. They are written in this repo. The TypeScript package does not have language 2, so this folder does not come from `sync.sh`. Each rule has its own folder, for example `language/`.

Two runners check every case:

- `crates/canonic-core/tests/conformance_v2.rs` (`cargo test`) maps each input with `map_bytes` and `write_json`, and with `map_json`.
- `python/tests/test_conformance_v2.py` (`pytest python/tests`) maps each input with `map(bytes)`, `map(parsed)` and `map_json`.

Both runners use the comparison rule in [../README.md](../README.md). If this folder has no cases, a runner fails. No case is skipped.

## Case format

One JSON file holds one case. A mapping case has this shape:

```json
{
  "spec": {},
  "schema": {},
  "inputs": [
    {"body": {}, "context": null, "expected": {}}
  ]
}
```

- `spec` is the mapping spec, and `schema` is its canonical schema.
- `body` is the response body as JSON. The runners write it as bytes for the bytes path and give it as it is to the parsed path.
- `raw` is for a body that is not valid JSON. It is a string, and it replaces `body`. The runners map it as bytes and with `map_json` only, because a parsed form does not exist. An input must have exactly one of `body` and `raw`, or the runners fail it.
- A body that is one JSON string at the top level also goes in `raw`, for example `"raw": "\"text\""`. On the parsed path, `map` reads a Python `str` as JSON text, not as a string value.
- `context` is the object that `$input` paths read, or `null`.
- `expected` is the output of `map`. The form `{"$rejected": "<reason>"}` means that `map` raises `Rejected` with that reason.

An error case has `spec`, `schema` and `error`. Compiling must fail with exactly that message: `CompileError` in Rust, `SpecError` in Python.

```json
{"spec": {}, "schema": {}, "error": "invalid spec: spec.language: must be 1 or 2"}
```

The runners give `context` to every way in. `Rejected` does not exist yet. Until it does, a runner fails an input with a `$rejected` expectation. The failure says that the feature is not implemented yet.
