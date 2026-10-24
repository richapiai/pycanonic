"""Inference and `canonic infer`, ported from the TypeScript CLI's infer.test.ts and the infer cases of
commands.test.ts. The `fixtures/cli/ts/` files are the TypeScript command's own stdout and stderr on the
same inputs (`node packages/cli/dist/cli.js infer ...`), so the CLI tests compare byte for byte."""

from __future__ import annotations

import json
import random
from itertools import permutations
from pathlib import Path
from typing import Any

import pytest

import pycanonic
from pycanonic.cli.__main__ import _load_transforms, main
from pycanonic.cli.infer.ask import Question, ask_low_confidence
from pycanonic.cli.infer.hungarian import assign
from pycanonic.cli.infer.infer import infer, infer_draft, js_round, to_fixed
from pycanonic.cli.infer.score import is_numeric, looks_full_name
from pycanonic.cli.infer.tokens import SEED, split, tokens
from pycanonic.cli.io import js_json, js_keys, js_trim

HERE = Path(__file__).resolve().parent
CLI = HERE / "fixtures/cli"
SPECS = HERE / "fixtures/specs"
TS = CLI / "ts"
SCHEMA = str(CLI / "canonical.json")
CANONICAL: dict[str, Any] = json.loads((CLI / "canonical.json").read_text())
SPEC: dict[str, Any] = json.loads((CLI / "spec.json").read_text())
WANT: dict[str, Any] = SPEC["fields"]
TRUTH = pycanonic.compile_spec(SPEC, CANONICAL)
# bench/person.js payload(0..11), written by node: its generator relies on JavaScript float arithmetic.
PAYLOADS: list[Any] = json.loads((CLI / "bench-payloads.json").read_text())
OPTS: dict[str, Any] = {"provider": "acme", "endpoint": "person.enrich"}


def differs(fields: dict[str, Any]) -> list[str]:
    return [k for k in WANT if js_json(WANT[k]) != js_json(fields.get(k))]


def run(capsys: pytest.CaptureFixture[str], *args: str) -> tuple[int, str, str]:
    code = main(list(args))
    out, err = capsys.readouterr()
    return code, out, err


def infer_args(*rest: str) -> list[str]:
    return ["infer", "--schema", SCHEMA, "--provider", "acme", "--endpoint", "person.enrich", *rest]


# tokens


@pytest.mark.parametrize(
    ("name", "out"),
    [
        ("workEmailAddress", ["work", "email", "address"]),
        ("li_url", ["li", "url"]),
        ("HQCity", ["hq", "city"]),
        ("org-size2", ["org", "size", "2"]),
    ],
)
def test_split(name: str, out: list[str]) -> None:
    assert split(name) == out


@pytest.mark.parametrize(
    ("name", "out"),
    [
        ("fname", ["first", "name"]),
        ("fullName", ["full", "name"]),
        ("employerName", ["company", "name"]),
        ("mobile_no", ["phone", "number"]),
        ("companies", ["company"]),
        ("addresses", ["address"]),
    ],
)
def test_tokens(name: str, out: list[str]) -> None:
    assert tokens(name, SEED) == out


def test_full_names_are_letters_in_any_script_but_not_digits() -> None:
    # \p{L} in the TypeScript, str.isalpha() here. Digits, including non-ASCII ones, are not letters.
    assert looks_full_name("  José  Ñúñez ") and looks_full_name("Анна Каренина") and looks_full_name("O'Brien Smith-Jones")
    assert not looks_full_name("Ada") and not looks_full_name("Ada 2") and not looks_full_name("Ada ٣")
    assert looks_full_name("Ada　Lovelace")  # JavaScript \s includes the ideographic space


def test_the_javascript_behaviors_the_port_reproduces() -> None:
    # Math.round and toFixed round halves up; Python's round() would give 0.12.
    assert (js_round(0.125), to_fixed(0.125), js_round(0.8749), to_fixed(1)) == (0.13, "0.13", 0.87, "1.00")
    # JSON.stringify numbers and key order.
    assert js_json({"b": [1.0, 1e-7, 1e-5, 1e21, 0.1, -0.0], "10": True, "2": None}) == '{"2":null,"10":true,"b":[1,1e-7,0.00001,1e+21,0.1,0]}'
    assert js_keys({"a": 1, "01": 2, "1": 3, "4294967295": 4}) == ["1", "a", "01", "4294967295"]
    assert js_trim("\ufeff\u3000 a \n") == "a" and js_trim("\x1c a") == "\x1c a"
    # Number(): decimal with exponent, trimmed, blank is not a number. Divergence: "0x1A" is 26 in
    # JavaScript and not numeric here, as the core's `cast number` reads it.
    assert [is_numeric(v) for v in [" 12 ", "1e3", "-.5", "", "  ", "0x1A", "Infinity", 12]] == [True, True, True, False, False, False, False, False]


# assign


def brute(m: list[list[int]]) -> int:
    rows, cols = len(m), len(m[0])
    best = 0
    for k in range(min(rows, cols) + 1):
        for chosen_rows in permutations(range(rows), k):
            for chosen_cols in permutations(range(cols), k):
                best = max(best, sum(m[i][j] for i, j in zip(chosen_rows, chosen_cols)))
    return best


def test_assign_matches_the_brute_force_optimum_on_random_matrices_square_or_not() -> None:
    rnd = random.Random(7)
    for _ in range(300):
        r, c = rnd.randint(1, 4), rnd.randint(1, 4)
        m = [[rnd.randint(0, 100) for _ in range(c)] for _ in range(r)]
        pick = assign([[float(x) for x in row] for row in m])
        cols = [j for j in pick if j != -1]
        assert len(set(cols)) == len(cols)
        assert sum(m[i][j] for i, j in enumerate(pick) if j != -1) == brute(m)


# infer on the bench person payloads


def test_reproduces_the_hand_written_spec_except_what_the_samples_cant_show() -> None:
    r = infer_draft(CANONICAL, PAYLOADS, **OPTS, raws=[js_json(s) for s in PAYLOADS])
    # No sample lacks workEmail, so nothing suggests the fallback. Lowercasing is a house style.
    assert differs(r.spec["fields"]) == ["email", "company.industry", "skills[]"]
    assert r.spec["fields"]["email"] == {"from": "contact.workEmail", "transform": ["trim", "lower"]}
    assert r.spec["anchors"] == {"email": '"workEmail":', "phone": '"mobile":', "company.domain": '"website":'}
    assert r.unmapped == []
    assert r.spec["status"] == "draft"


def gappy() -> list[Any]:
    return [p if i % 3 else {**p, "contact": {**p["contact"], "workEmail": None}} for i, p in enumerate(PAYLOADS)]


def test_with_gaps_and_known_outputs_it_finds_the_coalesce_and_the_lowercasing() -> None:
    samples = gappy()
    r = infer_draft(CANONICAL, samples, **OPTS, known=[TRUTH.map(s) for s in samples])
    assert differs(r.spec["fields"]) == ["skills[]"]
    headcount = next(f for f in r.fields if f.field == "company.headcount")
    assert (headcount.how, headcount.overlap) == ("match", 1)


def test_gaps_alone_are_enough_for_a_coalesce_when_the_backup_passes_the_validator() -> None:
    r = infer_draft(CANONICAL, gappy(), **OPTS)
    assert r.spec["fields"]["email"] == {"coalesce": ["contact.workEmail", "contact.emails[0].address"], "transform": ["trim", "lower"]}


PEOPLE = [
    ["Ada", "Lovelace", "ada@acme.com", "https://acme.com", "+44 20 7946 0018", "https://linkedin.com/in/ada"],
    ["Alan", "Turing", "alan@bletchley.org", "https://bletchley.org/about", "+44 20 7946 0019", "https://www.linkedin.com/in/alan-t/"],
    ["Grace", "Hopper", "grace@navy.mil", "http://navy.mil", "+1 202 555 0100", "https://linkedin.com/in/grace"],
    ["Edsger", "Dijkstra", "ewd@utexas.edu", "https://utexas.edu", "+1 512 555 0101", "https://linkedin.com/in/ewd"],
    ["Barbara", "Liskov", "liskov@mit.edu", "https://www.mit.edu", "+1 617 555 0102", "https://linkedin.com/in/liskov"],
]


def test_a_snake_case_provider_with_abbreviations() -> None:
    samples = [
        {"fname": f, "lname": l, "e_mail": e, "tel": tel, "li_url": li, "org": {"org_name": f"{l} Labs", "url": site}, "skill_list": ["sql", "go"], "ids": [1, 2]}
        for f, l, e, site, tel, li in PEOPLE
    ]
    r = infer_draft(CANONICAL, samples, **OPTS)
    fields = r.spec["fields"]
    assert {
        k: fields.get(k)
        for k in ["first_name", "last_name", "full_name", "email", "phone", "linkedin_url", "company.name", "company.domain"]
    } == {
        "first_name": "fname",
        "last_name": "lname",
        "full_name": {"from": ["fname", "lname"], "transform": "join"},
        "email": {"from": "e_mail", "transform": ["trim", "lower"]},
        "phone": {"from": "tel", "transform": "normalize_phone"},
        "linkedin_url": {"from": "li_url", "transform": "normalize_linkedin"},
        "company.name": "org.org_name",
        "company.domain": {"from": "org.url", "transform": "to_domain"},
    }
    # A plain array maps as it is. An array whose name matches nothing stays unmapped.
    assert fields["skills[]"] == {"from": "skill_list"}
    assert "ids" not in js_json(fields)
    out = pycanonic.compile_spec(r.spec, CANONICAL).map(samples[1])
    assert out["email"] == "alan@bletchley.org" and out["company"]["domain"] == "bletchley.org"


def test_values_beat_names_an_email_under_a_meaningless_key_still_maps() -> None:
    samples = [{"person": {"n": f"{f} {l}"}, "x1": e, "x2": "unrelated text"} for f, l, e, *_ in PEOPLE]
    r = infer_draft(CANONICAL, samples, **OPTS)
    assert r.spec["fields"]["email"] == {"from": "x1", "transform": ["trim", "lower"]}
    email = next(f for f in r.fields if f.field == "email")
    assert email.confidence is not None and email.confidence < 0.7


def test_nothing_to_go_on_leaves_fields_unmapped_and_names_the_best_candidate() -> None:
    r = infer_draft(CANONICAL, [{"q": 1}, {"q": 2}], **OPTS)
    assert r.spec["fields"] == {}
    assert "email" in [u["field"] for u in r.unmapped]


def test_the_ask_pass_maps_what_a_model_names_ignores_unknown_paths_and_keeps_it_flagged() -> None:
    samples = [{"fname": f, "lname": l, "e_mail": e, "zq7": f" Engineer {i} "} for i, (f, l, e, *_) in enumerate(PEOPLE)]
    r = infer_draft(CANONICAL, samples, **OPTS)
    assert "title" in [u["field"] for u in r.unmapped]
    asked: list[Question] = []

    def model(q: Question) -> str | None:
        asked.append(q)
        return 'The title is in zq7.\n"zq7"' if q["field"] == "title" else "made.up.path"

    out = ask_low_confidence(r, CANONICAL, samples, model)
    assert "zq7" in [x["path"] for x in next(q for q in asked if q["field"] == "title")["sources"]]
    assert out.spec["fields"]["title"] == {"from": "zq7", "transform": "trim"}
    title = next(f for f in out.fields if f.field == "title")
    assert title.how == "ask" and isinstance(title.confidence, float)
    assert "title" not in [u["field"] for u in out.unmapped]
    assert len(out.spec["fields"]) == len(r.spec["fields"]) + 1
    assert ask_low_confidence(r, CANONICAL, samples, lambda q: None).spec == r.spec


def test_a_provider_key_named_like_an_object_prototype_member_is_just_a_word() -> None:
    samples = [{"name": f, "constructor": "x", "valueOf": 1, "e_mail": e} for f, _, e, *_ in PEOPLE]
    assert infer_draft(CANONICAL, samples, **OPTS).spec["fields"]["email"] == {"from": "e_mail", "transform": ["trim", "lower"]}


def test_a_list_that_matches_by_name_with_no_field_inside_it_stays_unmapped() -> None:
    r = infer_draft(CANONICAL, [{"experiences": [{"zz": 1, "qq": 2}]}, {"experiences": [{"zz": 3, "qq": 4}]}], **OPTS)
    assert r.spec["fields"] == {}
    assert "experiences[]" in [u["field"] for u in r.unmapped]


def test_the_ask_pass_on_a_draft_with_nothing_mapped_returns_it_instead_of_failing_to_compile() -> None:
    r = infer_draft(CANONICAL, [{"zzz": 1}], **OPTS)
    assert ask_low_confidence(r, CANONICAL, [{"zzz": 1}], lambda q: None).spec["fields"] == {}


def test_a_provider_whose_response_is_an_array_maps_from_dollar() -> None:
    schema = {
        "type": "object",
        "properties": {
            "people": {"type": "array", "items": {"type": "object", "properties": {"first_name": {"type": "string"}, "email": {"type": "string", "semantic": "email"}}}}
        },
    }
    samples = [[{"fname": f, "e_mail": e} for f, _, e, *_ in page] for page in (PEOPLE[:3], PEOPLE[3:])]
    r = infer_draft(schema, samples, **OPTS)
    assert r.spec["fields"] == {"people[]": {"from": "$", "item": {"first_name": "fname", "email": {"from": "e_mail", "transform": ["trim", "lower"]}}}}
    assert pycanonic.compile_spec(r.spec, schema).map(samples[1]) == {
        "people": [{"first_name": "Edsger", "email": "ewd@utexas.edu"}, {"first_name": "Barbara", "email": "liskov@mit.edu"}]
    }


def test_the_first_value_of_a_plain_array_can_fill_a_single_valued_field() -> None:
    samples = [{"contact": {"mobiles": [tel, "+1 000 000 0000"]}} for *_, tel, _ in PEOPLE]
    assert infer_draft(CANONICAL, samples, **OPTS).spec["fields"]["phone"] == {"from": "contact.mobiles[0]", "transform": "normalize_phone"}


def test_integer_like_keys_come_first_as_in_javascript() -> None:
    # Object.entries order: "10" and "2" before "b", in numeric order. "01" is not an index.
    samples = [{"b": {"x": i}, "10": f"t{i}", "2": f"u{i}", "01": f"v{i}"} for i in range(3)]
    r = infer_draft({"type": "object", "properties": {"q": {"type": "string"}}}, samples, **OPTS)
    assert r.unmapped == [{"field": "q", "best": '["2"]', "score": 0.35}]


def test_the_public_infer_returns_the_spec_and_one_report_per_field() -> None:
    samples = [json.loads((CLI / f"odd/p{i}.json").read_text()) for i in range(5)]
    spec, report = infer(samples, CANONICAL, provider="acme", endpoint="person.enrich")
    assert spec["fields"]["email"] == {"from": "e_mail", "transform": ["trim", "lower"]}
    assert [(f.field, f.how, f.check) for f in report[:3]] == [("first_name", "match", False), ("email", "match", False), ("last_name", "unmapped", False)]
    assert report[2].best == "fname" and report[2].confidence == 0.63
    learned, _ = infer(
        [json.loads((CLI / f"learn/p{i}.json").read_text()) for i in range(5)],
        CANONICAL,
        provider="acme",
        endpoint="person.enrich",
        learn=[json.loads((CLI / "approved.json").read_text())],
    )
    assert learned["fields"]["company.industry"] == "vertikal"


# canonic infer


def test_help_exits_0_and_bad_usage_exits_2(capsys: pytest.CaptureFixture[str]) -> None:
    code, out, _ = run(capsys, "infer", "--help")
    assert code == 0 and "canonic infer" in out
    assert run(capsys, "infer")[0] == 2


@pytest.mark.parametrize(
    ("name", "flags"),
    [("samples", []), ("odd", []), ("learn", []), ("learn-learned", ["--learn", str(CLI / "approved.json")]), ("samples-known", ["--known", str(CLI / "known.json")])],
)
def test_the_draft_and_the_report_match_the_typescript_command_byte_for_byte(
    capsys: pytest.CaptureFixture[str], name: str, flags: list[str]
) -> None:
    folder = name.split("-")[0]
    code, out, err = run(capsys, *infer_args(*flags, str(CLI / folder)))
    assert code == 0
    assert out == (TS / f"{name}.out.json").read_text()
    assert err == (TS / f"{name}.err.txt").read_text()


def test_writes_the_same_draft_as_infer_and_reports_on_stderr(capsys: pytest.CaptureFixture[str]) -> None:
    code, out, err = run(capsys, *infer_args(str(CLI / "samples")))
    assert code == 0
    raws = [(CLI / f"samples/p{i}.json").read_text() for i in range(8)]
    assert json.loads(out) == infer_draft(CANONICAL, [json.loads(t) for t in raws], **OPTS, raws=raws).spec
    assert 'anchor: email "workEmail":' in err
    assert any(line.startswith("first_name ") and line.split()[1] == "split" for line in err.splitlines())


def test_known_picks_transforms_that_reproduce_known_values(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    out = tmp_path / "inferred.json"
    code, stdout, _ = run(capsys, *infer_args("--known", str(CLI / "known.json"), "--out", str(out), str(CLI / "samples")))
    assert (code, stdout) == (0, "")
    assert out.read_text() == (CLI / "inferred.json").read_text()
    assert json.loads(out.read_text())["fields"]["company.industry"] == {"from": "org.industry", "transform": "lower"}


def test_ask_sends_each_doubtful_field_as_json_and_null_answers_change_nothing(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    log = tmp_path / "questions.jsonl"
    args = infer_args(str(CLI / "odd"))
    plain = run(capsys, *args)
    code, out, _ = run(capsys, *args, "--ask", f"cat >> '{log}'; echo >> '{log}'; echo null")
    assert code == 0
    assert out == plain[1]
    assert log.read_text() == (CLI / "questions.jsonl").read_text()


def test_an_ask_answer_is_used_and_a_failing_command_skips_the_field(capsys: pytest.CaptureFixture[str]) -> None:
    code, out, err = run(capsys, *infer_args(str(CLI / "odd"), "--ask", """python3 -c 'import json,sys; q=json.load(sys.stdin); print("zq7" if q["field"]=="title" else "null")'"""))
    assert code == 0 and json.loads(out)["fields"]["title"] == "zq7"
    assert 'title                       ask      0.35  "zq7"  <- check' in err.splitlines()
    code, _, err = run(capsys, *infer_args(str(CLI / "odd"), "--ask", "echo nope >&2; exit 3"))
    assert code == 0 and "--ask failed for title: nope" in err


@pytest.mark.parametrize("command", ["printf '\\377\\n'", "echo '{not json'", "echo 42", "echo '[\"zq7\"]'", "exit 1", "true"])
def test_any_ask_reply_that_is_not_a_known_path_skips_the_field(capsys: pytest.CaptureFixture[str], command: str) -> None:
    # A non-UTF-8 byte, text that is not JSON, JSON that is not a string, a failing or silent command.
    plain = run(capsys, *infer_args(str(CLI / "odd")))
    code, out, _ = run(capsys, *infer_args(str(CLI / "odd"), "--ask", command))
    assert (code, out) == (0, plain[1])


def test_learn_specs_without_fields_are_named(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match=r"learn\[0\]: an approved spec needs a fields object"):
        infer([{"a": 1}], CANONICAL, provider="acme", endpoint="person.enrich", learn=[{"provider": "old"}])


def test_learn_picks_up_a_synonym_from_an_approved_spec(capsys: pytest.CaptureFixture[str]) -> None:
    args = infer_args(str(CLI / "learn"))
    assert "company.industry" not in json.loads(run(capsys, *args)[1])["fields"]
    assert json.loads(run(capsys, *args, "--learn", str(CLI / "approved.json"))[1])["fields"]["company.industry"] == "vertikal"


def test_bad_input_exits_2(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    code, _, err = run(capsys, *infer_args("--min-score", "abc", str(CLI / "samples")))
    assert code == 2 and '--min-score needs a number from 0 to 1, got "abc"' in err
    assert run(capsys, *infer_args("--min-score", "1.5", str(CLI / "samples")))[0] == 2
    assert run(capsys, *infer_args(str(tmp_path / "missing.json")))[0] == 2
    broken = tmp_path / "broken.json"
    broken.write_text("{not json")
    code, _, err = run(capsys, *infer_args(str(broken)))
    assert code == 2 and str(broken) in err


def test_an_unknown_flag_is_one_line(capsys: pytest.CaptureFixture[str]) -> None:
    code, out, err = run(capsys, "infer", "--bogus")
    assert (code, out) == (2, "")
    assert err.startswith("canonic: ") and err.count("\n") == 1 and "--bogus" in err
    assert run(capsys, "test", "--bogus")[2].count("\n") == 1


def test_transforms_registers_only_the_modules_own_functions(tmp_path: Path) -> None:
    mod = tmp_path / "private.py"
    mod.write_text("from os.path import join\nfrom functools import partial\n\n\ndef shout(v):\n    return v.upper()\n\n\nloud = partial(shout)\n")
    args, fns = _load_transforms(["--transforms", str(mod), "x"])
    assert args == ["x"] and list(fns) == ["shout"]


# Parity with the TypeScript draft on the example providers


def flat(d: dict[str, Any], pre: str = "") -> dict[str, Any]:
    out: dict[str, Any] = {}
    for k, v in d.items():
        out.update(flat(v, f"{pre}{k}.") if isinstance(v, dict) else {pre + k: v})
    return out


@pytest.mark.parametrize("provider", ["acme", "globex"])
def test_the_draft_maps_every_example_sample_like_the_typescript_draft(
    capsys: pytest.CaptureFixture[str], tmp_path: Path, provider: str
) -> None:
    schema_file = str(SPECS / "canonical/person.enrich.json")
    schema = json.loads(Path(schema_file).read_text())
    payloads = [c["payload"] for c in json.loads((SPECS / f"fixtures/{provider}.json").read_text())["cases"]]
    for i, p in enumerate(payloads):
        (tmp_path / f"{i}.json").write_text(js_json(p, 2))
    code, out, err = run(capsys, "infer", "--schema", schema_file, "--provider", provider, "--endpoint", "person.enrich", str(tmp_path))
    assert code == 0
    assert out == (TS / f"{provider}.out.json").read_text()
    assert err == (TS / f"{provider}.err.txt").read_text()
    assert err.startswith("only 3 samples: ")
    draft = pycanonic.compile_spec(out, schema)
    ts = pycanonic.compile_spec((TS / f"{provider}.out.json").read_text(), schema)
    hand = pycanonic.compile_spec((SPECS / f"{provider}.json").read_text(), schema)
    for p in payloads:
        assert draft.map(p) == ts.map(p)
        # The hand-written spec lowercases skills, a house style no sample shows.
        mine, theirs = flat(draft.map(p)), flat(hand.map(p))
        assert [k for k in theirs if mine.get(k) != theirs[k]] == ["skills"]


def test_the_sample_files_are_the_first_eight_bench_payloads() -> None:
    src = CLI / "samples"
    assert sorted(f.name for f in src.iterdir()) == [f"p{i}.json" for i in range(8)]
    assert [(src / f"p{i}.json").read_text() for i in range(8)] == [js_json(p, 2) for p in PAYLOADS[:8]]
