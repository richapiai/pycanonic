# Inference

`canonic infer` drafts a spec from your canonical schema and a handful of real responses from the provider. It reports how sure it is about each field. It runs offline, and nothing in it is on the request path. The Python command is a line-by-line port of the TypeScript one. On the shared fixtures it writes the same spec and the same report, byte for byte.

```
canonic infer --schema canonical/person.enrich.json --provider globex --endpoint person.enrich \
  --out globex.json samples/globex/
```

The draft goes to `--out` (or stdout) with `status: "draft"` and `version: 1`. The report goes to stderr.

## Samples

Give it 5 to 20 responses, as files or directories of `.json` files. Inference cannot find a field that is null in every sample. With fewer than 5 samples the command prints a warning and carries on:

```
only 3 samples: a field that happens to be null in all of them can't be found
```

A sample can be a bare array. The array is the source `$`, and the draft maps it with `"from": "$"`.

Pick samples that differ: people with and without a work email, companies with and without a website. Inference proposes `coalesce` only for a source that fills the gaps of another in the samples.

## Options

| Option | Effect |
|---|---|
| `--schema <file>` | canonical JSON Schema (required) |
| `--provider`, `--endpoint` | copied into the spec (required) |
| `--known <file>` | canonical output you already trust for some samples, keyed by sample file name: `{"3.json": {"email": "ada@globex3.com", ...}}` |
| `--learn <spec.json>` | learns synonyms from a spec that a person approved. Repeat it for several specs. |
| `--synonyms <file>` | extra name synonyms, `{"token": ["canonical", "tokens"]}`. An entry replaces a learned or built-in entry for the same token. |
| `--min-score <n>` | pairs that score below this stay unmapped. The default is 0.4. |
| `--ask <command>` | optional model pass for doubtful fields, see below |
| `--out <file>` | write the spec here instead of stdout |

`--learn` reads each approved mapping as evidence of which provider key means which canonical name. It learns a token only from a key with exactly one token it cannot explain. It also needs that neither the key nor its parent shares a word with the canonical name. So `emails.work` mapped to `email` does not teach it that "work" means email. The leave-one-provider-out evaluation below runs on synonyms learned this way.

Scores combine three signals:

- Name tokens. Camel, snake and kebab case are split, and abbreviations go through the synonym table, so `fname` meets `first_name`.
- The shape of the values. An email column passes the email validator whatever its key is called.
- With `--known`, whether the transformed values equal the known ones.

Known values outweigh the other two. They also pick transforms the samples cannot show alone, such as lowercasing an industry name.

Fields are assigned one-to-one with the Hungarian algorithm. A second pass adds `coalesce` backups. It adds `split_name` for first and last name from a full-name source, and `join` for a full name from first and last. It also adds array rules (`item`, `pluck`, or plain values). Transforms come from the observed values and the canonical `semantic` and `type`. An `email` semantic gets `["trim", "lower"]`. A string of digits for an `integer` field gets `{ "cast": "integer" }`. Padded strings get `trim`.

## The report

Here are eight fictional responses from a flat provider. The job title sits under a meaningless key `zq7`, and there is no seniority or founding date:

```
first_name                  match    0.91  "fname"
last_name                   match    0.91  "lname"
email                       match    0.87  {"from":"e_mail","transform":["trim","lower"]}
phone                       match    1.00  {"from":"tel","transform":"normalize_phone"}
title                       match    0.41  "jobs[0].job_title"  <- check
linkedin_url                match    1.00  {"from":"li_url","transform":"normalize_linkedin"}
location.city               match    0.81  "city"
location.country            match    0.81  "country_code"
company.name                match    0.91  "org_name"
company.domain              match    0.75  {"from":"org_url","transform":"to_domain"}
company.headcount           match    0.72  {"from":"num_employees","transform":{"cast":"integer"}}
company.industry            match    0.81  "sector"
updated_at                  match    0.72  {"from":"last_updated","transform":"parse_date"}
full_name                   join     0.60  {"from":["fname","lname"],"transform":"join"}
experiences[]               item     0.43  {"from":"jobs","item":{...}}  <- check
experiences[].title         match    0.91  "job_title"
experiences[].company       match    0.91  "employer"
experiences[].start         match    0.81  {"from":"start_date","transform":"parse_date"}
experiences[].end           match    0.81  {"from":"end_date","transform":"parse_date"}
skills[]                    values   0.67  {"from":"skill_list"}
seniority                   unmapped  best: id (0.35)
company.founded             unmapped  best: org_name (0.56)
anchor: email "e_mail":
anchor: phone "tel":
anchor: company.domain "org_url":
```

(The `item` map is shortened here. The command prints it in full.)

Each mapped line has four columns: the canonical field, how it was mapped, the confidence from 0 to 1, and the rule written to the spec. The "how" column is one of `match`, `coalesce`, `split`, `join`, `item`, `pluck`, `values` or `ask`. A line with confidence below 0.6 ends in `<- check`. Read those first. Here `title` took the title of the first past job, which is wrong, and inference said it was not sure.

An `unmapped` line names the best-scoring source and its score. It does so even for a source that went to another field or scored under `--min-score`. `company.founded` shows `org_name` because nothing better exists, not because it is a candidate. Array fields that stay unmapped have no best candidate.

`anchor` lines are the anchors written to the spec. Each is a routing field read from a single key whose token occurs exactly once in every raw sample.

## --ask: the optional model pass

`--ask <command>` sends each doubtful field to a shell command and uses the path it names. A doubtful field is a scalar canonical field that is unmapped, or mapped by `match` with confidence below 0.6. Array fields and `coalesce`, `split` and `join` rules are never sent.

```
canonic infer --schema canonical/person.enrich.json --provider globex --endpoint person.enrich \
  --ask "claude -p" --out globex.json samples/globex/
```

The command runs once per field, through the shell, with one JSON question on stdin:

```json
{
  "instruction": "Which source path holds this canonical field? Reply with exactly one path from sources, or null if none of them does.",
  "field": "title",
  "type": "string",
  "current": "jobs[0].job_title",
  "sources": [
    { "path": "id", "examples": ["g_0", "g_1", "g_2"] },
    { "path": "zq7", "examples": [" VP Engineering ", " Head of Data ", " Staff Engineer "] }
  ]
}
```

(`sources` is shortened.) `semantic` appears for a canonical field that has one. `current` appears for a field that inference already mapped. Each source carries up to 3 distinct non-null example values. `sources` holds the first 200 scalar paths found in the samples.

The reply is the last non-empty line of stdout, as a JSON string or a bare path. `null` or an empty reply means no answer. An answer is dropped for a path that is not one of `sources`, or that another field already reads. For a command that exits non-zero, the CLI prints `--ask failed for <field>: <stderr>` and skips that field. The rest of the run carries on. An empty `--ask ""` means no ask. A reply that is not valid UTF-8 is decoded with replacement characters, so it names no known path and the field is skipped.

With a command that answers `"zq7"` for `title`, the line becomes:

```
title                       ask      0.41  {"from":"zq7","transform":"trim"}  <- check
```

The transform (`trim` here) comes from the values, the same way as for any other match. An answered field keeps the confidence inference gave it. So a field matched below 0.6 still shows `<- check` and still goes to a person. An anchor on the field is dropped. The final spec is compiled once more before it is written.

## Review

Check a draft against fixtures before you use it. After a provider changes, compare new samples against old ones.

`canonic test` maps each fixture's payload through the spec and compares the fields listed in `expected`. It does not check fields left out of `expected`. It compares nested objects and list elements field by field, so a fixture can name one key of a list item and leave the rest. A fixture file is `{"payload": ..., "expected": ...}` or `{"cases": [...]}` of those. Any failed case makes it exit 1.

Two keys serve language 2 specs:

- `"context": {...}` in a case holds the request inputs that `$input` paths read.
- `"expected": {"$rejected": "<reason>"}` means the spec must reject the payload with that reason, as `Rejected.reason` gives it: `{"$rejected": "{\"missing\":\"data.name\"}"}`. `{"$rejected": null}` accepts any reason. A spec that maps such a payload fails the case, and so does a spec that rejects a payload whose `expected` is not `$rejected`.

```
$ canonic test --schema canonical/person.enrich.json globex.json fixtures/globex.json
3 cases, 0 failed
```

A failure names the file, the case index and the field:

```
globex.json#0 email: expected "ada.lovelace@acme0.com", got null
```

`canonic diff` compares two sets of samples from one provider, old and new:

```
$ canonic diff --schema canonical/person.enrich.json --spec globex.json --out globex.v2.json old/ new/
1 new response shape
- e_mail (was in 100%)
+ email_address (in 100%)
field email: filled 100% -> 0%
email                       {"from":"e_mail","transform":["trim","lower"]} -> {"from":"email_address","transform":["trim","lower"]}
```

- `- path` is a key path in at least 10% of the old samples and in none of the new ones. `+ path` is the reverse.
- `~ path 90% -> 40%` is a path whose fill rate moved by 25 points or more.
- With `--schema` and `--spec`, it also lists spec fields that lost more than a quarter of their fill. It counts only fields filled in at least 10% of the old samples. Fields inside a list's `item` map are included, measured per element (`field jobs[].title: filled 100% -> 0%`).
- It re-infers those fields alone from the new samples. For a field inside a list, it keeps the list's source and re-infers only that item field from the elements.
- It writes the next version with `status: "shadow"` to `--out`, or to stdout with the report on stderr.
- The re-inference takes `--known` (keyed by the new samples' file names), `--synonyms`, `--learn` and `--min-score`, as `canonic infer` does.
- A re-mapped field loses its old anchor. It gets a new one for a key that occurs once in every new sample.
- Pass `--transforms <module.py>` for a spec that uses your own transforms.
- A field it cannot re-map is reported as `no replacement found; needs a person`.

Drift makes it exit 1, and no drift exits 0, so it works as a CI check. Exit 2 means it cannot run: a missing directory, a spec that does not compile, an unknown flag. With no dropped spec field it writes no spec. So prefer `--out` to a shell redirect, which leaves an empty file. [drift-and-learning-loop.md](drift-and-learning-loop.md) covers what happens to a `shadow` spec next.

`canonic bench --schema <file> <spec> <samples>...` times `map` from parsed objects and from bytes, `map_json`, a view, and `extract` on your own samples, next to `json.loads`.

## Evaluation

`python/tests/eval/test_eval.py` measures inference leave-one-provider-out. Six synthetic providers (`providers.py`) describe the same people with different naming, nesting and value formats. The six styles are nested camelCase, flat snake_case, kebab-case, deep nesting, PascalCase and prefixed codes (`p_fn`, `c_dom`). Each ships with the spec a person writes for it.

For each provider, the test learns a synonym table from the other five reviewed specs only. It infers a spec from 12 samples. Then it maps 40 responses it never saw through both the draft and the reviewed spec. A correct field gives the same value from both specs on all 40.

The gates, per provider:

- routing fields (`email`, `phone`, `company.domain`) all correct,
- no confident-wrong field, which means wrong with confidence 0.6 or above,
- at least 95% of fields correct.

The current run passes all six at 100%: 18/18, 16/16, 17/17, 17/17, 17/17 and 15/15 fields. Three more providers in `unseen.py` were written after the heuristics were tuned. They are reported without a gate: 10/10, 13/14 and 11/11, routing 3/3 on each. These are the same numbers as the TypeScript run.

These are synthetic payloads. The repo does not measure accuracy on real provider responses. To see the tables:

```
.venv/bin/pytest python/tests/eval -q -s
```
