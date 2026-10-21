"""person.py and routing.py must build the bodies of canonic's person.js and routing.js byte for
byte (ts_outputs.json, written by dump.mjs), and the hand-written readers must agree with them."""

import json
from pathlib import Path

import person
import routing

TS = json.loads((Path(__file__).parent / "ts_outputs.json").read_text())


def test_person_bodies_match_typescript():
    assert [person.raw(i) for i in range(64)] == TS["raws"]


def test_routing_bodies_match_typescript():
    assert [routing.raw(i) for i in range(64)] == TS["routing_raws"]


def test_hand_written_matches_typescript():
    for i in range(64):
        p = json.loads(TS["raws"][i])
        assert person.hand_written(p) == TS["expected"][i]
        assert person.hand_routing(p) == TS["routing"][i]
        assert routing.hand_read(json.loads(TS["routing_raws"][i])) == TS["hand_read"][i]
