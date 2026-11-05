# fit evaluation results

Each provider is fitted on 15 pairs (seed 1). Then 60 fresh pairs (seed 2) are mapped through the spec that fit wrote.
An auto field is wrong if its value differs from the default's on a fresh pair. The suite fails on any wrong auto field.
The auto share is reported, not gated. The target is 80%.

| provider | fields | auto | review | manual | auto share | wrong auto | fresh pairs rejected |
|---|---|---|---|---|---|---|---|
| quillon | 36 | 29 | 7 | 0 | 81% | 0 | 0 |
| brask | 36 | 29 | 7 | 0 | 81% | 0 | 0 |
| tavio | 36 | 29 | 7 | 0 | 81% | 0 | 0 |
| merrow | 36 | 26 | 10 | 0 | 72% | 0 | 0 |
| zentel | 36 | 26 | 10 | 0 | 72% | 0 | 0 |
| obrik | 38 | 29 | 9 | 0 | 76% | 0 | 0 |
| fennix | 34 | 22 | 7 | 5 | 65% | 0 | 0 |
| lumari | 36 | 29 | 7 | 0 | 81% | 0 | 0 |
| corvane | 34 | 27 | 7 | 0 | 79% | 0 | 0 |
| pellis | 7 | 6 | 1 | 0 | 86% | 0 | 0 |
| drovo | 31 | 19 | 11 | 1 | 61% | 0 | 0 |
| kestrin | 36 | 26 | 8 | 2 | 72% | 0 | 0 |
| tessaly | 36 | 28 | 8 | 0 | 78% | 0 | 0 |
| total | 432 | 325 | 99 | 8 | 75% | 0 | 0 |

Most review rows are review by design: optional objects (`presence`, `if_empty`), fields where the default sent one value or none in 15 pairs, learned lookups, groups, and case-insensitive keys.
Manual rows are sections that a provider does not send in the same form, and values that disagree.
