import json
from pathlib import Path

import pytest

FIXTURES = Path(__file__).resolve().parents[2] / "crates/canonic-core/tests/fixtures"


def fixture(name):
    return json.loads((FIXTURES / name).read_text())


@pytest.fixture(scope="session")
def bench():
    from pycanonic import _core

    spec = fixture("specs/bench.json")
    schema = fixture("specs/canonical/bench.json")
    return _core.compile(spec, schema, None), schema, fixture("bench_bodies.json")


@pytest.fixture(scope="session")
def bench_spec():
    """The bench spec, its canonical schema and the raw bench bodies, uncompiled."""
    return fixture("specs/bench.json"), fixture("specs/canonical/bench.json"), fixture("bench_bodies.json")["raws"]


@pytest.fixture(scope="session")
def person():
    """The `person.enrich` canonical schema that acme and globex map onto."""
    return fixture("specs/canonical/person.enrich.json")
