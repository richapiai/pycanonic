# Excluded cases

Both runners skip every case named on a `- `case-name`: reason` line below.

## TypeScript bugs

Cases where the TypeScript reference is wrong, checked against the TypeScript source.

None.

## Documented divergences

Cases where pycanonic differs from TypeScript by design.

- `hostile/spec/21`: its two path keys are the lone surrogates U+D800 and U+DFFF. A UTF-8 string cannot hold either, so both become U+FFFD and pycanonic reads them as one key, where TypeScript reads two.
