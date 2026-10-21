"""The shared timing loop, ported from canonic's bench/measure.js. `canonic bench` and the repo
benchmarks (`bench/`) both use it. Every function takes an index in 0..63 and picks its own input,
or in 0..modulus-1 when `modulus` is given (`canonic bench` passes its sample count)."""

from __future__ import annotations

import time
from collections.abc import Callable
from typing import Any

_sink: Any = None  # keeps the last result alive


def _indices(n: int, modulus: int | None) -> list[int]:
    # Built before timing, so the loop pays for a list step rather than the index arithmetic.
    return [i & 63 for i in range(n)] if modulus is None else [i % modulus for i in range(n)]


def _batch_size(fn: Callable[[int], Any], batch_ns: int, modulus: int | None) -> int:
    global _sink
    n = 16
    while True:
        ix = _indices(n, modulus)
        t0 = time.perf_counter_ns()
        for i in ix:
            _sink = fn(i)
        if time.perf_counter_ns() - t0 >= batch_ns:
            return n
        n *= 2


def measure(
    fns: list[tuple[str, Callable[[int], Any]]],
    rounds: int = 21,
    batch_ns: int = 4_000_000,
    modulus: int | None = None,
) -> list[float]:
    """Median ns/op per function, sampled round-robin so drift hits every function alike."""
    global _sink
    sizes = []
    for _, fn in fns:
        _batch_size(fn, batch_ns, modulus)  # warm-up
        sizes.append(_batch_size(fn, batch_ns, modulus))
    ixs = [_indices(n, modulus) for n in sizes]
    samples: list[list[float]] = [[] for _ in fns]
    for r in range(rounds):
        for k in range(len(fns)):
            j = (k + r) % len(fns)
            fn, ix = fns[j][1], ixs[j]
            t0 = time.perf_counter_ns()
            for i in ix:
                _sink = fn(i)
            samples[j].append((time.perf_counter_ns() - t0) / len(ix))
    return [sorted(s)[len(s) // 2] for s in samples]


def fmt(ns: float) -> str:
    return f"{ns / 1000:.2f} µs" if ns >= 1000 else f"{ns:.0f} ns"
