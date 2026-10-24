"""Decision tiers: auto, review or manual for one fitted field."""

from __future__ import annotations

from typing import Literal

from .leaves import LeafFit

Tier = Literal["auto", "review", "manual"]

# The kinds that can be auto. Every other kind with a rule goes to review: the design's always-review
# list (lookup, const, regex, group, omit, when, reject, null_if, keys, absent) and the kinds the safe
# list does not name (wrap, flatten, filter, presence, split and the other transforms).
AUTO = {"path", "cast", "trim", "lower", "upper", "format_date", "parse_date", "strip_prefix", "strip_suffix", "join", "coalesce", "empty"}
MANUAL = {"none", "disagree"}
# A person decided these (kept, asked) or must decide them (absent), whatever the support.
REVIEW = {"absent", "kept", "asked"}


def tier(fit: LeafFit, *, min_auto: float = 0.95, min_pairs: int = 5, floor: float = 0.80) -> Tier:
    """Auto needs a safe kind, `min_pairs` pairs with a value or more, and both the support share
    and the holdout at `min_auto` or more. Below `floor` support, or with no rule, it is manual."""
    if fit.kind in MANUAL:
        return "manual"
    if fit.kind in REVIEW:
        return "review"
    if fit.total and fit.support < floor * fit.total:
        return "manual"
    if fit.kind in AUTO and fit.total >= min_pairs and fit.support >= min_auto * fit.total and fit.holdout >= min_auto:
        return "auto"
    return "review"
