# Spec format

A spec maps one provider's response shape onto your canonical schema. `compile_spec` turns it into a plan that the Rust core runs. pycanonic reads the same spec files as the TypeScript canonic. The sections up to [Language 2](#language-2) describe language 1, which both packages share. Language 2 adds rules that only pycanonic has. The grammar is `spec.schema.json` at the repo root. `crates/canonic-core/src/spec.rs` checks the same grammar by hand. A test fails on any disagreement between the two.

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

## Language 2

Language 2 adds the rules that real provider adapters need: constants, empty values, optional objects, list operations, grouping, request inputs, rejects, more transforms, definitions and looser canonical schemas. pycanonic has language 2. The TypeScript canonic does not. See [known-limits.md](known-limits.md#language-2-is-pycanonic-only).

A spec turns it on with `"language": 2` at the top level, an integer. A spec without the key is language 1. Any other value fails with `invalid spec: spec.language: must be 1 or 2`. A language 2 rule in a language 1 spec fails the compile and names the rule:

```
invalid spec: spec.fields["kind"]: "const" needs "language": 2
invalid spec: spec: "reject" needs "language": 2
```

The TypeScript package rejects the `language` key, so it fails loudly on a language 2 spec instead of mapping it wrongly. A plan that uses a language 2 rule is written as plan format 2. A plan with no language 2 rule stays format 1, so older readers still load it. `load` reads both formats.

Every rule gives the same output for `map(bytes)`, `map(dict)` and `map_json`. Mapping never raises on a bad body. The one exception is the `reject` rule, which raises `pycanonic.Rejected` on purpose.

Each example below marked as runnable is a whole case: a canonical schema, a spec, a body and the output of `map`. `python/tests/test_docs_examples.py` runs each one. The tables list what each rule gives for the inputs where hand-written adapters usually break: a missing path, null, an empty string, an empty array, an empty object and a wrong type. `conformance/cases-v2/` has one folder of cases per rule.

### Top-level keys in language 2

| Key | Value |
|---|---|
| `language` | `2` |
| `keys` | `"exact"` (the default) or `"case_insensitive"`, see [Keys](#keys) |
| `reject` | a non-empty list of conditions, see [Reject](#reject) |
| `definitions` | named field maps, see [Definitions](#definitions) |

In language 2, a field key is a path of keys that can end in `[]`. Any key name works: write one that is not an identifier in the quoted form, such as `["first-name"]` or `meta["@type"]`.

### Constants

`{"const": <any JSON value>}` writes the value as it is, whatever the body holds. The value can be a string, a number, a boolean, null, `[]`, `{}` or an object. A const field counts as a fill in `artifact.stats`, never as a miss.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"name": {"type": "string"}, "kind": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "org.enrich", "version": 1, "status": "draft",
    "fields": {"name": "org_name", "kind": {"const": "COMPANY"}, "tags[]": {"const": []}}},
  "body": {"org_name": "Widget Works", "kind": "PERSON"},
  "expected": {"name": "Widget Works", "kind": "COMPANY", "tags": []}
}
```

| Body | Output |
|---|---|
| any body, also one that is not JSON | the const value |

A const can take `when` and `omit` (see [Request inputs and conditions](#request-inputs-and-conditions)). On an object node, the const is the whole object. So a field key below it, such as `meta.v` under a const `meta`, fails with `inside a const field`. A `[]` key with a const needs an array node.

### Empty values

An array rule takes two options:

- `"missing"`: what a missing or null source gives, `"null"` (the default) or `"[]"`.
- `"empty"`: what an empty source array gives, `"[]"` (the default) or `"null"`.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"tags": {"type": "array", "items": {"type": "string"}}, "jobs": {"type": "array", "items": {"type": "string"}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "draft",
    "fields": {"tags[]": {"from": "labels", "missing": "[]"}, "jobs[]": {"from": "roles", "empty": "null"}}},
  "body": {"roles": []},
  "expected": {"tags": [], "jobs": null}
}
```

| Source | No option | `"missing": "[]"` | `"empty": "null"` |
|---|---|---|---|
| path missing | null | `[]` | null |
| null | null | `[]` | null |
| `""` | null, one error | null, one error | null, one error |
| `[]` | `[]` | `[]` | null |
| `{}` | null, one error | null, one error | null, one error |
| `"x"` or `5` | null, one error | null, one error | null, one error |

The `empty` option also acts on the result of the list operations. An array that a `filter` takes down to nothing is empty.

The canonical schema can set the empty form once, for every spec that maps the node, with `"x-empty"`:

| Node | Allowed `x-empty` | A missing or null source gives |
|---|---|---|
| leaf | `"null"`, `"missing"` | null, or no key for `"missing"` |
| array | `"null"`, `"[]"`, `"missing"` | null, `[]`, or no key |
| object | `"null"`, `"{}"`, `"missing"` | null, `{}`, or no key |

The schema loader rejects any other value, for example with `x-empty "[]" needs an array node`. A spec option (`missing`, `empty`, `if_empty`) wins over `x-empty`. `canonic schema` writes `x-empty` from the default provider's samples, so a fitted spec rarely needs the options.

`x-empty` applies in these cases:

- A mapped leaf that is null. With `"missing"`, the key is left out.
- A canonical node that the spec does not map. It takes its `x-empty` form. Without `x-empty`, an unmapped leaf or array is null, and an unmapped object is an object of nulls, as in language 1.
- A mapped object whose leaves are all null or absent. With `x-empty`, it becomes null, `{}` or no key. Without `x-empty`, it stays an object of nulls. An array inside it that gives `[]` only because its source is missing does not count as a value.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {
    "name": {"type": "string"},
    "nickname": {"type": "string", "x-empty": "missing"},
    "hq": {"type": "object", "x-empty": "null", "properties": {"city": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}, "x-empty": "[]"}}},
    "links": {"type": "array", "items": {"type": "string"}, "x-empty": "[]"}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "draft",
    "fields": {"name": "n", "nickname": "nick", "hq.city": "office.city", "hq.tags[]": {"from": "office.tags"}}},
  "body": {"n": "Ada", "nick": null},
  "expected": {"name": "Ada", "hq": null, "links": []}
}
```

### Optional objects

A field key can name a canonical object node, not only a leaf. Its rule is `{"present": <path>}` or `{"present": [<path>, ...]}`. Every path must be there and not null. If the condition fails, the object is null, or no key under `x-empty` `"missing"`. If it holds, the leaves inside are mapped as usual. A present object whose leaves are all null follows its `x-empty`, as above.

With `"if_empty": "null"`, an object whose leaves are all null is null. It needs no `present`. `"null"` is its only value. Both work inside `item` maps too. `present` or `if_empty` on a node that is not an object fails with `present needs an object node` or `if_empty needs an object node`.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"name": {"type": "string"}, "hq": {"type": "object", "properties": {"city": {"type": "string"}, "country": {"type": "string"}}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "org.enrich", "version": 1, "status": "draft",
    "fields": {"name": "n", "hq": {"present": "data.hq"}, "hq.city": "data.hq.city", "hq.country": "data.hq.country"}},
  "body": {"n": "Widget Works", "data": {"hq": null}},
  "expected": {"name": "Widget Works", "hq": null}
}
```

| `data.hq` | `{"present": "data.hq"}` | `{"if_empty": "null"}` |
|---|---|---|
| path missing | null | null |
| null | null | null |
| `""` | object of nulls | null |
| `[]` | object of nulls | null |
| `{}` | object of nulls, or `{}` under `x-empty` `"{}"` | null |
| `{"city": "Oslo"}` | `{"city": "Oslo", "country": null}` | `{"city": "Oslo", "country": null}` |

### Wrap

`"wrap": true` on an array rule turns a scalar or an object into a one-element array. An array source stays as it is. A missing or null source follows `missing` and `x-empty`. `wrap` acts on the top-level source only, not on each step of a `[]` path.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"industries": {"type": "array", "items": {"type": "string"}}, "offices": {"type": "array", "items": {"type": "object", "properties": {"city": {"type": "string"}}}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "org.enrich", "version": 1, "status": "draft",
    "fields": {"industries[]": {"from": "industry", "wrap": true}, "offices[]": {"from": "office", "wrap": true, "item": {"city": "city"}}}},
  "body": {"industry": "software", "office": {"city": "Oslo"}},
  "expected": {"industries": ["software"], "offices": [{"city": "Oslo"}]}
}
```

| Source | `"wrap": true` |
|---|---|
| path missing | null, or `[]` with `"missing": "[]"` |
| null | null, or `[]` with `"missing": "[]"` |
| `""` | `[""]` |
| `[]` | `[]` |
| `{}` | `[{}]`, or with `item` an object of nulls |
| `"x"` or `5` | `["x"]` or `[5]` |

### List operations

An array rule can take these options. They run in this order: flatten, `filter`, `unique`, `sort_by`, `limit`, then `group_by`.

- Flatten: a `from` path with `[]` in it collects across nested arrays, as in `"from": "jobs[].skills[]"`. Such a path must end in `[]`. A step that is missing or is not an array adds nothing and counts no error.
- `"filter"`: keeps the elements where a condition holds. A list of conditions means all of them must hold.
- `"unique"`: `true` compares whole elements, and a path compares the value at that path. The first element of each value stays. Elements whose key is null or missing all stay.
- `"sort_by"`: a path, with `"order": "asc"` (the default) or `"desc"`. The sort is stable. Null and missing values go last in both orders.
- `"limit"`: a positive integer. It keeps the first elements.

A condition has one of these forms. Its `path` is relative to the element, and `$` is the element itself. A condition path can also read `$input` (see [Request inputs and conditions](#request-inputs-and-conditions)).

| Condition | Holds when |
|---|---|
| `{"path": p, "eq": v}` | the value equals `v`. Numbers compare by value (`1` equals `1.0`). Other types never match across types, so `"1"` does not equal `1`. Objects compare without regard to key order. |
| `{"path": p, "in": [v, ...]}` | the value equals one of the list, as `eq` compares |
| `{"path": p, "present": true}` | the value is there and not null |
| `{"path": p, "present": false}` | the value is missing or null |

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"skills": {"type": "array", "items": {"type": "string"}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "draft",
    "fields": {"skills[]": {"from": "jobs[].skills[]", "filter": {"path": "level", "in": [1, 2, 3]}, "unique": "name", "sort_by": "level", "order": "desc", "limit": 2, "pluck": "name"}}},
  "body": {"jobs": [{"skills": [{"name": "a", "level": 1}, {"name": "b", "level": 9}]}, {"skills": [{"name": "c", "level": 2}, {"name": "a", "level": 3}, {"name": "d", "level": 2}]}]},
  "expected": {"skills": ["c", "d"]}
}
```

`sort_by` on values of mixed types puts numbers first, then strings, then booleans (`false` before `true`), then arrays and objects. Strings compare by Unicode code point, so `"B"` sorts before `"a"`. With `desc`, the whole order reverses, but null stays last. So `[3, "b", null, true, 1, "a"]` sorts to `[1, 3, "a", "b", true, null]`.

| Source | Any list operation |
|---|---|
| path missing | null, or the `missing` and `x-empty` form |
| null | the same as missing |
| `""`, `{}`, `5` | null, one error |
| `[]` | `[]`, or null with `"empty": "null"` |
| no element passes `filter` | `[]`, or null with `"empty": "null"` |

### Group and aggregates

`"group_by": <path>` on an array rule groups the elements by the value at that path, relative to each element.

- With `"consecutive": true` (the default), a new group starts each time the key changes, and the groups keep the source order.
- With `"consecutive": false`, all elements with the same key form one group, in the order of first appearance.
- An element whose key is null or missing is a group of its own.
- Keys compare by their JSON text with sorted object keys, and numbers compare by value. So `{"a": 1, "b": 2}` and `{"b": 2, "a": 1}` group together, and `1` groups with `1.0`.
- `group_by` runs after `limit`, so `limit` counts elements, not groups.

Inside the `item` map of a grouped rule:

- A plain path reads the first element of the group.
- `"$group"` is the group's elements, as the `from` of a nested array rule: `"roles[]": {"from": "$group", "item": {...}}`.
- `{"aggregate": <op>, "from": "$group[].<path>"}` folds one path across the group. `{"aggregate": "count", "from": "$group"}` counts the elements.

`$group` anywhere else fails the compile, for example `$group needs a group_by array` or `$group is only a list or aggregate from`. In language 2, a body key named `$group` needs the quoted form `["$group"]`.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"jobs": {"type": "array", "items": {"type": "object", "properties": {
    "company": {"type": "string"}, "start": {"type": "string"}, "end": {"type": "string"},
    "roles": {"type": "array", "items": {"type": "object", "properties": {"title": {"type": "string"}}}}}}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "draft",
    "fields": {"jobs[]": {"from": "positions", "group_by": "company_id", "item": {
      "company": "company_name",
      "start": {"aggregate": "min", "from": "$group[].started"},
      "end": {"aggregate": "max", "from": "$group[].ended", "nulls": "propagate"},
      "roles[]": {"from": "$group", "item": {"title": "title"}}}}}},
  "body": {"positions": [
    {"company_id": "A", "company_name": "Orbit", "title": "Engineer", "started": "2019-01", "ended": "2020-06"},
    {"company_id": "A", "company_name": "Orbit", "title": "Lead", "started": "2020-07", "ended": null},
    {"company_id": "B", "company_name": "Nimbus", "title": "Intern", "started": "2018-01", "ended": "2018-09"}]},
  "expected": {"jobs": [
    {"company": "Orbit", "start": "2019-01", "end": null, "roles": [{"title": "Engineer"}, {"title": "Lead"}]},
    {"company": "Nimbus", "start": "2018-01", "end": "2018-09", "roles": [{"title": "Intern"}]}]}
}
```

The aggregate ops:

| Op | Result | Takes |
|---|---|---|
| `first`, `last` | the first or last value, passed through as it is | any value |
| `min`, `max` | the smallest or largest value | numbers, or strings by code point, so ISO dates compare right |
| `count` | the number of elements, or of values that are not null for a `[].<path>` | any value |
| `sum` | the total | numbers |
| `any`, `all` | whether any or all values are `true` | booleans |
| `join` | the values joined with `"sep"` (default one space) | strings and numbers |

`"nulls": "skip"` (the default) leaves null values out. With `"nulls": "propagate"`, any null value makes the result null. A value of a type the op does not take gives null and counts an error. So does a mix of numbers and strings for `min` and `max`.

Aggregates also work outside a group, on any array path. `{"aggregate": "count", "from": "data.items"}` counts the items of a page.

| Source | `count`, `sum` | `any` | `all` | other ops |
|---|---|---|---|---|
| path missing or null | `0` | `false` | `true` | null |
| `[]` | `0` | `false` | `true` | null |
| `""`, `{}`, `5` | null, one error | null, one error | null, one error | null, one error |

A grouped source follows the list operation table: a missing source gives null, `[]` gives `[]`, and a source that is not an array gives null and counts an error.

### Request inputs and conditions

`map(body, context={...})` and `map_json(raw, context={...})` take the request inputs as a dict. A path that starts with `$input` reads it, as in `$input.page`. `$input` works as a scalar `from` and as a condition path. Without a context, every `$input` path is missing, and nothing raises. A body key named `$input` is not read this way. Write it as `["$input"]`.

A context must be a JSON object. A plan that reads `$input` raises `TypeError("context must be a JSON object: <reason>")` for anything else, on every way in. A plan that does not read `$input` ignores the context.

Any field rule can take `"when": <condition>` or a list of conditions, with the forms of `filter`. The condition path starts at the response root, or at the element inside an `item` map. If the condition fails, the field is null, or no key under `x-empty` `"missing"`. On an object node, `{"when": ...}` acts on the whole object.

With `"omit": true` and a failed `when`, the key is left out of the output. `omit` needs `when`, and it must be `true` or `false`. It is the only rule that changes the output's key set, so `canonic fit` always sends it to review.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"name": {"type": "string"}, "page": {"type": "number"}, "bio": {"type": "string"}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.search", "version": 1, "status": "draft",
    "fields": {"name": "name", "page": "$input.page", "bio": {"from": "bio", "when": {"path": "$input.full", "eq": true}, "omit": true}}},
  "body": {"name": "Ada", "bio": "Mathematician"},
  "context": {"page": 2, "full": false},
  "expected": {"name": "Ada", "page": 2}
}
```

| Condition path | `eq`, `in` | `present: true` | `present: false` |
|---|---|---|---|
| path missing, or no context for `$input` | reads as null: holds only for `"eq": null` or an `in` list with null | fails | holds |
| null | holds only for `"eq": null` or an `in` list with null | fails | holds |
| `""`, `[]`, `{}` | holds on an equal value | holds | fails |

A routing field cannot read `$input` and cannot have `when`, and neither can an object that holds one. Both fail the compile (`routing fields can't read $input`, `routing fields can't have when`), because `extract` has no context.

### Reject

The top-level `"reject"` key holds a list of conditions. Besides the `filter` forms, a reject condition can be `{"missing": <path>}` (missing or null) or `{"empty": <path>}` (missing, null, `""`, `[]` or `{}`). Paths start at the response root and can read `$input`.

If any condition holds, `map` and `map_json` raise `pycanonic.Rejected`. Its `reason` is the first condition that held, as compact JSON text, for example `'{"missing":"data.name"}'`. The use case is failover: a provider answers 200 with a body that has no usable data, and the caller tries the next provider. `reason` also survives pickling, so a worker process can pass it on.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"name": {"type": "string"}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "draft",
    "reject": [{"missing": "data.name"}, {"path": "data.status", "eq": "not_found"}],
    "fields": {"name": "data.name"}},
  "body": {"data": {"name": null}},
  "expected": {"$rejected": "{\"missing\":\"data.name\"}"}
}
```

| Value at the path | `missing` | `empty` |
|---|---|---|
| path missing | rejects | rejects |
| null | rejects | rejects |
| `""` | maps | rejects |
| `[]` or `{}` | maps | rejects |
| `0`, `false`, `[0]`, `{"k": null}` | maps | maps |
| body that is not JSON | rejects | rejects |

`artifact.stats.snapshot()` has a `"rejected"` count only for a plan with reject conditions. A rejected body counts there, not in `"responses"`, so it does not lower fill rates. `extract` and `stream` ignore reject rules.

### Transforms in language 2

Language 2 adds thirteen default transforms and formats for `parse_date`. The string transforms are `regex`, `strip_prefix`, `strip_suffix`, `replace`, `lookup`, `null_if`, `upper`, `title`, `split` and `to_string`. The date and number transforms are `format_date`, `parse_number` and `parse_range`. [transforms.md](transforms.md#language-2-transforms) has each one with its edge cases.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"slug": {"type": "string"}, "member_id": {"type": "string"}, "reaction": {"type": "string"}, "since": {"type": "string"}, "size_min": {"type": "integer"}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "org.enrich", "version": 1, "status": "draft",
    "fields": {
      "slug": {"from": "url", "transform": {"regex": "/company/([^/?#]+)"}},
      "member_id": {"from": "urn", "transform": {"strip_prefix": "urn:acme:member:"}},
      "reaction": {"from": "reaction", "transform": {"lookup": {"table": {"LIKE": "liked"}, "else": "keep"}}},
      "since": {"from": "since", "transform": {"parse_date": {"formats": ["%B %Y"]}}},
      "size_min": {"from": "size", "transform": {"parse_range": "min"}}}},
  "body": {"url": "https://x.example/company/widget-works/", "urn": "urn:acme:member:ABC", "reaction": "LIKE", "since": "June 2014", "size": "51-200"},
  "expected": {"slug": "widget-works", "member_id": "ABC", "reaction": "liked", "since": "2014-06", "size_min": 51}
}
```

### Container transforms

In language 2, a custom transform can build a whole array or object. The field must be an array or object node (an `x-open` map counts), and the rule must be `{"from": <path>, "transform": "<name>"}` with one custom transform. The function gets the source value as a dict or list and returns a list or a dict.

pycanonic checks the result against the canonical node:

- A leaf takes any value that is not a list or a dict.
- An array takes a list whose items fit the item node.
- An object takes a dict with no keys outside the node. A key that is missing or null gets its `x-empty` form, and the keys come out in schema order.
- An `x-open` map takes any dict.

A result that does not fit gives null and counts an error, so a wrong shape never reaches the output. A transform that raises does the same. A missing or null source gives null without a call. List options with a container transform fail with `a container transform takes only from and transform`. In language 1, a custom transform on an array runs once per element, as before.

```python
def by_team(people):
    teams = {}
    for p in people:
        teams.setdefault(p["team"], []).append(p["name"])
    return [{"team": t, "members": m} for t, m in teams.items()]

art = pycanonic.compile_spec(spec, schema, transforms={"by_team": by_team})
# spec: "teams[]": {"from": "staff", "transform": "by_team"}
```

Use a container transform only for logic that is not mapping. `group_by` covers most of the cases that used to need one.

### Definitions

The top-level `"definitions"` key names field maps, and `"item": {"$ref": "<name>"}` uses one. A definition is a field map relative to the element, with the same rules as an `item` map. One use is a batch endpoint that returns a list of the objects a single endpoint returns. Definitions can refer to other definitions.

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"teams": {"type": "array", "items": {"type": "object", "properties": {
    "name": {"type": "string"},
    "members": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}, "city": {"type": "string"}}}}}}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "team.batch", "version": 1, "status": "draft",
    "definitions": {"team": {"name": "title", "members[]": {"from": "staff", "item": {"$ref": "person"}}}, "person": {"name": "n", "city": "c"}},
    "fields": {"teams[]": {"from": "teams", "item": {"$ref": "team"}}}},
  "body": {"teams": [{"title": "core", "staff": [{"n": "Ada", "c": "Oslo"}, {"n": "Bo"}]}]},
  "expected": {"teams": [{"name": "core", "members": [{"name": "Ada", "city": "Oslo"}, {"name": "Bo", "city": null}]}]}
}
```

`$ref` must be the only key of the `item` object. These fail the compile:

```
acme people v1: definitions: no definition "x"
acme people v1: definitions: cycle through "a" -> "b" -> "a"
invalid spec: spec.definitions["person"]: must be a non-empty object
```

A definition cannot read `$group` at its top level, because a definition does not know whether its user is grouped. A grouped array rule inside a definition still works.

### Page envelopes

A page envelope such as `{total, page, size, items}` needs no new rule. Build it from an aggregate, `$input` and an array rule:

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {
    "total": {"type": "number"}, "page": {"type": "number"}, "size": {"type": "number"},
    "items": {"type": "array", "x-empty": "[]", "items": {"type": "object", "properties": {"name": {"type": "string"}}}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.search", "version": 1, "status": "draft",
    "fields": {
      "total": {"aggregate": "count", "from": "results"},
      "page": "$input.page",
      "size": "$input.size",
      "items[]": {"from": "results", "item": {"name": "full_name"}}}},
  "body": {"results": [{"full_name": "Ada"}, {"full_name": "Bo"}]},
  "context": {"page": 1, "size": 25},
  "expected": {"total": 2, "page": 1, "size": 25, "items": [{"name": "Ada"}, {"name": "Bo"}]}
}
```

pycanonic maps one body at a time. Merging several fetched pages into one response is the caller's job.

### Keys

Source keys match exactly, as in language 1. `"keys": "case_insensitive"` at the top level also matches keys without regard to case, for a provider whose key case drifts. Each step of a path reads the exact key first. If there is none, it reads the first key in document order that is equal in Unicode lowercase, so `"Émail"` matches `"émail"`.

The option acts on every body path: `from`, `coalesce`, `pluck`, list options, `group_by`, aggregates, `present`, `when` and `reject`. `$input` paths stay exact. A spec with `anchors` cannot use it (`spec.anchors: not allowed with "keys": "case_insensitive"`).

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"email": {"type": "string"}, "name": {"type": "string"}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "draft", "keys": "case_insensitive",
    "fields": {"email": "email", "name": "profile.name"}},
  "body": {"EMAIL": "upper@example.test", "email": "lower@example.test", "Profile": {"NAME": "Ada"}},
  "expected": {"email": "lower@example.test", "name": "Ada"}
}
```

| Body | Path `email` reads |
|---|---|
| `{"email": null, "Email": "x"}` | null, because the exact key wins, null or not |
| `{"Email": "a", "EMAIL": "b"}` | `"a"`, the first match in document order |
| no key in any case | missing |

A case-insensitive plan reads the whole body on every way in. See [known-limits.md](known-limits.md#case-insensitive-keys).

### Canonical schema in language 2

A schema used by a language 2 spec accepts shapes that the language 1 schema rejects:

| Keyword or shape | Effect |
|---|---|
| any string as a field name | `"first-name"`, `"@type"`, also `toJSON` and the other JavaScript prototype names. The spec writes them quoted: `["first-name"]`, `meta["@type"]`, `["@graph"][]`. |
| `"x-open": true` on an object node | a map with dynamic keys. A path rule on it copies the source object as it is. A spec cannot address its children. It needs an object node. |
| an array of arrays | a spec can copy it whole. `item` and `pluck` on it fail with `arrays of arrays can only be copied whole`. |
| `"type": [...]` | a node with more than one type, as `canonic schema` writes it for mixed samples |
| `"x-empty"` | the empty form of the node, see [Empty values](#empty-values) |
| `"x-mixed-empty": true` | `canonic schema` writes it for samples that disagree on the empty form. `canonic fit` reads it, and the compiler ignores it. |
| `{"type": "array", "items": ...}` at the root | an array root, see below |

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {"first-name": {"type": "string"}, "meta": {"type": "object", "properties": {"@type": {"type": "string"}}}, "attrs": {"type": "object", "x-open": true}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "draft",
    "fields": {"[\"first-name\"]": "given", "meta[\"@type\"]": "kind", "attrs": "labels"}},
  "body": {"given": "Ada", "kind": "Person", "labels": {"team": "core", "floor": 2}},
  "expected": {"first-name": "Ada", "meta": {"@type": "Person"}, "attrs": {"team": "core", "floor": 2}}
}
```

A language 1 schema keeps every language 1 rule. `x-open` and `x-empty` have no effect there, and `toJSON` still fails.

### Array roots

In language 2, the canonical schema's root can be an array: `{"type": "array", "items": {...}}`. The spec then has exactly one field, `"[]"`. It is an array rule that reads from `"$"` (a body that is a bare array) or from a path. `map`, `map_json` and the CLI return a list. Any other field key fails with `fields: an array root needs exactly the field "[]"`.

<!-- run -->
```json
{
  "schema": {"type": "array", "x-empty": "[]", "items": {"type": "object", "properties": {"name": {"type": "string"}}}},
  "spec": {"language": 2, "provider": "acme", "endpoint": "person.batch", "version": 1, "status": "draft",
    "fields": {"[]": {"from": "data.items", "item": {"name": "n"}}}},
  "body": {"data": {"items": [{"n": "Ada"}, {"n": "Bo"}]}},
  "expected": [{"name": "Ada"}, {"name": "Bo"}]
}
```

The root's `x-empty` decides what a missing source gives: null (the default) or `[]`. `"missing"` fails, because the body is always there: `canonical schema: an array root can't have x-empty "missing"`. Every list option and `reject` work on an array root. A language 1 schema still needs an object root.

| Source | No `x-empty` | `"x-empty": "[]"` |
|---|---|---|
| path missing, or body not JSON | null | `[]` |
| null | null | `[]` |
| `[]` | `[]` | `[]` |
| an object, not an array | null, one error | null, one error |
