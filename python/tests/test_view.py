import gc
import json

import pytest

from pycanonic import _core


def test_reads_route_fields_and_nested_objects(bench):
    a, schema, bodies = bench
    raw, want = bodies["raws"][0], bodies["expected"][0]
    for src in (raw.encode(), raw, json.loads(raw)):
        v = a.view(src)
        assert v.email == want["email"]
        assert v.company.domain == want["company"]["domain"]
        assert v.experiences == want["experiences"]


def test_to_dict_equals_map_for_every_bench_body(bench):
    a, schema, bodies = bench
    for raw in bodies["raws"]:
        for src in (raw.encode(), json.loads(raw)):
            v = a.view(src)
            v.email, v.company.name  # some fields cached first
            d = v.to_dict()
            assert d == a.map(src)
            assert json.dumps(d) == json.dumps(a.map(src))
            assert v.company.to_dict() == d["company"]


def test_reads_do_not_touch_stats(bench):
    a, schema, bodies = bench
    a.stats_reset()
    before = a.stats_snapshot()
    for raw in bodies["raws"]:
        v = a.view(raw.encode())
        for name in v:
            getattr(v, name)
        v.company.domain
        v.to_dict()
    assert a.stats_snapshot() == before


def test_a_field_is_computed_once_and_only_when_read(bench_spec):
    spec, schema, raws = bench_spec
    calls = {"domain": 0, "name": 0}

    def counter(field):
        def f(v):
            calls[field] += 1
            return v

        return f

    spec = json.loads(json.dumps(spec))
    spec["fields"]["company.domain"] = {"from": "org.website", "transform": "count_domain"}
    spec["fields"]["company.name"] = {"from": "org.name", "transform": "count_name"}
    a = _core.compile(spec, schema, {"count_domain": counter("domain"), "count_name": counter("name")})
    raw = next(r for r in raws if "website" in r and '"name"' in r)
    v = a.view(raw.encode())
    assert v.company is v.company
    first = v.company.domain
    assert v.company.domain == first
    assert calls == {"domain": 1, "name": 0}


def test_unknown_field_raises_attribute_error(bench):
    a, schema, bodies = bench
    v = a.view(bodies["raws"][0].encode())
    with pytest.raises(AttributeError):
        v.nope
    with pytest.raises(AttributeError):
        v.company.email
    assert getattr(v, "nope", 7) == 7


def test_iter_and_dir_list_fields_in_schema_order(bench):
    a, schema, bodies = bench
    v = a.view(bodies["raws"][0])
    assert list(v) == list(schema["properties"])
    assert list(v.company) == list(schema["properties"]["company"]["properties"])
    assert set(schema["properties"]) <= set(dir(v))


def test_an_unparseable_body_reads_as_absent(bench):
    a, schema, bodies = bench
    v = a.view(b"{not json")
    assert v.email is None
    assert v.to_dict() == a.map(b"{not json")


def test_view_holds_its_own_copy_of_the_body(bench):
    a, schema, bodies = bench
    raw = bodies["raws"][0]
    body = raw.encode() + b" "
    v = a.view(body)
    del body
    gc.collect()
    assert v.to_dict() == a.map(raw)


def test_a_base_exception_from_a_transform_raises_from_the_read(bench_spec):
    spec, schema, raws = bench_spec
    spec = json.loads(json.dumps(spec))
    spec["fields"]["email"] = {"from": "contact.workEmail", "transform": "stop"}

    def stop(v):
        raise KeyboardInterrupt

    a = _core.compile(spec, schema, {"stop": stop})
    raw = next(r for r in raws if "workEmail" in r)
    v = a.view(raw)
    with pytest.raises(KeyboardInterrupt):
        v.email
    assert v.full_name == json.loads(raw)["profile"]["fullName"]


def test_a_list_field_is_built_once(bench):
    a, schema, bodies = bench
    v = a.view(bodies["raws"][0].encode())
    assert v.experiences is v.experiences
    assert v["skills"] is v.skills


def test_reads_of_parsed_input_do_not_touch_stats(bench):
    a, schema, bodies = bench
    a.stats_reset()
    before = a.stats_snapshot()
    for raw in bodies["raws"]:
        v = a.view(json.loads(raw))
        for name in v:
            v[name]
        v.company.domain
        v.to_dict()
    assert a.stats_snapshot() == before


def test_methods_win_attribute_access_and_getitem_reads_the_field():
    schema = {
        "type": "object",
        "properties": {"to_dict": {"type": "string"}, "__class__": {"type": "string"}, "x": {"type": "string"}},
    }
    spec = {
        "provider": "p",
        "endpoint": "e",
        "version": 1,
        "status": "promoted",
        "fields": {"to_dict": "a", "__class__": "b", "x": "c"},
    }
    a = _core.compile(spec, schema, None)
    v = a.view({"a": "A", "b": "B", "c": "C"})
    assert v.to_dict() == {"to_dict": "A", "__class__": "B", "x": "C"}
    assert v.__class__ is _core.View
    assert (v["to_dict"], v["__class__"], v["x"], v.x) == ("A", "B", "C", "C")
    with pytest.raises(KeyError):
        v["nope"]
    with pytest.raises(KeyError):
        v[0]
