# Transforms

pycanonic ships nine default transforms and seven routing validators. They live in `crates/canonic-core/src/transforms.rs`, and Python sees them as `default_transforms` and `validators`. Each one is a port of the TypeScript function, and the conformance corpus checks them against it.

A spec with `"language": 2` can also use the twenty-nine transforms in [Language 2 transforms](#language-2-transforms), and `parse_date` with a list of formats. A language 1 spec that names one of them fails the compile with `unknown transform "regex"`.

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

Language 2 also takes an object: `{"part": "first" | "last", "mode": "last_word" | "first_word"}`. `mode` is optional, and `"last_word"` is the rule above, so `{"part": "last"}` gives the same result as `"last"`. `first_word` splits on any JavaScript white space, takes the first word as the first name, and joins the other words with single spaces as the last name. A string that is not text gives null. An unknown key or value fails the compile.

| Input | `first`, `first_word` | `last`, `first_word` |
|---|---|---|
| `"Mary Ann Smith"` | `"Mary"` | `"Ann Smith"` |
| `"  Mary   Ann  "` | `"Mary"` | `"Ann"` |
| `"Ada"` | `"Ada"` | null |
| `""` or `"   "` | null | null |
| `5` or null | null | null |

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"last_name": {"type": "string"}}},
  "spec": {
    "language": 2,
    "provider": "acme",
    "endpoint": "people",
    "version": 1,
    "status": "promoted",
    "fields": {"last_name": {"from": "name", "transform": {"split_name": {"part": "last", "mode": "first_word"}}}}
  },
  "body": {"name": "Mary Ann  Smith"},
  "expected": {"last_name": "Ann Smith"}
}
```

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

Validators check format only. A personal address passes `email` the same way a work address does. [drift-and-learning-loop.md](drift-and-learning-loop.md) covers the known-entity check that catches that.

Routing fields cannot have a `default`. `compile_spec` rejects it, because the default skips the validator.

## Language 2 transforms

These transforms need `"language": 2` at the top of the spec. They follow the same rules as the nine above: each one returns null for input it cannot handle and never raises. A null from a value that was not null counts one error. A null input gives null and counts no error. The compiler checks each argument. A bad pattern or an unknown output layout fails the compile with `invalid argument for <name>`.

`default_transforms` holds these transforms too, for tests: `T["regex"]("a-1", "a-(\\d)")` gives `"1"`.

### regex

Argument, required: a pattern. If the pattern has a group, the result is group 1. Otherwise the result is the whole match. A string with no match, and anything that is not a string, gives null.

| Pattern | Input | Output |
|---|---|---|
| `"/team/([^/?#]+)"` | `"https://x.example/team/widget-works/?a=1"` | `"widget-works"` |
| `"id-(\\d+)"` | `"x id-42 y"` | `"42"` |
| `"\\d+"` | `"ab12cd34"` | `"12"` |
| `"id-(\\d+)"` | `42` | null |

The patterns use the Rust `regex` crate. It runs in linear time and has no backreferences or lookaround, so a hostile pattern or input cannot hang a request.

### strip_prefix and strip_suffix

Argument, required: a non-empty string. A string that starts (or ends) with it loses it. A string without it comes back unchanged. A string that is nothing but the prefix gives null. Anything that is not a string gives null.

| Transform | Input | Output |
|---|---|---|
| `{"strip_prefix": "id:acme:"}` | `"id:acme:ABC"` | `"ABC"` |
| `{"strip_prefix": "id:acme:"}` | `"ABC"` | `"ABC"` |
| `{"strip_prefix": "id:acme:"}` | `"id:acme:"` | null |
| `{"strip_suffix": ".git"}` | `"tools.git"` | `"tools"` |

### replace

Argument, required: `{"from": <text>, "to": <text>}` or `{"pattern": <regex>, "to": <text>}`. It replaces every match. `to` is plain text, also with a pattern, so `$1` is not a group reference. Anything that is not a string gives null.

| Argument | Input | Output |
|---|---|---|
| `{"from": "_", "to": " "}` | `"data_eng"` | `"data eng"` |
| `{"pattern": "[^0-9]+", "to": ""}` | `"+1 (415) 555-0100"` | `"14155550100"` |

### lookup

Argument, required: `{"table": {...}}` with an optional `"else"`. The table maps a value to a scalar or null. The key is the value's string form, as `to_string` writes it. So `2`, `2.0` and `"2"` all find the key `"2"`, and `true` finds `"true"`. An object or array input gives null.

`"else"` decides what a value outside the table gives:

| `else` | A value outside the table gives |
|---|---|
| missing, `null` or `"null"` | null, and one error |
| `"keep"` | the value as it is |
| any other scalar | that scalar |

```json
{"lookup": {"table": {"UP": "liked", "STAR": "starred"}, "else": "keep"}}
```

### null_if

Argument, required: a list of scalars. A value equal to one of them gives null. Numbers compare by value, so `0` also matches `0.0`. No other type crosses: `"0"` and `false` do not match `0`. An object or array input gives null.

```json
{"null_if": ["", "N/A", "-", 0]}
```

### upper and title

No argument. `upper` uppercases a string with Unicode rules (`"straße"` becomes `"STRASSE"`). `title` uppercases the first letter or digit of each word and lowercases the rest. A word starts after any character that is not a letter, a digit or an apostrophe, so `"jean-luc o'brien"` becomes `"Jean-Luc O'brien"`. Anything that is not a string gives null.

### split

Argument, required: `{"sep": <non-empty text>, "index": <integer>}`. It splits a string on `sep` and takes one part. A negative index counts from the end. An index out of range, or an empty part, gives null.

| Argument | Input | Output |
|---|---|---|
| `{"sep": " ", "index": 0}` | `"Ada Lovelace"` | `"Ada"` |
| `{"sep": " ", "index": -1}` | `"Ada King Lovelace"` | `"Lovelace"` |
| `{"sep": ",", "index": 1}` | `"a,,c"` | null |
| `{"sep": ",", "index": 1}` | `"a"` | null |

### to_string

No argument. A string comes back as it is. Numbers and booleans come back as JavaScript `String()` writes them: `5` gives `"5"`, `2.5` gives `"2.5"`, `true` gives `"true"`. An object or array gives null.

### format_date

Argument, required: the output layout.

| Argument | Output for `"2021-03-01T23:30:00-05:00"` |
|---|---|
| `"date"` | `"2021-03-02"` |
| `"datetime"` | `"2021-03-02T04:30:00Z"` |
| `"datetime_ms"` | `"2021-03-02T04:30:00.000Z"` |
| `"epoch_s"` | `1614659400` |
| `"epoch_ms"` | `1614659400000` |

The input is what `parse_date` reads (`YYYY`, `YYYY-MM`, `YYYY-MM-DD` or an epoch number), or an ISO date and time. A time with an offset is converted to UTC, so the date can move. A partial date fills day 1 and month 1, and a date with no time is midnight UTC: `"2021-03"` gives `"2021-03-01T00:00:00.000Z"`.

The time part comes after the `T` or space. It must read as `HH:MM`, then an optional `:SS` and fraction, then an optional offset (`Z`, `+01:00` or `+0100`). A time it cannot read gives null and counts an error. It never guesses midnight: `"2021-03-01Tjunk"` and `"2021-03-01T25:00:00Z"` give null. Other layouts, such as `"June 2014"`, give null too. Put `parse_date` with a format in front for those.

### parse_date with formats

Language 1 `parse_date` takes no argument. Language 2 also accepts `{"formats": [...]}`, a non-empty list of layouts. Each layout holds one `%Y` and at most one month: `%m` for 1 or 2 digits, `%b` for `Jun`, or `%B` for `June`. It can hold a `%d` only with a month. Any other character must match itself. Month names are English, in any case. The output has the precision of the layout: `"%B %Y"` on `"june 2014"` gives `"2014-06"`.

The first layout that fits the whole trimmed string wins. If none fits, the language 1 rules run, so `"2021-03-01"` still gives `"2021-03-01"`. A date that does not exist (`"31 Feb 2021"` with `"%d %b %Y"`) gives null.

### parse_number

Argument, optional: `{"units": true}`. It reads a number from text: commas are dropped, and one leading currency sign (`$ € £ ¥ ₹`) or 3-letter code (`USD 40`) is skipped. A number input comes back as it is. A boolean gives null.

With `"units": true`, a `K`, `M` or `B` suffix in any case multiplies by one thousand, one million or one billion.

| Input | `parse_number` | `{"parse_number": {"units": true}}` |
|---|---|---|
| `"1,234.5"` | `1234.5` | `1234.5` |
| `"USD 40"` | `40` | `40` |
| `"40k"` | null | `40000` |
| `"$3.2M"` | null | `3200000` |

### parse_range

Argument, required: `"min"` or `"max"`. It reads one end of a size range. A number input gives itself for both ends.

| Input | `min` | `max` |
|---|---|---|
| `"100-999"` | `100` | `999` |
| `"100 - 999"` | `100` | `999` |
| `"5"` | `5` | `5` |
| `"1,000+"` | `1000` | null, and one error |
| `"a-b"` | null | null |

An open range has no max, so `max` gives null and counts an error, as any null from a value that was not null does. See [known-limits.md](known-limits.md#an-open-range-counts-as-an-error).

### Numbers

Language 2 has numbers and compares: `add`, `sub`, `mul`, `div`, `ceil_div`, `floor_div`, `min`, `max`, `eq`, `lt`, `lte`, `gt` and `gte`. On one source they take one number argument, as in `{"mul": 100}`. On a `from` list of two or more sources, the first transform takes no argument and folds the sources left to right: `{"from": ["total", "$input.size"], "transform": "ceil_div"}`. A transform later in the chain gets one value, so it takes an argument. Any other combination fails the compile.

An input is a number, or a string that holds one (the rule of `cast`). Booleans, objects, arrays, null and a missing source give null. A fold with a missing or null source gives null. Division by zero gives null, and so does a result that is not finite. `ceil_div` and `floor_div` give integers, and a whole result of the other number transforms is an integer. The compares give booleans. A custom transform of the same name wins over the default. In a chain, a null result counts as one error when the source was present.

| Rule | Input | Output |
|---|---|---|
| `ceil_div` on `["total", "size"]` | `101`, `10` | `11` |
| `ceil_div` | `"101"`, `10` | `11` |
| `div` | `2.5`, `2` | `1.25` |
| `div` or `ceil_div` | `101`, `0` | null |
| `add` | `"1e400"`, `1` | null |
| `mul` | `1e200`, `1e200` | null |
| `add` | `true`, `1` | null |
| `{"mul": 100}` | `1.5` | `150` |
| `{"eq": 0}` | `"0"` | `true` |
| `gte` on `["a", "b"]` | `2`, `2` | `true` |

### len

No argument. It gives the number of elements of an array, or the number of characters of a string. An object, a number, a boolean, null and a missing source give null. As the first transform on a `from` list, it gives null.

| Input | Output |
|---|---|
| `[]` | `0` |
| `[1, 2]` | `2` |
| `"héllo"` | `5` |
| `{}` | null |

### format

Argument, required: a template string. It builds a string from the sources, so it is the inverse of `strip_prefix`: a bare id becomes a full id or a URL. On one source, `{}` or `{0}` writes the value. On a `from` list, `{0}`, `{1}` and so on write the sources in order, and the slots can come in any order and repeat. `{{` writes `{` and `}}` writes `}`. As the first transform on a `from` list, it fills the slots from the list. Later in a chain it gets one value.

A number is written as JavaScript writes it, and a big integer keeps its exact digits. A boolean is `true` or `false`. A null, a missing source or an object or array gives null, and so does a list with any such source. A null from a source that was present counts one error.

These arguments fail the compile, with the field and an example: a template with no placeholder, `{}` mixed with `{n}`, an unclosed `{` or a stray `}`, `{}` on a `from` list, an index past the last source, and `{1}` or higher on one source.

| Rule | Input | Output |
|---|---|---|
| `{"format": "urn:example:item:{}"}` | `"123"` | `"urn:example:item:123"` |
| same | `123` | `"urn:example:item:123"` |
| same | `true` | `"urn:example:item:true"` |
| same | `""` | `"urn:example:item:"` |
| same | null, missing, `{}` or `[]` | null |
| `{"format": "{0}/{1}"}` on `["org", "name"]` | `"acme"`, `"ada"` | `"acme/ada"` |
| same | `"acme"`, null | null |
| `{"format": "{{x}}={}"}` | `"1"` | `"{x}=1"` |
| `{"format": "item:"}` | any | compile error |
| `{"format": "{}/{0}"}` | any | compile error |
| `{"format": "{}"}` on `["a", "b"]` | any | compile error |
| `{"format": "{0}{2}"}` on `["a", "b"]` | any | compile error |

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"url": {"type": "string"}}},
  "spec": {
    "language": 2,
    "provider": "acme",
    "endpoint": "people",
    "version": 1,
    "status": "promoted",
    "fields": {"url": {"from": ["org", "slug"], "transform": {"format": "https://x.example/{0}/people/{1}"}}}
  },
  "body": {"org": "acme", "slug": "ada"},
  "expected": {"url": "https://x.example/acme/people/ada"}
}
```

### url_part

Argument, required: `"host"`, `"path"`, `{"path_segment": <integer>}` or `{"query": "<name>"}`. It reads one part of a URL. Only a string that holds `scheme://` counts. Anything else gives null: no `://`, a scheme with characters other than letters, digits, `+`, `-` and `.`, or a value that is not a string.

- `host` is the host as written (case is kept), without userinfo and without the port. An IPv6 literal keeps its brackets. An empty host gives null.
- `path` is the percent-decoded path, so `%2F` becomes `/`. Use `path_segment` to read one segment. The path runs from the first `/` after the host to the `?` or `#`. No path gives null. `+` stays `+`.
- `path_segment` is the n-th non-empty segment of the path, decoded. Zero is the first, and a negative count goes from the end (`-1` is the last). A segment past either end gives null.
- `query` is the value of the first parameter with that name, from the text between `?` and `#`. Pairs split on `&` and names split on the first `=`. Names are compared after decoding. `+` is a space in the name and the value. A parameter that is present but empty (`?q=` or `?q`) gives `""`.
- A bad escape in the part that is read gives null: `%zz`, a lone `%` or `%4`, or bytes that are not UTF-8 after decoding. A bad escape in another path segment or another parameter does not matter.

A null from a string counts one error.

The examples read `https://x.example/people/ada%20lovelace/?ref=a%2Fb&q=1+2#top`.

| Rule | Output |
|---|---|
| `"host"` | `"x.example"` |
| `"path"` | `"/people/ada lovelace/"` |
| `{"path_segment": 0}` | `"people"` |
| `{"path_segment": -1}` | `"ada lovelace"` |
| `{"path_segment": 5}` | null |
| `{"query": "ref"}` | `"a/b"` |
| `{"query": "q"}` | `"1 2"` |
| `{"query": "zz"}` | null |
| `{"query": "top"}` | null (the fragment is not the query) |
| `"host"` on `"x.example/people"` | null (no `://`) |
| `"host"` on `"https://u:pw@X.Example:8080/p"` | `"X.Example"` |
| `"path"` on `"https://x.example/%zz"` | null |

A bad argument fails the compile with the argument that was given and a valid example.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"slug": {"type": "string"}, "ref": {"type": "string"}}},
  "spec": {
    "language": 2,
    "provider": "acme",
    "endpoint": "people",
    "version": 1,
    "status": "promoted",
    "fields": {
      "slug": {"from": "link", "transform": {"url_part": {"path_segment": -1}}},
      "ref": {"from": "link", "transform": {"url_part": {"query": "ref"}}}
    }
  },
  "body": {"link": "https://x.example/people/ada%20lovelace/?ref=a%2Fb&q=1+2#top"},
  "expected": {"slug": "ada lovelace", "ref": "a/b"}
}
```

## Container transforms

In a language 2 spec, a single custom transform on an array or object field gets the whole source value. It returns a list or a dict. [spec-format.md](spec-format.md#container-transforms) has the rules.
