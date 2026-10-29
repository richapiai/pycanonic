"""Maximum-score one-to-one assignment (Hungarian algorithm with potentials, O(n^2 m))."""

from __future__ import annotations

import math
from collections.abc import Callable


def assign(score: list[list[float]]) -> list[int]:
    """score[i][j] for row i and column j. Returns, per row, the column it is assigned or -1.
    Rows get a column only when there are enough columns. Pairs with score 0 count as unassigned."""
    rows = len(score)
    cols = len(score[0]) if score else 0
    if rows == 0 or cols == 0:
        return [-1] * rows
    # The algorithm wants rows <= columns, so pad with zero-score columns.
    m = max(rows, cols)

    def cost(i: int, j: int) -> float:  # i and j are 1-based
        return -(score[i - 1][j - 1] if j <= cols else 0)

    u = [0.0] * (rows + 1)
    v = [0.0] * (m + 1)
    p = [0] * (m + 1)  # p[j]: row matched to column j (1-based), 0 = none
    way = [0] * (m + 1)
    for i in range(1, rows + 1):
        augment(i, m, cost, u, v, p, way)
    out = [-1] * rows
    for j in range(1, cols + 1):
        if p[j] > 0 and score[p[j] - 1][j - 1] > 0:
            out[p[j] - 1] = j - 1
    return out


# Positional state is the textbook form of this algorithm. It stays so a reader can check the code against the textbook.
def augment(i: int, m: int, cost: Callable[[int, int], float], u: list[float], v: list[float], p: list[int], way: list[int]) -> None:
    minv = [math.inf] * (m + 1)
    used = [False] * (m + 1)
    p[0] = i
    j0 = 0
    while True:
        used[j0] = True
        i0 = p[j0]
        delta = math.inf
        j1 = 0
        for j in range(1, m + 1):
            if used[j]:
                continue
            cur = cost(i0, j) - u[i0] - v[j]
            if cur < minv[j]:
                minv[j] = cur
                way[j] = j0
            if minv[j] < delta:
                delta = minv[j]
                j1 = j
        for j in range(m + 1):
            if used[j]:
                u[p[j]] += delta
                v[j] -= delta
            else:
                minv[j] -= delta
        j0 = j1
        if p[j0] == 0:
            break
    while True:
        j1 = way[j0]
        p[j0] = p[j1]
        j0 = j1
        if not j0:
            break
