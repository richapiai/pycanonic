"""Decision tiers: auto, review or manual for one fitted field."""

from __future__ import annotations

from typing import Literal

from .leaves import LeafFit

Tier = Literal["auto", "review", "manual"]

# The kinds that can be auto. Every other kind with a rule goes to review: the design's always-review
# list (lookup, const, regex, group, omit, when, reject, null_if, keys, absent) and the kinds the safe
# list does not name (wrap, flatten, filter, presence, if_empty, split and the other transforms).
AUTO = {"path", "cast", "trim", "lower", "upper", "format_date", "parse_date", "strip_prefix", "strip_suffix", "format", "join", "coalesce"}
MANUAL = {"none", "disagree", "union"}
# A person decided these (kept, asked) or must decide them (absent, reject), whatever the support.
REVIEW = {"absent", "kept", "asked", "reject"}


def tier(fit: LeafFit, *, min_auto: float = 0.95, min_pairs: int = 5, floor: float = 0.80) -> Tier:
    """Auto needs a safe kind, `min_pairs` pairs with a value or more, the support share and the
    holdout at `min_auto` or more, and default samples that agree on the empty form (no
    `mixed_empty`), and two or more distinct values in the default (no `one_value`: a path that
    gives one value tells nothing that a constant does not). Below `floor` support, or with no
    rule, it is manual. A field that needed a tolerance on any pair is review at most. An `empty`
    field (the default never sent a value) is review, or manual when the spec does not write the
    default's empty form and the default's samples agree on that form."""
    if fit.kind in MANUAL:
        return "manual"
    if fit.kind == "empty":  # the default sent no value, so no sample shows what a value maps to
        return "manual" if not fit.mixed_empty and fit.total and fit.support < floor * fit.total else "review"
    if fit.kind in REVIEW:
        return "review"
    if fit.total and fit.support < floor * fit.total:
        return "manual"
    if fit.kind in AUTO and not fit.mixed_empty and not fit.one_value and fit.total >= min_pairs and fit.support >= min_auto * fit.total and fit.holdout >= min_auto and not fit.tolerated:
        return "auto"
    return "review"
