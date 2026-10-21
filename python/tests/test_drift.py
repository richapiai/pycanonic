"""shape, fingerprint and FillMonitor through Python; pins computed with the TypeScript dist."""

from __future__ import annotations

import dataclasses
import json
from typing import Any

import pytest

from pycanonic import FieldAlert, FillMonitor, fingerprint, shape

# cd canonic && node -e 'import("./packages/canonic/dist/index.js").then(m=>{console.log(m.fingerprint({"a":1,"b":[{"c":2}]}), JSON.stringify(m.shape({"a":1,"b":[{"c":2}]})))})'
# 6a2ab0d3 ["\"a\"","\"b\"[]","\"b\"[].\"c\""]
SIMPLE = {"a": 1, "b": [{"c": 2}]}
# cd canonic && node -e 'import("./packages/canonic/dist/index.js").then(m=>{const d={"\u{1F600}":1,"\uFF01":{"x":[{"y":null,"z":2}]},"q\"\n":true};console.log(m.fingerprint(d), JSON.stringify(m.shape(d)))})'
# 2297bcc6 ["\"q\\\"\\n\"","\"😀\"","\"！\"","\"！\".\"x\"[]","\"！\".\"x\"[].\"z\""]
WIDE = {"\U0001f600": 1, "\uff01": {"x": [{"y": None, "z": 2}]}, 'q"\n': True}


def test_pinned_fingerprints_match_typescript() -> None:
    assert shape(SIMPLE) == ['"a"', '"b"[]', '"b"[]."c"']
    assert fingerprint(SIMPLE) == "6a2ab0d3"
    assert shape(WIDE) == ['"q\\"\\n"', '"\U0001f600"', '"\uff01"', '"\uff01"."x"[]', '"\uff01"."x"[]."z"']
    assert fingerprint(WIDE) == "2297bcc6"


@pytest.mark.parametrize("doc", [SIMPLE, WIDE, [{"a": 1}, {"b": None, "c": []}], 3, None])
def test_a_parsed_value_shapes_like_its_bytes_and_text(doc: Any) -> None:
    raw = json.dumps(doc)
    assert shape(doc) == shape(raw) == shape(raw.encode())
    assert fingerprint(doc) == fingerprint(raw) == fingerprint(raw.encode())


def test_a_str_is_json_text_and_scalars_have_an_empty_shape() -> None:
    assert shape('"text"') == shape(b"3") == shape(None) == []
    with pytest.raises(ValueError):
        shape("text")
    assert shape({"t": ({"a": 1},)}) == ['"t"[]', '"t"[]."a"']


def test_bad_input_raises() -> None:
    with pytest.raises(ValueError):
        shape(b"{nope")
    with pytest.raises(TypeError):
        shape({"a": object()})
    cyclic: dict[str, Any] = {}
    cyclic["a"] = cyclic
    with pytest.raises(ValueError):
        fingerprint(cyclic)


def snap(responses: int, fields: dict[str, tuple[int, int]]) -> dict[str, Any]:
    return {"responses": responses, "fields": {f: {"fill": a, "errors": e} for f, (a, e) in fields.items()}}


def test_a_fill_drop_alerts() -> None:
    m = FillMonitor()
    for _ in range(10):
        assert m.check(snap(2000, {"rare": (100, 0)})) == []
    alerts = m.check(snap(2000, {"rare": (0, 0)}))
    assert alerts == [FieldAlert(field="rare", kind="fill", rate=0.0, baseline=0.05, responses=2000)]
    with pytest.raises(dataclasses.FrozenInstanceError):
        alerts[0].rate = 1.0  # type: ignore[misc]


def test_an_error_rise_alerts() -> None:
    m = FillMonitor()
    for _ in range(10):
        m.check(snap(1000, {"date": (990, 1)}))
    assert m.check(snap(1000, {"date": (940, 50)})) == [
        FieldAlert(field="date", kind="errors", rate=0.05, baseline=0.001, responses=1000)
    ]


def test_small_windows_merge_until_min_responses() -> None:
    m = FillMonitor(min_responses=300)
    for _ in range(5):
        assert m.check(snap(300, {"email": (270, 0)})) == []
    assert m.check(snap(100, {"email": (0, 0)})) == []
    assert m.check(snap(200, {"email": (0, 0)})) == [
        FieldAlert(field="email", kind="fill", rate=0.0, baseline=0.9, responses=300)
    ]


def test_an_item_field_is_judged_only_in_windows_with_enough_elements() -> None:
    m = FillMonitor()

    def w(fill: int, total: int) -> dict[str, Any]:
        return {"responses": 1000, "fields": {"jobs[].title": {"fill": fill, "errors": 0, "total": total}}}

    for _ in range(5):
        m.check(w(900, 1000))
    assert m.check(w(0, 3)) == []
    assert m.check(w(0, 300)) == [FieldAlert(field="jobs[].title", kind="fill", rate=0.0, baseline=0.9, responses=300)]


def test_a_stats_snapshot_feeds_the_monitor() -> None:
    from pycanonic import compile_spec

    spec = {"provider": "p", "endpoint": "e", "version": 1, "status": "promoted", "fields": {"email": "workEmail"}}
    canonical = {"type": "object", "properties": {"email": {"type": "string"}}}
    a = compile_spec(spec, canonical)
    m = FillMonitor(min_responses=10)

    def period(body: dict[str, Any]) -> list[FieldAlert]:
        for _ in range(10):
            a.map(body)
        s = a.stats.snapshot()
        a.stats.reset()
        return m.check(s)

    for _ in range(3):
        assert period({"workEmail": "a@b.co"}) == []
    assert period({"email": "a@b.co"}) == [FieldAlert(field="email", kind="fill", rate=0.0, baseline=1.0, responses=10)]
