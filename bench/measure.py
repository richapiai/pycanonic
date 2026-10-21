"""The shared timing loop for the repo benchmarks; it lives in the package so `canonic bench` ships it."""

from pycanonic.cli.measure import fmt, measure

__all__ = ["fmt", "measure"]
