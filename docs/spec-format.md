# Spec format

A spec maps one provider's response shape onto your canonical schema. `compile_spec` turns it into a plan that the Rust core runs. pycanonic reads the same spec files as the TypeScript canonic. The grammar is `spec.schema.json` at the repo root. `crates/canonic-core/src/spec.rs` checks the same grammar by hand. A test fails on any disagreement between the two.

Two example specs map different providers onto one canonical schema:

- `python/tests/fixtures/specs/canonical/person.enrich.json` is the canonical schema.
- `python/tests/fixtures/specs/acme.json` maps a nested camelCase provider.
- `python/tests/fixtures/specs/globex.json` maps a flat snake_case provider.

## Top-level keys

| Key | Required | Value |
|---|---|---|
| `provider` | yes | non-empty string |
| `endpoint` | yes | non-empty string |
| `version` | yes | integer, 1 or more |
| `status` | yes | `draft`, `shadow`, `promoted` or `deprecated` |
| `fields` | yes | non-empty object, canonical field to rule |
| `anchors` | no | routing field to raw key token, see [Anchors](#anchors) |
| `$schema` | no | string, for editor autocomplete |

Any other top-level key is an error. `provider`, `endpoint` and `version` identify the artifact. Every error found against the canonical schema starts with them (`acme person.enrich v1: ...`). Grammar errors start with `invalid spec:`.

The artifact carries `status`. `Registry.put` serves only `promoted` and `shadow` artifacts and raises on `draft` or `deprecated`. `canonic infer` writes `draft`. `canonic diff` proposes the next version as `shadow`.

## Field keys

Keys of `fields` are canonical paths: identifiers (`[A-Za-z_][A-Za-z0-9_]*`) joined by dots. A key that ends in `[]` is an array field and takes an array rule. Every other key takes a scalar rule.

```json
"company.domain": { "from": "org.website", "transform": "to_domain" },
"skills[]": { "from": "tags", "pluck": "label", "transform": "lower" }
```

Canonical fields you do not map still appear in the output, as `None`. `map()` always returns the full canonical shape in schema order.

## Source paths

Every `from`, `coalesce` entry and `pluck` is a path into the provider response. A path is a sequence of:

- identifiers, `[A-Za-z_$][A-Za-z0-9_$-]*`, joined by dots: `org.website`, `data.job-title`
- array indices: `emails[0]`, `[0]`
- quoted keys in JSON string syntax, for anything else: `["full-name"]`, `geo["home city"]`
- a leading `$`, which is the response itself: `$` alone, `$[0].name`, `$.data` (the same as `data`)

```
contact.emails[0].address
["job title"][0]
_meta.updated
$
```

Use `$` for a provider whose response is a bare array: `"results[]": { "from": "$", "item": { ... } }`. Write a key that is literally named `$` as `["$"]`. A routing field cannot read `$`.

A name reads a key of a JSON object. An index reads only arrays. A path that does not resolve gives `None` and never raises.

## Scalar rules

The short form is a bare path:

```json
"full_name": "profile.fullName"
```

The object form takes exactly one of `from` or `coalesce`, plus optional `default` and `transform`:

| Key | Value |
|---|---|
| `from` | a path, or an array of 2 or more paths |
| `coalesce` | an array of 2 or more paths |
| `default` | string, number or boolean |
| `transform` | see [Transforms](#transforms) |

`coalesce` reads the paths in order and takes the first value that is present and not null. An empty string counts as present. The transforms then run on that one value. If they reject it, pycanonic does not go back to the next path. From `acme.json`:

```json
"email": {
  "coalesce": ["contact.workEmail", "contact.emails[0].address"],
  "transform": ["trim", "lower"]
}
```

`from` with an array of paths passes a list of their values to the transforms, with `None` for a missing one. The value is `None` only for all paths missing. This form exists for `join`. From `globex.json`:

```json
"full_name": { "from": ["fname", "lname"], "transform": "join" }
```

Without a transform, the field gets the list itself.

`default` is used for a missing source, and for a transform that returns null. It is returned as written and does not go through `transform`. Routing fields cannot have one. `compile_spec` rejects it, because it skips the validator. A field that falls back to its default counts as a miss in `artifact.stats`. A transform failure also counts as an error.

## Array rules

An array field (`"name[]"`) needs an object with `from`, a path to the provider's array. There are three forms:

| Form | Canonical items | Each element becomes |
|---|---|---|
| `item` | objects | an object mapped by the nested field map |
| `pluck` | scalars | the value at the `pluck` path inside the element |
| neither | scalars | the element itself |

A canonical array with no `items` takes the third form. Each element is copied as it is, objects included.

`item` holds a field map with the same rules as `fields`, relative to each element. From `acme.json`:

```json
"experiences[]": {
  "from": "workHistory",
  "item": {
    "title": "role",
    "company": "orgName",
    "start": { "from": "from", "transform": "parse_date" }
  }
}
```

`pluck` reads one path out of each element:

```json
"skills[]": { "from": "tags", "pluck": "label", "transform": "lower" }
```

With neither, each plain value is copied. From `globex.json`:

```json
"skills[]": { "from": "skill_list", "transform": "lower" }
```

`transform` on a `pluck` or plain-value array runs once per element. An element that the transform rejects becomes `None` in place. So `["SQL", 5, null, "Go"]` with `lower` maps to `["sql", None, None, "go"]` and counts one error, for the `5`. A null element is not an error.

Rules:

- `item` and `pluck` cannot be combined.
- `item` cannot take `transform`. Put transforms on the item's own fields.
- `item` needs a canonical array of objects. `pluck` and plain values need a canonical array of scalars.
- A missing or null `from` gives `None`. A `from` that is present but is not an array gives `None` and counts an error.
- Arrays are never collapsed. A one-element array stays an array. A single object where an array was expected is `None`.

## Transforms

`transform` takes one of three forms:

```json
"transform": "normalize_phone"
"transform": { "split_name": "first" }
"transform": ["trim", "lower"]
```

The forms are a name, a one-key object with the name and its argument, or a non-empty array of either. An array runs left to right and stops at the first null. Names match `[A-Za-z_][A-Za-z0-9_]*`. See [transforms.md](transforms.md) for the defaults and [custom-transforms.md](custom-transforms.md) for your own.

The compiler checks each name and the argument of each default transform. An unknown name or a bad argument fails the compile.

## Anchors

```json
"anchors": { "email": "\"e_mail\":" }
```

An anchor lets `artifact.extract` find a routing field by a search for its raw key token, without a scan. The compiler accepts an anchor only for these cases:

- the key is a routing field in this spec,
- the field reads a single `from` path whose last segment is a key,
- the token is exactly that key as a JSON string followed by `:`. So `from: "contact.e_mail"` needs `"\"e_mail\":"`.

`canonic infer` proposes an anchor only for a token that occurs exactly once in every raw sample. See [streaming.md](streaming.md).

## Canonical schema

The canonical schema is a JSON Schema object. pycanonic reads `properties` and `type: "array"` with `items`. An array without `items` holds elements of any shape. It also reads these keywords on leaves (`crates/canonic-core/src/canonical.rs`):

| Keyword | Used by | Effect |
|---|---|---|
| `semantic` | compiler, inference | the kind of value: `email`, `phone`, `domain`, `linkedin_url`, `url`, `iso_date`, `money` |
| `routing` | compiler | `true` marks a routing field |
| `aliases` | inference | extra names to match provider keys against |
| `type` | inference | picks a `cast` and scores values. The compiler ignores it. |

A routing field must have a `semantic` that has a validator. The check runs as soon as a spec maps the field. The compiler adds that validator after the field's transforms. So a routing value that fails validation is `None` and counts an error. `extract` and `stream` read the routing fields. `artifact.routing` lists the ones this spec maps. From `person.enrich.json`:

```json
"email": {
  "type": "string",
  "semantic": "email",
  "routing": true,
  "aliases": ["work_email", "business_email"]
}
```

The loader enforces these limits:

- A routing field cannot sit inside an array.
- Arrays of arrays cannot be addressed.
- Field names must be identifiers. `toJSON` and every name on the JavaScript `Object.prototype` (`constructor`, `toString`, `valueOf`, `__proto__` and others) are rejected. This keeps every spec portable between pycanonic and the TypeScript package.

`validate_canonical(schema)` runs these checks without a spec. It returns the problems as a list of `Issue`, or `[]`. Call it where you write or generate a schema.

## Errors

Grammar errors come back together in one message, each with its location:

```
invalid spec: spec.status: must be one of draft, shadow, promoted, deprecated; spec.fields["email"]: needs exactly one of from, coalesce; spec.fields["skills[]"]: item and pluck can't be combined
```

Errors against the canonical schema stop at the first one. Each names the field path, nested items included:

```
acme person.enrich v2: fields["nickname"]: canonical schema has no field "nickname"
acme person.enrich v2: fields["email"]: unknown transform "fix_email"
acme person.enrich v2: fields["first_name"]: invalid argument for split_name
acme person.enrich v2: fields["skills"]: canonical field is an array; add []
acme person.enrich v2: fields["experiences[]"]: array of objects; use item
acme person.enrich v2: fields["experiences[]"].item["title"]: unknown transform "nope"
acme person.enrich v2: anchors["title"]: only routing fields take anchors
```

The error text is the same as in the TypeScript package. The conformance corpus checks every message. `validate_spec(spec)` returns the grammar issues as `Issue` objects without a compile, for editors and CI.
