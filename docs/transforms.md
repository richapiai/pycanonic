# Transforms

pycanonic ships nine default transforms and seven routing validators. They live in `crates/canonic-core/src/transforms.rs`, and Python sees them as `default_transforms` and `validators`. Each one is a port of the TypeScript function, and the conformance corpus checks them against it.

Every one returns null for input it cannot handle and never raises. A transform that gets a value and returns null sets the field to `None`, or to its `default`. It also adds one to that field's `errors` in `artifact.stats.snapshot()`. Property tests in `crates/canonic-core/tests/` run each transform on junk input and random values.

```python
from pycanonic import default_transforms as T, validators as V

T["cast"](" 4.5e3 ", "number")             # 4500
T["split_name"]("Ada King Lovelace", "last")  # "Lovelace"
V["email"]("N/A")                           # None
```

A call through these dicts goes into Rust by name. It costs about 100 ns, which is fine for tests and glue code. Inside a spec, the plan calls the Rust function directly.

## Default transforms

### cast

Argument, required: `"number"`, `"integer"`, `"boolean"` or `"string"`. Anything else fails the compile with `invalid argument for cast`.

| To | Accepts | Returns null for |
|---|---|---|
| `number` | finite numbers. Also strings that JavaScript `Number()` parses, with surrounding whitespace (`" 4.5e3 "` is 4500). | `""`, `"  "`, `"1,234"`, `NaN`, `Infinity`, booleans, hex, octal and binary literals (`"0x10"`, `" 0x10"`) |
| `integer` | what `number` accepts, with a whole number as the result (`"12"` is 12) | `"12.5"`, plus everything `number` rejects |
| `boolean` | booleans. `1` and `0`. Strings `true`/`yes`/`1` and `false`/`no`/`0`, any case, trimmed. | `2`, `"maybe"` |
| `string` | strings as they are. Finite numbers and booleans as JavaScript `String()` writes them. | objects, arrays, `NaN` |

A whole number comes back as an `int`, so `"12.0"` cast to `number` is `12`. `True` is a boolean, not the number 1. A hex, octal or binary literal is an id, never a number. This holds with surrounding whitespace too: `" 0x10"` returns null. JavaScript `Number()` reads both as 16, and both packages reject them. The conformance corpus pins the rule.

### lower

No argument. It lowercases a string (`"ÀDA"` becomes `"àda"`). Anything else returns null.

### trim

No argument. It trims whitespace from both ends of a string. Anything else returns null.

### split_name

Argument, required: `"first"` or `"last"`. It trims the string and splits on spaces.

| Input | `first` | `last` |
|---|---|---|
| `"  Ada King Lovelace "` | `"Ada"` | `"Lovelace"` |
| `"Ada"` | `"Ada"` | null |
| `"   "` | null | null |

Middle names are dropped. Suffixes and multi-word surnames come out wrong (`"Ada Lovelace Jr."` gives last name `"Jr."`). Register your own transform for data that has them.

### join

Argument, optional: a separator string, default `" "`. It is built for `from` with an array of paths.

- A list joins its non-empty strings and finite numbers and skips everything else. `["Ada", None, "", "Lovelace"]` gives `"Ada Lovelace"`, and `["a", 1, "b"]` with `"-"` gives `"a-1-b"`.
- A non-empty string is returned as it is.
- A list with nothing to join, an empty string, or any other type returns null.

### parse_date

No argument. The output is an ISO date string.

- Strings are trimmed and must be `YYYY`, `YYYY-MM` or `YYYY-MM-DD`. Anything after a `T` or a space at position 10 is dropped, so `"2021-03-01T23:30:00-05:00"` gives `"2021-03-01"`. You get back the precision you send: `"2021-03"` stays `"2021-03"`.
- The date must exist. `"2020-02-29"` passes. `"2021-02-29"` and `"2021-13"` return null.
- Other layouts return null: `"03/2021"`, `"2021-3-1"`, `"June 2014"`.
- Numbers are epoch timestamps and must be positive whole numbers. Below 10^11 they are seconds, otherwise milliseconds. The result is the UTC day: `1614556800` and `1614556800000` both give `"2021-03-01"`.
- `0`, negatives and fractions return null, because providers send `0` for "unknown".

### normalize_phone

No argument. It keeps digits and a leading `+`.

- It takes a string (trimmed) or a number.
- It removes the separators space, `-`, `(`, `)` and `.`. Any other character returns null, so `"+1 415 555 0100 ext 2"` fails.
- It needs 7 to 15 digits.
- `"+1 (415) 555-0100"` gives `"+14155550100"`. `"415.555.0100"` and `4155550100` give `"4155550100"`.

It does not know country codes and does not add one. For E.164 with country rules, inject a parser. See [custom-transforms.md](custom-transforms.md).

### normalize_linkedin

No argument. It returns `https://www.linkedin.com/<kind>/<slug>` for profile, company and school URLs.

- The host must be `linkedin.com` or a subdomain of it (`uk.linkedin.com`). `notlinkedin.com` fails.
- The path must start with `/in/`, `/company/` or `/school/` and have a slug. `/jobs/view/...` and `/in/` alone return null.
- The scheme, query string, fragment and trailing slash are dropped. The whole URL is lowercased, slug included: `"uk.linkedin.com/in/Ada?trk=x"` gives `"https://www.linkedin.com/in/ada"`.

### to_domain

No argument. It returns a bare lowercase domain.

- It takes a URL, a host or an email address. For an email (an `@` and no `/`), it takes the part after the last `@`.
- It strips the scheme, path, query, fragment, port, a leading `www.` and a trailing dot.
- The result must be a valid domain. That means at least one dot and at most 253 characters. Labels have 1 to 63 characters from `a-z`, `0-9` and `-`, and do not start or end with `-`.

| Input | Output |
|---|---|
| `"https://www.Acme.com/about?x=1"` | `"acme.com"` |
| `"ada@acme.co.uk"` | `"acme.co.uk"` |
| `"blog.acme.com:8080"` | `"blog.acme.com"` |
| `"localhost"`, `"acme..com"`, `"-acme.com"`, `"ac me.com"` | null |

Internationalized domains in Unicode form return null. Send them in punycode.

## Bound arguments

`{ "cast": "integer" }`, the other three `cast` targets, and `{ "split_name": "first" }` or `"last"` compile to a plan step with the argument already bound. The plan makes one call with no argument dispatch. This happens only for the default transforms. An injected transform with the same name gets the argument from the spec.

## Routing validators

A canonical field with `routing: true` needs a `semantic` from this table. The compiler adds its validator after the field's transforms. A validator returns the value unchanged for a pass and null for a fail. It never rewrites the value. A routing value that fails validation counts as an error, as with any other transform.

| Semantic | Passes | Fails |
|---|---|---|
| `email` | a string with exactly one `@`, not first, no spaces, and a valid domain after it: `"Ada@Acme.com"` | `"N/A"`, `"a@b@acme.com"`, `"@acme.com"`, `"ada @acme.com"`, `"ada@localhost"` |
| `phone` | anything `normalize_phone` accepts: `"+1 (415) 555-0100"` | `"n/a"` |
| `domain` | a string that is a valid domain as written (any case): `"acme.com"` | `"https://acme.com"` |
| `linkedin_url` | anything `normalize_linkedin` accepts | `"https://acme.com/in/ada"` |
| `url` | `http://` or `https://` (any case) followed by a valid domain host | `"acme.com"`, `"ftp://acme.com"` |
| `iso_date` | a string with no surrounding whitespace in a form `parse_date` accepts: `"2014-06"`, `"2014-06-01"` | `"2024-02-30"`, `" 2014-06-01"`, `"June 2014"` |
| `money` | a positive finite number, or a string of digits with commas and dots. In front, an optional `$ € £ ¥ ₹` or 3-letter uppercase code. Behind, an optional ` XXX` code and an optional `K`/`M`/`B` suffix: `"$1,200.50"`, `"USD 3.2M"`, `"1200 EUR"`, `"€40k"` | `"$"`, `"USD"`, `"-5"`, `"12 apples"`, `"1..2"`, `0` |

Validators check format only. A personal address passes `email` just as a work address does. [drift-and-learning-loop.md](drift-and-learning-loop.md) covers the known-entity check that catches that.

Routing fields cannot have a `default`. `compile_spec` rejects it, because the default skips the validator.
