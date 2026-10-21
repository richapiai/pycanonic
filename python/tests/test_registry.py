"""Port of canonic/packages/canonic/test/registry.test.ts."""

import pytest

from pycanonic import Registry, compile_spec


@pytest.fixture
def make(person):
    def make(version, status, from_="e"):
        spec = {
            "provider": "acme",
            "endpoint": "person.enrich",
            "version": version,
            "status": status,
            "fields": {"email": from_},
        }
        return compile_spec(spec, person)

    return make


def test_get_returns_the_promoted_version_get_shadow_the_shadow_version_or_none(make):
    r = Registry()
    assert r.get("acme", "person.enrich") is None
    v3 = make(3, "promoted")
    v4 = make(4, "shadow")
    r.put("acme", "person.enrich", 3, v3)
    assert r.get_shadow("acme", "person.enrich") is None
    r.put("acme", "person.enrich", 4, v4)
    assert r.get("acme", "person.enrich") is v3
    assert r.get_shadow("acme", "person.enrich") is v4
    assert r.get("acme", "company.enrich") is None
    assert r.get("other", "person.enrich") is None


def test_a_request_holding_the_old_artifact_finishes_on_it_after_a_swap(make):
    r = Registry()
    r.put("acme", "person.enrich", 3, make(3, "promoted", "old"))
    held = r.get("acme", "person.enrich")
    r.put("acme", "person.enrich", 4, make(4, "promoted", "new"))
    payload = {"old": "old@acme.com", "new": "new@acme.com"}
    assert held.map(payload)["email"] == "old@acme.com"
    assert r.get("acme", "person.enrich").map(payload)["email"] == "new@acme.com"


def test_promoting_the_shadow_version_clears_the_shadow_slot(make):
    r = Registry()
    r.put("acme", "person.enrich", 4, make(4, "shadow"))
    r.put("acme", "person.enrich", 4, make(4, "promoted"))
    assert r.get_shadow("acme", "person.enrich") is None
    assert r.get("acme", "person.enrich").version == 4


def test_promoting_another_version_keeps_the_shadow_slot(make):
    r = Registry()
    v5 = make(5, "shadow")
    r.put("acme", "person.enrich", 5, v5)
    r.put("acme", "person.enrich", 4, make(4, "promoted"))
    assert r.get_shadow("acme", "person.enrich") is v5


def test_deprecated_and_draft_versions_are_refused(make):
    r = Registry()
    with pytest.raises(ValueError, match="is deprecated"):
        r.put("acme", "person.enrich", 2, make(2, "deprecated"))
    with pytest.raises(ValueError, match="is draft"):
        r.put("acme", "person.enrich", 5, make(5, "draft"))
    assert r.get("acme", "person.enrich") is None


def test_put_refuses_an_artifact_that_does_not_match_its_key(make):
    r = Registry()
    with pytest.raises(ValueError, match="does not match"):
        r.put("acme", "person.enrich", 9, make(3, "promoted"))
    with pytest.raises(ValueError, match="does not match"):
        r.put("beta", "person.enrich", 3, make(3, "promoted"))
    with pytest.raises(ValueError, match="does not match"):
        r.put("acme", "company.enrich", 3, make(3, "promoted"))
