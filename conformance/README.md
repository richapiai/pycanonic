# Conformance corpus

`cases/` is a vendored copy of the TypeScript corpus (`canonic/conformance/cases`, documented in `canonic/conformance/README.md`). Each case holds a spec, its canonical schema, raw inputs and what the TypeScript artifact returns for them, or the exact compile error. Two runners check pycanonic against every case:

- `crates/canonic-core/tests/conformance.rs` (`cargo test`): `map_bytes` written with `write_json`, `map_json`, `extract` and the stats snapshot.
- `python/tests/test_conformance.py` (`pytest python/tests`): `map(bytes)`, `map(parsed)` and `map_json`, one test per case.

`SOURCE_COMMIT` holds three lines: the TypeScript commit the copy came from, that commit's date, and the date of the sync. CI fails once the sync date is more than 30 days old. To refresh, with `canonic` checked out next to this repo:

```sh
conformance/sync.sh
```

## Comparison rule

Both runners apply the same rule (`compare` in `conformance.rs`, `compare.py` for Python):

- Numbers compare by value, so `1` equals `1.0`. Past 2^53, the rule allows one double step of difference (`|a - b| <= ulp(a)`). A JavaScript number does not hold more precision than that.
- Strings compare after lone surrogates become U+FFFD on both sides. The TypeScript JSON carries them as `\ud800` escapes, and a UTF-8 string cannot hold them.
- Objects compare key by key, in order. Key order is part of the contract.
- `extract` compares the routing keys. It is skipped on cases flagged `duplicate_keys`: there `map` keeps the last value of a repeated key (as `JSON.parse` does) and `extract` keeps the first.
- When an input's expected `map` is `null`, TypeScript did not parse it. The runner checks that mapping the bytes gives the all-null canonical shape, the same nested objects with every leaf `null`. TypeScript never maps such a body, so the runners map it after the stats check.
- Stats compare `responses`, plus `fill`, `errors` and `total` for every field, exactly.
- For an `error` case, compiling must fail with exactly that message: `CompileError` in Rust, `SpecError` in Python.

## Exclusions

`EXCLUDED.md` has two sections, one line per case with the reason. "TypeScript bugs" lists cases where the TypeScript reference is wrong, checked against the TypeScript source. "Documented divergences" lists cases where pycanonic differs by design, such as two lone-surrogate keys that both become U+FFFD. Both runners skip every name in either section.
