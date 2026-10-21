"""The public surface of `pycanonic`; typed, and checked with `mypy --strict` by the last test."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

import pycanonic
from pycanonic import (
    Artifact,
    Issue,
    SpecError,
    Stats,
    _core,
    compile_spec,
    default_transforms,
    load,
    validate_canonical,
    validate_spec,
    validators,
)

Bench = tuple[dict[str, Any], dict[str, Any], list[str]]


BAD_SPEC = {
    "provider": "acme",
    "endpoint": "person.enrich",
    "version": 1,
    "status": "promoted",
    "fields": {"nope": "x"},
}


def test_compile_spec_raises_spec_error_with_the_core_message(person: dict[str, Any]) -> None:
    with pytest.raises(ValueError) as core:
        _core.compile(BAD_SPEC, person)
    with pytest.raises(SpecError) as err:
        compile_spec(BAD_SPEC, person)
    assert isinstance(err.value, ValueError)
    assert str(err.value) == str(core.value) == 'acme person.enrich v1: fields["nope"]: canonical schema has no field "nope"'


def test_compile_spec_returns_an_artifact(bench_spec: Bench) -> None:
    spec, schema, _ = bench_spec
    a = compile_spec(spec, schema, transforms={})
    assert isinstance(a, Artifact)
    assert (a.provider, a.status) == (spec["provider"], spec["status"])


def test_validate_spec_returns_issues() -> None:
    issues = validate_spec({"fields": {}})
    assert issues[0] == Issue(at="spec.provider", message="must be a non-empty string")
    assert all(isinstance(i, Issue) for i in issues)
    assert len(issues) == 5


def test_validate_spec_is_empty_for_a_good_spec(bench_spec: Bench) -> None:
    assert validate_spec(bench_spec[0]) == []


def test_validate_canonical_returns_issues(person: dict[str, Any]) -> None:
    assert validate_canonical({"type": "string"}) == [
        Issue("canonical schema", "root must be an object with properties")
    ]
    assert validate_canonical(person) == []


def test_load_of_a_plan_file_maps_identically(bench_spec: Bench, tmp_path: Path) -> None:
    spec, schema, raws = bench_spec
    a = compile_spec(spec, schema)
    path = tmp_path / "bench.plan.json"
    path.write_text(a.plan_json(), encoding="utf-8")
    for b in (load(path), load(str(path)), load(a.plan_json()), load(a.plan_json().encode())):
        assert (b.provider, b.endpoint, b.version, b.status, b.routing) == (
            a.provider,
            a.endpoint,
            a.version,
            a.status,
            a.routing,
        )
        for raw in raws:
            assert b.map(raw) == a.map(raw)
            assert b.map_json(raw) == a.map_json(raw)


def test_load_of_a_missing_path_raises(tmp_path: Path) -> None:
    with pytest.raises(FileNotFoundError):
        load(tmp_path / "missing.json")


def test_load_passes_custom_transforms(tmp_path: Path, person: dict[str, Any]) -> None:
    spec = {
        "provider": "acme",
        "endpoint": "person.enrich",
        "version": 1,
        "status": "promoted",
        "fields": {"full_name": {"from": "name", "transform": "shout"}},
    }

    def shout(v: object) -> object:
        return v.upper() if isinstance(v, str) else None

    a = compile_spec(spec, person, transforms={"shout": shout})
    b = load(a.plan_json(), transforms={"shout": shout})
    assert b.map({"name": "ada"})["full_name"] == "ADA"


def test_stats_object_matches_stats_snapshot(bench_spec: Bench) -> None:
    spec, schema, raws = bench_spec
    a = compile_spec(spec, schema)
    for raw in raws:
        a.map(raw)
    stats = a.stats
    assert isinstance(stats, Stats)
    snap = stats.snapshot()
    assert snap == a.stats_snapshot()
    assert snap["responses"] == len(raws)
    assert snap["fields"]["experiences[].title"]["total"] >= 0
    stats.reset()
    assert a.stats.snapshot()["responses"] == 0


def test_default_transforms_and_validators_like_the_readme() -> None:
    assert default_transforms["lower"]("ÀDA") == "àda"
    assert default_transforms["split_name"]("Ada Lovelace", "last") == "Lovelace"
    assert sorted(default_transforms) == sorted(
        [
            "cast",
            "lower",
            "trim",
            "split_name",
            "join",
            "parse_date",
            "normalize_phone",
            "normalize_linkedin",
            "to_domain",
        ]
    )
    assert validators["email"]("N/A") is None
    assert validators["email"]("ada@acme.com") == "ada@acme.com"
    assert sorted(validators) == sorted(["email", "phone", "domain", "linkedin_url", "url", "iso_date", "money"])


def test_every_name_in_all_exists() -> None:
    for name in pycanonic.__all__:
        assert hasattr(pycanonic, name), name


def test_the_package_ships_stubs_and_the_typed_marker() -> None:
    root = Path(pycanonic.__file__).parent
    assert (root / "py.typed").is_file()
    assert (root / "_core.pyi").is_file()


@pytest.mark.skipif(importlib.util.find_spec("mypy") is None, reason="mypy is not installed")
def test_this_file_passes_mypy_strict() -> None:
    drift = str(Path(__file__).with_name("test_drift.py"))
    shadow = str(Path(__file__).with_name("test_shadow.py"))
    cli = str(Path(__file__).with_name("test_cli_test_build.py"))
    infer = str(Path(__file__).with_name("test_infer.py"))
    diff = str(Path(__file__).with_name("test_diff.py"))
    bench = str(Path(__file__).with_name("test_bench_cmd.py"))
    done = subprocess.run(
        [sys.executable, "-m", "mypy", "--strict", "--no-incremental", __file__, drift, shadow, cli, infer, diff, bench],
        capture_output=True,
        text=True,
    )
    assert done.returncode == 0, done.stdout + done.stderr
