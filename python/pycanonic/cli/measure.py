"""The shared timing loop, ported from canonic's bench/measure.js. `canonic bench` and the repo
benchmarks (`bench/`) both use it. Every function takes an index in 0..63 and picks its own input."""

from __future__ import annotations

import time
from collections.abc import Callable
from typing import Any

_sink: Any = None  # keeps the last result alive


def _batch_size(fn: Callable[[int], Any], batch_ns: int) -> int:
    global _sink
    n = 16
    while True:
        t0 = time.perf_counter_ns()
        for i in range(n):
            _sink = fn(i & 63)
        if time.perf_counter_ns() - t0 >= batch_ns:
            return n
        n *= 2


def measure(fns: list[tuple[str, Callable[[int], Any]]], rounds: int = 21, batch_ns: int = 4_000_000) -> list[float]:
    """Median ns/op per function, sampled round-robin so drift hits every function alike."""
    global _sink
    sizes = []
    for _, fn in fns:
        _batch_size(fn, batch_ns)  # warm-up
        sizes.append(_batch_size(fn, batch_ns))
    samples: list[list[float]] = [[] for _ in fns]
    for r in range(rounds):
        for k in range(len(fns)):
            j = (k + r) % len(fns)
            fn, n = fns[j][1], sizes[j]
            t0 = time.perf_counter_ns()
            for i in range(n):
                _sink = fn(i & 63)
            samples[j].append((time.perf_counter_ns() - t0) / n)
    return [sorted(s)[len(s) // 2] for s in samples]


def fmt(ns: float) -> str:
    return f"{ns / 1000:.2f} µs" if ns >= 1000 else f"{ns:.0f} ns"
