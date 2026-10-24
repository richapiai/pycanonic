"""The corpus comparison rule for the Python runner; README.md states it, crates/canonic-core/tests/common/mod.rs has the Rust twin.
The rule lives in pycanonic.cli.fit.compare, which `fit` and `verify` share."""

from pycanonic.cli.fit.compare import diff, fffd, same

__all__ = ["diff", "fffd", "same"]
