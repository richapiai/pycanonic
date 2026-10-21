# Security

Report a vulnerability privately through GitHub. Open the repository's Security tab and choose "Report a vulnerability". Please do not open a public issue for it. You get a reply within 5 working days.

Fixes land in the latest minor release only.

## What pycanonic trusts

Specs are trusted input. They live in your repository and go through review like code. Even so, a spec never becomes code. `compile_spec` turns it into a plan, a data structure that the Rust executor walks. A path that does not match the path grammar fails the compile. The conformance corpus runs the hostile specs and payloads from the TypeScript package through both runners. They hold quotes, backticks, newlines, lone surrogates and `__proto__`. A spec that makes pycanonic read or write outside its own output is a vulnerability. Please report it.

Response bodies are untrusted. The parser caps nesting at 512 levels and never panics on malformed input. `map` and the view read only the keys a spec names, and transforms return null on anything they cannot handle. The fuzz targets in `crates/canonic-core/fuzz/` run the parser, `map_bytes`, `extract` and `stream` on generated input every night. The only `unsafe` code is the SIMD block scan in `crates/canonic-core/src/json/simd.rs`.

The extractor (`extract`, `stream`, `routing_tap`) does not check that its input is JSON. On malformed input it can return a wrong value without an error. Map the full body before you act on a value, and treat the extractor as a fast path that you check.

Custom transforms and the `--transforms` module of the CLI are your own Python code, and pycanonic runs them as given. `canonic infer --ask` runs its argument through the shell. Pass only a command you trust.
