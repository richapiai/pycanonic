"""`Artifact.extract`: the examples from canonic/docs/streaming.md and the bench bodies."""

import pytest
from conftest import fixture

from pycanonic import compile_spec

ADA = '{"fname":"Ada","e_mail":" Ada@Globex.com ","tel":"+1 415 555 0100","org_url":"https://globex.com/x"}'
ROUTED = {"email": "ada@globex.com", "phone": "+14155550100", "company.domain": "globex.com"}


@pytest.fixture(scope="module")
def globex(person):
    return compile_spec(fixture("specs/globex.json"), person)


def anchored_spec():
    """canonic/bench/person.js `anchoredSpec`, as the `anchored` fixture array was made."""
    s = fixture("specs/bench.json")
    s["anchors"] = {"email": '"workEmail":', "phone": '"mobile":', "company.domain": '"website":'}
    s["fields"]["email"] = {"from": "contact.workEmail", "transform": ["trim", "lower"]}
    return s


def test_extract_the_docs_example(globex):
    assert globex.extract(ADA) == ROUTED
    assert list(globex.extract(ADA)) == globex.routing


def test_extract_bytes_equals_str(globex):
    assert globex.extract(ADA.encode()) == globex.extract(ADA)


def test_extract_trusts_its_input(globex):
    assert globex.extract('{"e_mail":"a@b.com", oops') == {"email": "a@b.com", "phone": None, "company.domain": None}
    assert globex.extract('{"e_mail":"first@a.com","e_mail":"last@a.com"}')["email"] == "first@a.com"


def test_extract_rejects_other_types(globex):
    with pytest.raises(TypeError):
        globex.extract({"e_mail": "a@b.com"})


def test_extract_does_not_count_stats(globex):
    globex.stats.reset()
    before = globex.stats.snapshot()
    globex.extract(ADA)
    assert globex.stats.snapshot() == before


@pytest.mark.parametrize("anchored", [False, True], ids=["scanner", "anchored"])
def test_extract_every_bench_body(bench_spec, anchored):
    spec, schema, raws = bench_spec
    a = compile_spec(anchored_spec() if anchored else spec, schema)
    want = fixture("bench_bodies.json")["anchored" if anchored else "routing"]
    for raw, w in zip(raws, want, strict=True):
        assert a.extract(raw.encode()) == w
        assert a.extract(raw) == w
