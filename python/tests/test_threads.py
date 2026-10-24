import threading
import time

from conftest import fixture

from pycanonic import _core

THREADS, CALLS = 8, 10_000


def test_concurrent_maps_count_every_response():
    a = _core.compile(fixture("specs/bench.json"), fixture("specs/canonical/bench.json"), None)
    raws = [r.encode() for r in fixture("bench_bodies.json")["raws"]]

    # Each body's own counts, so the totals after the run are known exactly.
    per_body = []
    for r in raws:
        a.stats_reset()
        a.map(r)
        per_body.append(a.stats_snapshot()["fields"])
    a.stats_reset()
    want = {}
    for i in range(CALLS):
        for name, counts in per_body[i % len(raws)].items():
            slot = want.setdefault(name, dict.fromkeys(counts, 0))
            for k, n in counts.items():
                slot[k] += n * THREADS

    done = threading.Event()
    seen = []
    failures = []

    def work():
        for i in range(CALLS):
            a.map(raws[i % len(raws)])

    def watch():
        while not done.is_set():
            s = a.stats_snapshot()
            seen.append(s["responses"])
            # An assert here would only reach threading.excepthook. Collect the results and check them after join.
            failures.extend(n for n, f in s["fields"].items() if f["fill"] > f.get("total", s["responses"]))
            # A spinning Python thread holds the GIL for a whole switch interval at a time.
            time.sleep(0.001)

    workers = [threading.Thread(target=work) for _ in range(THREADS)]
    watcher = threading.Thread(target=watch)
    watcher.start()
    for t in workers:
        t.start()
    for t in workers:
        t.join()
    done.set()
    watcher.join()

    snap = a.stats_snapshot()
    assert snap["responses"] == THREADS * CALLS
    assert snap["fields"] == want
    assert seen and seen == sorted(seen)
    assert failures == []
