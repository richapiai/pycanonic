# Onboarding a provider: canonic schema, fit and verify

Say one endpoint of your API already has a default provider, and its responses are the contract your customers see. To add a second provider, you need a spec that turns the new provider's responses into the default's shape. Three commands write and check that spec from paired samples:

1. `canonic schema` derives the canonical schema from the default provider's responses.
2. `canonic fit` learns a language 2 spec from the pairs and sorts every field into auto, review or manual.
3. `canonic verify` maps every pair through the spec and fails below a threshold, so CI can gate the provider.

A pair is one request sent to both providers: the default's response and the new provider's response, under the same file name. pycanonic never calls a provider. Collecting the pairs, with keys, retries and pagination, is your job.

The specs that `fit` writes use [language 2](spec-format.md#language-2), so only pycanonic can run them.

## The pairs

```
samples/
  default/   p01.json p02.json ...   the default provider's responses
  new/       p01.json p02.json ...   the new provider's responses, same file names
  inputs/    p01.json p02.json ...   optional: the request inputs of each pair, for $input
```

Files pair by name. `fit` skips a file without a partner, a body that is not JSON, and a body that is a JSON scalar. The report gives the reason for each. A body that is a bare array is a body like any other, and paths read it from `$`. An inputs file must be a JSON object, or the pair is skipped with `inputs not JSON` or `inputs not an object`.

Use 10 to 30 pairs that differ: people with and without a work email, companies with and without an office, short and long work histories. `fit` warns below 8 pairs and stops below 3.

## canonic schema

```
canonic schema samples/default/ --out canonical.json
```

It reads the default provider's responses (files, or folders of `.json` files) and writes a language 2 canonical schema. For each node it records:

- the JSON types seen, with a list for samples that disagree,
- the key order, in order of first appearance across the samples,
- the form an empty value takes, as `"x-empty"`: `"null"`, `"[]"`, `"{}"` or `"missing"` (the key is left out),
- `"x-open": true` for a map of dynamic keys. That needs more than 20 distinct keys, and on average each sample carries fewer than half of them.

If the samples disagree on the empty form, the schema takes the most frequent form that the node allows and adds `"x-mixed-empty": true`. A tie goes to the first of null, `[]`, `{}` and missing. The fields to check go to stderr: mixed types, mixed empty forms, and fields that are null in every sample.

`fit` and `verify` reproduce exactly what `x-empty` says, so check those lines before you go on. A field that is null in every sample has no type and no value to learn from.

## canonic fit

```
canonic fit --default samples/default/ --provider samples/new/ --inputs samples/inputs/ \
  --schema canonical.json --provider-name globex --endpoint person.enrich \
  --out globex.json --report report
```

| Flag | Meaning |
|---|---|
| `--schema <file>` | the canonical schema. Without it, `fit` derives one from the `--default` samples. |
| `--inputs <dir>` | request inputs per pair, for fields that depend on them |
| `--keep <file>` | reviewed rules to keep as written, see [The manual path](#the-manual-path) |
| `--ask <command>` | asks a shell command about the manual fields, as `canonic infer --ask` does |
| `--report <file>` | writes the report to `<file>.md` and `<file>.json`. Without it, the Markdown report goes to stdout. |
| `--transforms-out <file>` | writes a Python stub for each manual field that needs code |
| `--min-auto <f>` | the support and holdout share an auto field needs, default `0.95` |
| `--min-pairs <n>` | the pairs with a value an auto field needs, default `5` |
| `--strict` | a field to review also exits 2 |

The spec has `"status": "draft"`. Exit codes:

| Exit | Meaning |
|---|---|
| 0 | every field is auto |
| 1 | one or more fields need review. Fields kept with `--keep` do not count. |
| 2 | one or more fields are manual, or a pair was skipped, or `--strict` and a field needs review |

### How fit decides

`fit` looks at values first and names second. For each canonical leaf, it reads the default's value in every pair. Then it searches the new provider's body in the same pair for a path and a transform chain that give that value.

1. It indexes every scalar in the provider's body by path.
2. It tries chains in order of simplicity: no transform, then one transform, then two. It learns the arguments from the data: a prefix, a date layout, a join separator, a regex, a lookup table.
3. It scores a candidate by the share of pairs that it reproduces exactly, out of the pairs where the default has a value. Ties go to the simpler chain, then to name similarity, then to the shorter path.
4. Arrays: it aligns the default's array with a provider array by their element values. It tries a plain path, `wrap`, a flatten path, `group_by` and `filter`, in that order.
5. Structure: an optional object (`present`) for an object that is null in exactly the pairs where a provider path is missing. A constant where the default's value never changes and nothing in the provider gives it. An empty form from the schema's `x-empty`.
6. Inputs: some fields appear only in pairs whose inputs meet a condition. Such a field gets a `when`. If the default leaves the key out in the other pairs, it also gets `omit`.
7. Rejects: it proposes a `reject` rule for a leaf that the default always fills and the provider can leave missing.
8. Holdout: it repeats the search with each pair left out in turn. Then it reports how often the rule still holds on the pair it did not see. For a field inside an array, it leaves out the whole pair with all of its elements, because elements of one response share their structure.

Before `fit` writes the spec, it compiles the spec and maps every pair through it, rejects included. A field whose runtime output differs from what the search reported becomes manual, with a note. So `fit` never writes a rule that it has not run.

Sometimes one provider key is spelled in more than one case across the pairs, such as `Email` in one body and `email` in another. Then `fit` adds `"keys": "case_insensitive"` to the spec as a review row.

### Rejects in fit

A proposed reject goes into the spec as a review row. A pair that the spec rejects counts as a failover. The reject row's note names each one, for example `Rejects 1 of 15 pairs: rejects p07.json, where the default answered.`. Rejected pairs are left out of every field's support and total. If the search and the runtime disagree on a reject, `fit` drops the reject rules and marks them manual.

## Tiers

| Tier | Meaning | Criteria (defaults) |
|---|---|---|
| auto | `fit` decided, and a reviewer can skip the field | meets every auto rule below |
| review | `fit` decided, and a person must check the rule | has a rule at 80% or more, but misses an auto rule |
| manual | `fit` found no rule | no candidate at 80% or more, values that disagree, or a value that needs code |

An auto field meets all of these rules. The numbers are the defaults of `--min-auto` and `--min-pairs`.

- Its kind is on the auto list below.
- It is exact on 95% or more of the pairs where the default has a value.
- It has 5 such pairs or more.
- Its holdout is 95% or more.
- The default sends two or more distinct values. One value tells nothing that a constant does not.
- The default's samples agree on the empty form.

These kinds can be auto:

- an exact path, `coalesce` and `join`
- `cast`, `trim`, `lower` and `upper`
- `format_date`, `parse_date`, `strip_prefix` and `strip_suffix`
- an empty form taken from the schema

Every other kind goes to review:

- `lookup`, `const`, `regex` and the other transforms
- `group`, `wrap`, `flatten` and `filter`
- `presence`, `if_empty`, `when` (with or without `omit`), `reject` and `keys`
- fields the provider never sends, answers from `--ask`, and rules kept with `--keep`

A two-step chain takes the kind of the step that is not on the auto list.

## What is decided automatically, and what can break

This is the table the reviewer reads. The report prints the row of each field that is not auto, under "what can break". The texts below are the texts `fit` prints.

| Decision | Kind in the report | Tier | What can break on live traffic |
|---|---|---|---|
| Field to path, exact value | `path` | auto | The provider renames a key later. Caught by drift detection and the fingerprint, not by `fit`. |
| Cast, trim, case, date layout | `cast`, `trim`, `lower`, `upper`, `format_date`, `parse_date` | auto | A value format that the samples did not show, such as a date without a month. The transform returns null and counts an error. Watch the error counts after launch. |
| Other one-step transforms | `split`, `title`, `to_string`, `parse_number`, `parse_range` | review | The same text as the row above. |
| Prefix strip | `strip_prefix` | auto | A record with a different prefix keeps it, so the value comes out with that prefix still on. |
| Suffix strip | `strip_suffix` | auto | A record with a different suffix keeps it, so the value comes out with that suffix still on. |
| Empty array or null, from the default's samples | `empty` | auto (review for samples that disagree) | The default provider itself is inconsistent (sometimes null, sometimes `[]`). The schema flags fields where samples disagree, and those fields go to review. |
| Coalesce order | `coalesce` | auto | The second source has different meaning on records the samples did not cover. |
| Join separator | `join` | auto | A record with one part missing, or with a different separator, maps to a wrong or partial value. |
| Learned lookup table | `lookup` | review | A value outside the table. `"else"` decides: keep (wrong vocabulary passes through), null (data lost), or a fixed value. |
| Constant | `const` | review | The default's value only looked constant in the samples. The most common cause of silent wrong data. Add pairs that differ. |
| Learned regex | `regex` | review | A URL or string format the samples did not show. |
| Group by | `group` | review | The grouping key is wrong for records with gaps or repeats, for example two stints at one company with another job between them. Check with a pair that has that pattern. |
| Optional object | `presence` | review | If the provider sends the parent object with different content, the object can be null by mistake. |
| Empty form of an object the default always sent | `if_empty` | review | The default sent this object in every sample, so its empty form is a guess. If every leaf in it is null, the object is null. If the default sends {} or an object of nulls there, the output differs. |
| Condition on a request input | `when` | review | An input combination the samples did not show. |
| Omit a key | `omit` | review | Customers parsing a fixed key set. |
| Reject rule | `reject` | review | Too strict: good answers fail over to the next provider and cost money. Too loose: empty answers reach customers. |
| Case-insensitive keys | `keys` | review | Two keys that differ only in case: the exact key wins, else the first key in the body that matches without regard to case. |
| A scalar wrapped in an array | `wrap` | review | A provider that sometimes sends a list here gets a list inside a list. |
| An array collected across nested arrays | `flatten` | review | Elements from inner arrays that the samples did not show get mixed in. |
| Elements kept by a condition | `filter` | review | A status value that the samples did not show drops the element, or keeps it. |
| A marker value turned into null | `null_if` | review | A real value that equals the null marker is lost. |
| Provider never sends the field | `absent` | review | Customers that filter on that field get nulls. A business decision, not a mapping one. |
| Rule from `--ask` | `asked` | review | A model chose this source path. Check it against the examples. |
| Rule kept with `--keep` | `kept` (the `report.md` heading says `kept by reviewer`) | review | kept by reviewer |
| Values disagree between providers | `disagree` | manual | A value mismatch that a lookup table cannot fix, for example old and new industry names. Decide per field: lookup table, custom transform, or accept the difference. |
| Needs code | `none` | manual | Covered by the generated stub and its doctests. |

Notes on the table:

- `omit` does not get its own row. It is part of a `when` row. `fit` never proposes `null_if`. The `omit` and `null_if` texts are for rules a person writes.
- A field can also print a note in front of the text, such as the closest rule for a `disagree` field or `The default's samples disagree on the empty form.`

No tool here catches these problems:

- A provider that is right on the samples and wrong on other inputs. The only fix is more pairs, and more varied pairs.
- A provider that changes its format after launch. Drift detection and shadow comparison catch this. See [drift-and-learning-loop.md](drift-and-learning-loop.md).
- Fields that the default provider gets wrong too. `fit` copies the default's behavior, errors included.

## The report

`report.md` has one line per auto field, in a table. Each review and manual field gets a section with the rule and the evidence (k of n pairs, and the holdout). The section also has up to three examples, and the "what can break" text. An example puts the default's value next to the value the rule maps (`mapped`). A field with no rule shows the provider's own value instead (`provider`). `report.json` holds the same rows for tools: `field`, `tier`, `kind`, `rule`, `support`, `total`, `holdout`, `examples` and `risk`, plus a `summary` and the `skipped` pairs. Rows that `fit` does not hold out (`kept`, `asked`, `presence`, `if_empty`, `keys` and `reject`) have `holdout` null, and `report.md` prints `n/a` for them.

## The manual path

A person can write or change any rule by hand. To keep a reviewed rule through the next fit, put it in a sidecar file:

```json
{"status": "state", "jobs[]": {"from": "positions", "group_by": "org", "item": {"org": "org", "roles[]": {"from": "$group", "item": {"title": "title"}}}}}
```

`canonic fit --keep reviewed.json` keeps each of those rules as written and refits the rest. The unit of a lock is one top-level key of `fields`. For an array field, that key holds the whole array rule, `item` map and options included. A kept rule has kind `kept` in `report.json` and the heading `kept by reviewer` in `report.md`. It is in the review tier, and `fit` never downgrades it.

`fit` writes a new spec each time. A hand edit that is not in the sidecar is lost on refit. For each field whose rule in the old `--out` file differs from the new one, `fit` prints `fit: <field> changed since the last spec; add it to --keep to hold it`. The spec holds no review data, so it stays a plain language 2 spec.

For a manual field that needs code, `--transforms-out transforms.py` writes one function stub per field, with the failing pairs as doctests. Fill it in, name it in the spec, and pass the module to `canonic verify --transforms`. `--ask <command>` sends the manual fields to a command, such as a model, as `canonic infer --ask` does. Its answers count as review, never auto.

## canonic verify

```
canonic verify globex.json --schema canonical.json --default samples/default/ --provider samples/new/ \
  [--inputs samples/inputs/] [--transforms transforms.py] [--min-match 0.95] [--max-reject 0.10] [--fail-unmapped] [--json]
```

It maps each provider body through the spec and compares the output with the default's body of the same name. It checks:

1. Shape, at every depth: the same keys in the same order, the same JSON types, and null against an object or an array, in both directions. An object of nulls where the default sends null fails as `null against object`. An `x-open` map can differ in keys. A null against a scalar is a value miss, not a shape failure.
2. Values: for each field the spec maps, the share of pairs that match. Numbers compare by value (`1` equals `1.0`). Everything else compares as JSON, without regard to key order. A pair where both sides are null counts neither way, so a field's total counts only pairs where one side has a value. Fields that the spec does not map are left out of the value check.
3. Rejects: every rejected pair is listed. A rejected pair fails the run in one case: the spec without its reject rules maps that pair to exactly the default's output, so the reject threw away a correct answer. If the spec rejects more than `--max-reject` of the pairs (default 10%), the run fails too.
4. Skipped pairs: a pair on one side only, or a body that is not JSON, fails the run.
5. Leaves the spec does not map: each canonical leaf that the spec leaves out but the default fills is listed, for example `not mapped: status (12 pairs with a value)`. They do not fail the run, because the provider can lack a field on purpose. With `--fail-unmapped`, they do.

It prints a table, or JSON with `--json`. If every check passes, it exits 0. If a check fails, it exits 1. If a file cannot be read or there are no pairs, it exits 2. Run it in CI on every spec change, and again after each new recording of pairs.

## A worked example

`python/tests/fixtures/fit/acme-to-globex/` holds 10 invented pairs. The default provider, acme, sends a flat person:

```json
{"id": 1001, "name": "Ada Lane", "email": "ada@mail0.example.test", "joined": "2020-01-10", "status": "active",
 "hq": {"city": "Lyon"}, "tags": [], "jobs": [{"org": "Orbit", "roles": [{"title": "role00"}, {"title": "role01"}]}]}
```

The new provider, globex, sends the same person in another shape, with one flat list of positions:

```json
{"uid": "1001", "person": {"first": "Ada", "last": "Lane"}, "contact": {"email": "ada@mail0.example.test"},
 "since": "2020-01-10T10:00:00Z", "state": "active", "office": {"town": "Lyon", "floor": 2},
 "positions": [{"org": "Orbit", "title": "role00"}, {"org": "Orbit", "title": "role01"}]}
```

First the schema, then the fit:

```
$ F=python/tests/fixtures/fit/acme-to-globex
$ canonic schema $F/default --out canonical.json
$ canonic fit --default $F/default --provider $F/provider --provider-name globex --endpoint person.enrich \
    --schema canonical.json --out globex.json --report report
fit: 9 auto, 2 review, 1 manual
$ echo $?
2
```

The schema records `"x-empty": "null"` for `hq` and `"x-empty": "[]"` for `tags`, because that is what acme sends for an empty value. The auto part of `report.md`:

```
| field | kind | rule | pairs | holdout |
|---|---|---|---|---|
| id | cast | `{"from": "uid", "transform": [{"cast": "integer"}]}` | 10 of 10 | 100% |
| name | join | `{"from": ["person.first", "person.last"], "transform": [{"join": " "}]}` | 10 of 10 | 100% |
| email | path | `"contact.email"` | 10 of 10 | 100% |
| joined | format_date | `{"from": "since", "transform": [{"format_date": "date"}]}` | 10 of 10 | 100% |
| hq.city | path | `"office.town"` | 6 of 6 | 100% |
| tags[] | path | `{"from": "labels"}` | 10 of 10 | 100% |
| jobs[].org | path | `"org"` | 17 of 17 | 100% |
...
```

Two fields need review. `hq` is an optional object (`{"present": "office"}`), because acme sends null in the pairs where globex has no office. `jobs[]` groups the positions by `org`, which needs a person to check pairs with gaps and repeats. One field is manual:

```
## status: manual, disagree

- rule: `null`
- evidence: 0 of 10 pairs, holdout 0%
- p03.json: default `"retired"`, provider `"inactive"`
- p05.json: default `"retired"`, provider `"inactive"`
- p08.json: default `"retired"`, provider `"inactive"`
- what can break: Closest rule: {"from": "state"} on 7 of 10 pairs. A value mismatch that a lookup table cannot fix, ...
```

globex has no `retired` state, so no lookup table can fix the field. Here the reviewer accepts the difference. They keep `"status": "state"` in `reviewed.json` and fit again:

```
$ echo '{"status": "state"}' > reviewed.json
$ canonic fit ... --keep reviewed.json --out globex.json --report report
fit: 9 auto, 3 review, 0 manual
$ canonic verify globex.json --schema canonical.json --default $F/default --provider $F/provider
id       10/10  1.00
name     10/10  1.00
email    10/10  1.00
joined   10/10  1.00
status   7/10  0.70
hq       6/6  1.00
hq.city  6/6  1.00
tags[]   10/10  1.00
jobs[]   10/10  1.00
verify: failed (10 pairs, 0 rejected)
```

`verify` fails, because `status` matches on 7 of 10 pairs and the bar is 95%. That is the decision written down: either the team accepts it with `--min-match 0.7` for this provider, or globex is not used where `retired` matters. The spec that `fit` wrote, as a runnable case on the first pair:

<!-- run -->
```json
{
  "schema": {"type": "object", "properties": {
    "id": {"type": "number"}, "name": {"type": "string"}, "email": {"type": "string"}, "joined": {"type": "string"}, "status": {"type": "string"},
    "hq": {"type": "object", "properties": {"city": {"type": "string"}}, "x-empty": "null"},
    "tags": {"type": "array", "items": {"type": "string"}, "x-empty": "[]"},
    "jobs": {"type": "array", "items": {"type": "object", "properties": {"org": {"type": "string"}, "roles": {"type": "array", "items": {"type": "object", "properties": {"title": {"type": "string"}}}}}}}}},
  "spec": {"language": 2, "provider": "globex", "endpoint": "person.enrich", "version": 1, "status": "draft",
    "fields": {
      "id": {"from": "uid", "transform": [{"cast": "integer"}]},
      "name": {"from": ["person.first", "person.last"], "transform": [{"join": " "}]},
      "email": "contact.email",
      "joined": {"from": "since", "transform": [{"format_date": "date"}]},
      "status": "state",
      "hq": {"present": "office"},
      "hq.city": "office.town",
      "tags[]": {"from": "labels"},
      "jobs[]": {"from": "positions", "group_by": "org", "consecutive": true, "item": {"org": "org", "roles[]": {"from": "$group", "item": {"title": "title"}}}}}},
  "body": {"uid": "1001", "person": {"first": "Ada", "last": "Lane"}, "contact": {"email": "ada@mail0.example.test"},
    "since": "2020-01-10T10:00:00Z", "state": "active", "office": {"town": "Lyon", "floor": 2},
    "positions": [{"org": "Orbit", "title": "role00"}, {"org": "Orbit", "title": "role01"}, {"org": "Nimbus", "title": "role02"}]},
  "expected": {"id": 1001, "name": "Ada Lane", "email": "ada@mail0.example.test", "joined": "2020-01-10", "status": "active",
    "hq": {"city": "Lyon"}, "tags": [],
    "jobs": [{"org": "Orbit", "roles": [{"title": "role00"}, {"title": "role01"}]}, {"org": "Nimbus", "roles": [{"title": "role02"}]}]}
}
```

## How well fit does

`python/tests/fit_eval/` is the evaluation suite. It has 13 invented provider pairs with known true specs. They include grouping with gaps and repeats, optional objects, dropped keys, renamed enums, nested pages, inconsistent empty values and key-case drift. `fit` learns each spec from 15 pairs. Then 60 fresh pairs go through the spec it wrote. An auto field that differs from the default on a fresh pair counts as wrong.

Results on 2026-10-24 (`python/tests/fit_eval/RESULTS.md`):

| | Fields | Auto | Review | Manual | Auto share | Wrong auto |
|---|---|---|---|---|---|---|
| 13 providers | 432 | 325 | 100 | 7 | 75% | 0 |

Zero wrong auto fields is a release gate, and the suite fails on any. The auto share is a target, not a gate: the target is 80%, and `fit` does not meet it yet. Most review rows are review by design: optional objects, fields where the default sent fewer than two values in 15 pairs, learned lookups, groups and case-insensitive keys.
