"""interpreters.py: the payloads match canonic's interpreters.js, and every library maps them to the
same output as pycanonic. This is the correctness gate; timing stays in `python bench/interpreters.py`."""

import hashlib
import re

import orjson

import interpreters as I

# interpreters.js does not export its payloads, so this hash was taken once by evaluating its
# `GROUPS` .. `genPayload` section in Node at canonic 936d6fd:
#   sha256(JSON.stringify(Array.from({ length: 64 }, (_, i) => genPayload(i + 1))))
JS_SHA256 = "65fa3fe084f2ec4dd540e83f3fa7a12a9ef40fd31bcf750ce3d43a29c126fb5e"


def test_payloads_are_deterministic_and_match_javascript():
    a = orjson.dumps([I.gen_payload(i + 1) for i in range(64)])
    assert a == orjson.dumps([I.gen_payload(i + 1) for i in range(64)])
    assert hashlib.sha256(a).hexdigest() == JS_SHA256


def test_mapping_has_501_leaves():
    assert len(re.findall(rb'"f\d+":|"skill\d+"', orjson.dumps(I.CANONIC.map(I.payloads[0])))) == 501


def test_every_library_matches_pycanonic():
    names = {name.split(" ")[0] for name, *_ in I.LIBRARIES}
    assert {"glom", "jsonpath-ng", "jsonbender", "pydantic"} <= names and names & {"jmespath", "jmespath-community"}
    for k in (0, 3, 63):
        p, raw = I.payloads[k], I.raws[k]
        want, want_plain = I.norm(I.CANONIC.map(p)), I.norm(I.CANONIC_PLAIN.map(p))
        assert I.norm(I.CANONIC.map(raw)) == want
        assert I.CANONIC.map_json(raw) == orjson.dumps(I.CANONIC.map(raw))
        for name, transforms, fn in I.LIBRARIES + I.REFERENCES:
            assert I.norm(fn(p)) == (want if transforms else want_plain), (name, k)
