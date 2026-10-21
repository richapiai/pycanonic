# Drift and the learning loop

pycanonic gives you four tools to notice that a provider changed its responses. They also tell you whether a fixed spec is safe to promote. The four are `shape`/`fingerprint`, `FillMonitor`, `ShadowDiff` and `merge_gate`. They live in `crates/canonic-core/src/drift.rs` and `shadow.rs`. The scheduler, the bot that opens pull requests and the deployment that runs the shadow spec are yours to write. This page shows how the pieces fit.

A renamed key is the failure these tools exist for. Say a provider renames `workEmail` to `email`. The spec then reads a path that no longer exists, `map()` returns `None` for that field, and nothing raises. Without drift signals a rename looks like an ordinary miss.

## shape and fingerprint

```python
from pycanonic import shape, fingerprint

shape({"a": 1, "b": {"c": None, "d": [1]}})  # ['"a"', '"b"', '"b"."d"[]']
fingerprint({"a": 1})                        # "91ba3da0"
```

`shape(value)` returns the sorted key paths of a parsed response. It also takes the raw body as `bytes`. Keys are JSON-quoted, so a key that holds a dot cannot collide with nesting. A key whose value is null is left out.

An array is one path ending in `[]`, whatever its length. The keys of its object elements are merged under it (`"jobs"[]."role"`). So array lengths and element order do not change the shape, but a key renamed inside the elements does. An empty array has no element keys, so it shapes differently from a filled one, as a missing key does. A response that is itself an array starts at `[]` (`[]."name"`). Strings, numbers and null have an empty shape.

`fingerprint(value)` is a 32-bit FNV-1a hash of the shape as 8 hex digits. Two responses with the same shape have the same fingerprint. Key order does not matter. The fingerprint equals the TypeScript one for the same response. The one exception is a key with a lone surrogate, see [known-limits.md](known-limits.md#lone-surrogates-become-ufffd).

Keep the set of fingerprints you have seen per provider. Fingerprint a sample of responses, and treat a new fingerprint as a drift event.

## FillMonitor

`FillMonitor` compares each window of `artifact.stats` with the windows before it. It returns the fields whose fill rate fell or whose error rate rose.

```python
import time
from pycanonic import FillMonitor

monitor = FillMonitor()  # one per artifact

while True:
    time.sleep(60)
    window = artifact.stats.snapshot()
    artifact.stats.reset()
    for alert in monitor.check(window):
        on_drift(alert)
```

`artifact.stats.snapshot()` returns `{"responses": ..., "fields": {spec_field: {"fill": ..., "errors": ...}}}`. `map()` and `map_json()` count every response. `view`, `extract` and `stream` do not count. A field that falls back to its `default` counts as a miss, not a fill.

Fields inside an `item` map are counted per element and carry their own denominator:

```python
{
    "responses": 50,
    "fields": {
        "jobs[]": {"fill": 50, "errors": 0},                  # 50 of 50 responses had the array
        "jobs[].title": {"fill": 0, "errors": 0, "total": 500},  # 0 of 500 elements had a title
    },
}
```

`total` is the number of elements `map()` saw in that array. Say a provider renames `role` inside its `jobs` elements. Then `jobs[]` stays full, `jobs[].title` drops to zero, and `FillMonitor` alerts on it. An item field is judged in a window only once `total` reaches `min_responses`. The alert's `responses` is then that element count. Plucked arrays have one entry, for the array.

Each alert is a `FieldAlert(field, kind, rate, baseline, responses)`, where `kind` is `"fill"` or `"errors"`. The test in `python/tests/test_drift.py` renames `workEmail` after five windows of 300 responses with email filled in 90%. It gets:

```python
FieldAlert(field="email", kind="fill", rate=0.0, baseline=0.9, responses=300)
```

| Option | Default | Meaning |
|---|---|---|
| `min_responses` | 200 | a window with fewer responses is not judged. It is merged into the next one. |
| `history` | 24 | how many judged windows the baseline averages over |
| `sigmas` | 4 | how far past the baseline a rate must move, in standard errors |
| `min_drop` | 0.25 | the monitor ignores fill drops smaller than this fraction of the baseline |
| `min_rise` | 0.01 | the monitor ignores error-rate rises smaller than this, in absolute terms |

A fill alert needs the drop to exceed both `sigmas` standard errors and `min_drop` of the baseline. An error alert needs the rise to exceed both `sigmas` standard errors and `min_rise`. The standard error comes from the baseline rate, with add-one smoothing. So a field at 0% or 100% still has some spread. The first window only sets the baseline.

A field that alerts keeps its old baseline, and the bad window does not go into its history. The alert repeats every window until the spec is fixed. It does not fade as the bad rate becomes normal.

The monitor holds its state in memory. For several processes that serve the same artifact, add their snapshots field by field (`total` included) before you call `check`.

## ShadowDiff

`ShadowDiff` compares a candidate spec with the promoted one on the same responses.

```python
from pycanonic import ShadowDiff

diff = ShadowDiff(promoted.routing)

# for each response while the candidate is in shadow
diff.add(promoted.map(body), candidate.map(body))

report = diff.snapshot()
# {"responses": ..., "routing": [...], "fields": {"email": {"compared", "same", "improvements", "regressions", "conflicts"}, ...}}
```

Every canonical leaf is classified on every response, keyed by canonical path (`company.domain`, `experiences`). An array is one leaf. Inside it, key order does not count, and a key that is null equals a key that is missing. So `[{"title": "CTO"}]` and `[{"org": None, "title": "CTO"}]` are the same. That lets one side be output that `map()` did not build, such as the hand-written code a spec replaces.

A list is judged the way a scalar is. Say the candidate's list has the same length and only fills fields that the promoted list left null. That is an improvement, which is what a fixed rename inside list items looks like. The reverse is a regression. A changed value or a different length is a conflict.

- A response where both sides are null is not counted at all. Counting it lets a rarely filled field that the candidate dropped hide behind a 99% match.
- Improvement: promoted null, candidate has a value. A fixed rename looks like this.
- Regression: promoted has a value, candidate null.
- Conflict: both have values and they differ.
- Same: both have values and they are equal.

`compared` is the number of responses where at least one side had a value.

## merge_gate

`merge_gate(report, ...)` decides whether a candidate can go to promoted without a person. It returns `GateResult(merge, reasons)`. `merge` is `True` only for an empty `reasons`.

The rules:

- A report with no responses never merges.
- Routing fields: zero regressions and zero conflicts. Improvements on a routing field need at least one `known` entity that states that field's value in `expected`.
- Known entities: for each one, every routing field in `expected` must equal the candidate's value.
- Every other field: regressions plus conflicts at most `max_change` of `compared`. Improvements never count against it.
- Every alert in `fill_alerts` blocks the merge.

| Option | Default | Meaning |
|---|---|---|
| `max_change` | 0.01 | share of compared responses a non-routing field can regress or conflict on |
| `known` | none | `(candidate, expected)` pairs: the candidate's `map()` output for entities whose true routing values you know, next to those values in canonical shape (`{"company": {"domain": "globex.com"}}`) |
| `fill_alerts` | none | alerts a `FillMonitor` raised on the candidate's stats during the soak |

The known-entity check exists because validators check format only. Say a candidate maps a personal address into `email`. It passes the email validator and shows up as improvements. A known entity with the right work address catches it.

Output from a candidate that fixes a renamed email key, on 8 responses:

```python
merge_gate(report)
# GateResult(merge=False, reasons=['email: 8 routing improvements with no known-entity check'])

merge_gate(report, known=known)
# GateResult(merge=True, reasons=[])

merge_gate(report, known=[(candidate.map(body), {"email": "someone@else.com"})])
# GateResult(merge=False, reasons=['known entity 0: email is "ada.lovelace@globex0.com", expected "someone@else.com"'])
```

## Wiring the loop

```
drift event ─▶ re-infer from fresh samples ─▶ PR with status "shadow" ─▶ deploy
     ▲                                                                      │
     │              shadow runs next to the promoted spec, every field diffed
     │                                                                      ▼
     └──── gate fails: PR waits for a person        merge_gate passes: status "promoted", merge
```

1. Drift event. A `FillMonitor` alert, or a fingerprint you have not seen for this provider.
2. Re-infer. Collect 5 to 20 fresh responses and run `canonic diff --schema <canonical> --spec <promoted spec> --out next.json old/ new/`. Add `--transforms`, `--synonyms` or `--learn` for private transforms or a private dictionary. It re-maps only the fields whose fill dropped and writes the next version with `status: "shadow"`. See [inference.md](inference.md#review).
3. Pull request. Your bot commits `next.json` and opens a PR. CI runs `canonic test` against the provider's fixtures.
4. Deploy in shadow. `registry.put(provider, endpoint, version, artifact)` stores a `shadow` artifact next to the promoted one. `registry.get` still returns the promoted artifact, and `registry.get_shadow` returns the candidate. For each response, serve `promoted.map(body)`. Pass it with `candidate.map(body)` to `ShadowDiff.add`. Log the candidate's output and never return it.
5. Soak. Feed the candidate's stats windows to a `FillMonitor`. A new monitor has no baseline. To judge the candidate against the promoted history, replay the promoted windows you kept into a fresh monitor first.
6. Gate. At the end of the soak, call `merge_gate(diff.snapshot(), known=known, fill_alerts=alerts)`. For `merge=True`, the bot changes `status` to `promoted` and merges. A promoted artifact with the same version as the current shadow removes it from the shadow slot. For `merge=False`, the PR stays open with `reasons` in its description.

`Registry.put` swaps a reference. A request that already holds the old artifact finishes on it.
