"""Fixes from the 0.2.0 review of the Rust core: each test failed before its fix."""

import json
import math

import pycanonic

NUM = {"type": "object", "properties": {"n": {"type": "number"}}}


def _spec(fields, language=1):
    s = {"provider": "acme", "endpoint": "e", "version": 1, "status": "draft", "fields": fields}
    if language == 2:
        s["language"] = 2
    return s


def test_custom_transform_nan_or_inf_is_null_and_an_error_everywhere():
    for ret in (float("nan"), float("inf"), float("-inf"), 10**5000):
        a = pycanonic.compile_spec(_spec({"n": {"from": "x", "transform": "t"}}), NUM, transforms={"t": lambda v, r=ret: r})
        assert a.map({"x": 1}) == {"n": None}
        assert a.map(b'{"x": 1}') == {"n": None}
        assert a.map_json(b'{"x": 1}') == b'{"n":null}'
        assert a.stats_snapshot()["fields"]["n"] == {"fill": 0, "errors": 3}


def test_on_missing_transform_returning_nan_is_an_error():
    spec = _spec({"n": {"from": "x", "transform": "t", "on_missing": "transform"}}, language=2)
    a = pycanonic.compile_spec(spec, NUM, transforms={"t": lambda v: math.nan})
    assert a.map({}) == {"n": None}
    assert json.loads(a.map_json(b"{}")) == {"n": None}
    assert a.stats_snapshot()["fields"]["n"]["errors"] == 2


def test_overflowing_sum_is_null_and_an_error():
    a = pycanonic.compile_spec(_spec({"n": {"aggregate": "sum", "from": "xs"}}, language=2), NUM)
    assert a.map({"xs": [1e308, 1e308]}) == {"n": None}
    assert a.map_json(b'{"xs": [1e308, 1e308]}') == b'{"n":null}'
    assert a.stats_snapshot()["fields"]["n"]["errors"] == 2
