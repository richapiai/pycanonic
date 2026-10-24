// The A9 transforms of language 2: regex, strip_prefix, strip_suffix, replace, lookup, null_if,
// upper, title, split, to_string, format_date, parse_date formats, parse_number and parse_range.

use canonic_core::{ArgError, Builtin, Plan, Transform, Value, builtin, compile};
use proptest::prelude::*;
use serde_json::{Value as J, json};
use std::borrow::Cow;
use std::time::{Duration, Instant};

fn s(x: &str) -> Value<'static> {
    Value::Str(Cow::Owned(x.to_string()))
}

fn tx(name: &str, arg: Option<J>) -> Builtin {
    builtin(name, arg.as_ref()).unwrap_or_else(|e| panic!("{name} {arg:?}: {e:?}"))
}

fn run(name: &str, arg: Option<J>, v: Value<'static>) -> Value<'static> {
    tx(name, arg).apply(v, None)
}

#[test]
fn cases() {
    use Value::{Bool, Float, Int, Null};
    let lookup = |e: J| Some(json!({"table": {"LIKE": "liked", "1": "one"}, "else": e}));
    let cases: Vec<(&str, Option<J>, Value<'static>, Value<'static>)> = vec![
        (
            "regex",
            Some(json!("/company/([^/?#]+)")),
            s("https://x.com/company/acme/?a=1"),
            s("acme"),
        ),
        ("regex", Some(json!("/company/([^/?#]+)")), s("nope"), Null),
        ("regex", Some(json!("[0-9]+")), s("id 42 x"), s("42")),
        ("regex", Some(json!("(a)?b")), s("b"), Null),
        (
            "strip_prefix",
            Some(json!("urn:li:person:")),
            s("urn:li:person:ABC"),
            s("ABC"),
        ),
        (
            "strip_prefix",
            Some(json!("urn:li:person:")),
            s("ABC"),
            s("ABC"),
        ),
        (
            "strip_prefix",
            Some(json!("urn:li:person:")),
            s("urn:li:person:"),
            Null,
        ),
        (
            "strip_suffix",
            Some(json!(".git")),
            s("repo.git"),
            s("repo"),
        ),
        ("strip_suffix", Some(json!(".git")), s("repo"), s("repo")),
        ("strip_suffix", Some(json!(".git")), s(".git"), Null),
        (
            "replace",
            Some(json!({"from": "-", "to": " "})),
            s("a-b-c"),
            s("a b c"),
        ),
        (
            "replace",
            Some(json!({"pattern": "[0-9]+", "to": "#"})),
            s("a1b22c"),
            s("a#b#c"),
        ),
        // `to` is literal text: `$1` is not a group reference.
        (
            "replace",
            Some(json!({"pattern": "(b)", "to": "$1"})),
            s("abc"),
            s("a$1c"),
        ),
        ("lookup", lookup(json!("keep")), s("LIKE"), s("liked")),
        ("lookup", lookup(json!("keep")), s("LOVE"), s("LOVE")),
        ("lookup", lookup(json!("null")), s("LOVE"), Null),
        ("lookup", lookup(json!("other")), s("LOVE"), s("other")),
        ("lookup", lookup(json!(0)), s("LOVE"), Int(0)),
        ("lookup", lookup(json!("keep")), Int(1), s("one")),
        ("lookup", lookup(json!("keep")), Float(1.0), s("one")),
        ("lookup", lookup(json!("keep")), Int(2), Int(2)),
        (
            "lookup",
            Some(json!({"table": {"true": 1, "x": null}})),
            Bool(true),
            Int(1),
        ),
        ("lookup", Some(json!({"table": {"x": null}})), s("x"), Null),
        // The default for `else` is "null".
        ("lookup", Some(json!({"table": {}})), s("LOVE"), Null),
        ("null_if", Some(json!(["", "N/A", 0])), s("N/A"), Null),
        ("null_if", Some(json!(["", "N/A", 0])), s(""), Null),
        ("null_if", Some(json!(["", "N/A", 0])), Float(0.0), Null),
        ("null_if", Some(json!(["", "N/A", 0])), Int(0), Null),
        ("null_if", Some(json!(["", "N/A", 0])), s("0"), s("0")),
        (
            "null_if",
            Some(json!(["", "N/A", 0])),
            Bool(false),
            Bool(false),
        ),
        ("null_if", Some(json!([false])), Bool(false), Null),
        ("null_if", Some(json!([1.5])), Float(1.5), Null),
        ("title", None, s("ada LOVELACE"), s("Ada Lovelace")),
        ("title", None, s("jean-luc o'brien"), s("Jean-Luc O'brien")),
        ("upper", None, s("àda"), s("ÀDA")),
        (
            "split",
            Some(json!({"sep": " ", "index": -1})),
            s("Ada King Lovelace"),
            s("Lovelace"),
        ),
        (
            "split",
            Some(json!({"sep": " ", "index": 0})),
            s("Ada King Lovelace"),
            s("Ada"),
        ),
        (
            "split",
            Some(json!({"sep": " ", "index": 3})),
            s("Ada King Lovelace"),
            Null,
        ),
        (
            "split",
            Some(json!({"sep": " ", "index": -4})),
            s("Ada King Lovelace"),
            Null,
        ),
        (
            "split",
            Some(json!({"sep": ",", "index": 1})),
            s("a,,b"),
            Null,
        ),
        ("to_string", None, Int(12), s("12")),
        ("to_string", None, Float(1.5), s("1.5")),
        ("to_string", None, Bool(true), s("true")),
        ("to_string", None, s("x"), s("x")),
        ("to_string", None, Float(f64::NAN), Null),
    ];
    for (name, arg, input, want) in cases {
        assert_eq!(
            run(name, arg.clone(), input.clone()),
            want,
            "{name} {arg:?} on {input:?}"
        );
    }
}

#[test]
fn regex_is_linear_time() {
    let input = format!("{}b", "a".repeat(30));
    let t = tx("regex", Some(json!("(a+)+$")));
    let start = Instant::now();
    assert_eq!(t.apply(s(&input), None), Value::Null);
    assert!(start.elapsed() < Duration::from_millis(50));
}

#[test]
fn arguments_are_checked() {
    let bad: Vec<(&str, Option<J>)> = vec![
        ("regex", None),
        ("regex", Some(json!(1))),
        ("regex", Some(json!("(unclosed"))),
        ("strip_prefix", None),
        ("strip_prefix", Some(json!(""))),
        ("strip_suffix", Some(json!(["x"]))),
        ("replace", None),
        ("replace", Some(json!({"from": "a"}))),
        ("replace", Some(json!({"from": "", "to": "x"}))),
        ("replace", Some(json!({"from": "a", "to": 1}))),
        ("replace", Some(json!({"pattern": "(", "to": "x"}))),
        (
            "replace",
            Some(json!({"from": "a", "pattern": "b", "to": "x"})),
        ),
        ("replace", Some(json!({"from": "a", "to": "x", "y": 1}))),
        ("lookup", None),
        ("lookup", Some(json!({"else": "keep"}))),
        ("lookup", Some(json!({"table": [], "else": "keep"}))),
        ("lookup", Some(json!({"table": {"a": [1]}}))),
        ("lookup", Some(json!({"table": {}, "else": {"a": 1}}))),
        ("lookup", Some(json!({"table": {}, "x": 1}))),
        ("null_if", None),
        ("null_if", Some(json!("N/A"))),
        ("upper", Some(json!(null))),
        ("title", Some(json!("x"))),
        ("split", None),
        ("split", Some(json!({"sep": " "}))),
        ("split", Some(json!({"sep": "", "index": 0}))),
        ("split", Some(json!({"sep": " ", "index": 1.5}))),
        ("split", Some(json!({"sep": " ", "index": 0, "x": 1}))),
        ("to_string", Some(json!("string"))),
    ];
    for (name, arg) in bad {
        assert_eq!(
            builtin(name, arg.as_ref()),
            Err(ArgError::InvalidArgument),
            "{name} {arg:?}"
        );
    }
}

#[test]
fn dates_numbers_and_ranges() {
    use Value::{Bool, Float, Int, Null};
    let dms = || Some(json!("datetime_ms"));
    let formats = |f: J| Some(json!({ "formats": f }));
    let units = || Some(json!({"units": true}));
    let cases: Vec<(&str, Option<J>, Value<'static>, Value<'static>)> = vec![
        // The rows of the brief.
        (
            "format_date",
            dms(),
            s("2021-03"),
            s("2021-03-01T00:00:00.000Z"),
        ),
        (
            "format_date",
            dms(),
            s("1996"),
            s("1996-01-01T00:00:00.000Z"),
        ),
        (
            "format_date",
            dms(),
            s("2021-03-01T23:30:00-05:00"),
            s("2021-03-02T04:30:00.000Z"),
        ),
        (
            "format_date",
            dms(),
            Int(1614556800),
            s("2021-03-01T00:00:00.000Z"),
        ),
        (
            "format_date",
            Some(json!("epoch_ms")),
            s("2021-03-01"),
            Int(1614556800000),
        ),
        ("format_date", Some(json!("date")), s("2021-02-29"), Null),
        (
            "parse_date",
            formats(json!(["%B %Y"])),
            s("june 2014"),
            s("2014-06"),
        ),
        (
            "parse_date",
            formats(json!(["%m/%Y"])),
            s("03/2021"),
            s("2021-03"),
        ),
        ("parse_number", units(), s("$3.2M"), Int(3200000)),
        ("parse_number", units(), s("40k"), Int(40000)),
        ("parse_number", units(), s("1,234"), Int(1234)),
        ("parse_number", None, s("40k"), Null),
        ("parse_range", Some(json!("min")), s("10,001+"), Int(10001)),
        ("parse_range", Some(json!("max")), s("10,001+"), Null),
        // format_date, each output.
        (
            "format_date",
            Some(json!("date")),
            s("2021-03-01T23:30:00-05:00"),
            s("2021-03-02"),
        ),
        (
            "format_date",
            Some(json!("datetime")),
            s("2021-03-01T04:30:15.250Z"),
            s("2021-03-01T04:30:15Z"),
        ),
        (
            "format_date",
            dms(),
            s("2021-03-01T04:30:15.25Z"),
            s("2021-03-01T04:30:15.250Z"),
        ),
        (
            "format_date",
            dms(),
            s("2021-03-01 04:30:15+0130"),
            s("2021-03-01T03:00:15.000Z"),
        ),
        // No offset is UTC.
        (
            "format_date",
            dms(),
            s("2021-03-01T04:30"),
            s("2021-03-01T04:30:00.000Z"),
        ),
        (
            "format_date",
            Some(json!("epoch_s")),
            s("2021-03-01T00:00:01.999Z"),
            Int(1614556801),
        ),
        (
            "format_date",
            Some(json!("epoch_s")),
            s("1969-12-31"),
            Int(-86400),
        ),
        (
            "format_date",
            Some(json!("epoch_ms")),
            Int(1614556800123),
            Int(1614556800123),
        ),
        (
            "format_date",
            Some(json!("date")),
            Float(1614556800.0),
            s("2021-03-01"),
        ),
        // A time parse_date drops but format_date cannot read is the date at midnight, as parse_date gives it.
        (
            "format_date",
            dms(),
            s("2021-03-01Tjunk"),
            s("2021-03-01T00:00:00.000Z"),
        ),
        (
            "format_date",
            dms(),
            s("2021-03-01T24:00:00Z"),
            s("2021-03-01T00:00:00.000Z"),
        ),
        // Past year 9999 after the offset, or what parse_date refuses, is null.
        ("format_date", dms(), s("9999-12-31T23:00:00-05:00"), Null),
        ("format_date", dms(), s("2021-13"), Null),
        ("format_date", dms(), Int(0), Null),
        ("format_date", dms(), Bool(true), Null),
        // parse_date formats.
        (
            "parse_date",
            formats(json!(["%b %d, %Y"])),
            s("JUN 5, 2014"),
            s("2014-06-05"),
        ),
        (
            "parse_date",
            formats(json!(["%d/%m/%Y"])),
            s(" 29/02/2021 "),
            Null,
        ),
        (
            "parse_date",
            formats(json!(["%d/%m/%Y"])),
            s("28/02/2021"),
            s("2021-02-28"),
        ),
        ("parse_date", formats(json!(["%Y"])), s("2014"), s("2014")),
        (
            "parse_date",
            formats(json!(["%m/%Y", "%B %Y"])),
            s("March 2021"),
            s("2021-03"),
        ),
        ("parse_date", formats(json!(["%B %Y"])), s("jun 2014"), Null),
        (
            "parse_date",
            formats(json!(["%b %Y"])),
            s("june 2014"),
            Null,
        ),
        (
            "parse_date",
            formats(json!(["%m/%Y"])),
            s("3/2021"),
            s("2021-03"),
        ),
        ("parse_date", formats(json!(["%m/%Y"])), s("13/2021"), Null),
        ("parse_date", formats(json!(["%m/%Y"])), s("03/2021x"), Null),
        // A value no format reads falls back to the rules without an argument.
        (
            "parse_date",
            formats(json!(["%m/%Y"])),
            s("2021-03-04"),
            s("2021-03-04"),
        ),
        (
            "parse_date",
            formats(json!(["%m/%Y"])),
            Int(1614556800),
            s("2021-03-01"),
        ),
        // parse_number.
        ("parse_number", None, s("1,234.5"), Float(1234.5)),
        ("parse_number", None, s("USD 1,000"), Int(1000)),
        ("parse_number", None, s("€12"), Int(12)),
        ("parse_number", None, Int(7), Int(7)),
        ("parse_number", None, Float(2.0), Int(2)),
        ("parse_number", units(), s("1.5b"), Int(1500000000)),
        ("parse_number", units(), s("USD 2.5K"), Int(2500)),
        ("parse_number", units(), s("12"), Int(12)),
        (
            "parse_number",
            Some(json!({"units": false})),
            s("40k"),
            Null,
        ),
        ("parse_number", units(), s("k"), Null),
        ("parse_number", units(), s("1e3k"), Null),
        ("parse_number", None, s("N/A"), Null),
        ("parse_number", None, s(""), Null),
        ("parse_number", None, Bool(true), Null),
        ("parse_number", None, Float(f64::NAN), Null),
        // parse_range.
        ("parse_range", Some(json!("min")), s("51-200"), Int(51)),
        ("parse_range", Some(json!("max")), s("51-200"), Int(200)),
        ("parse_range", Some(json!("min")), s("51 - 200"), Int(51)),
        ("parse_range", Some(json!("max")), s("51 - 200"), Int(200)),
        ("parse_range", Some(json!("min")), s("5"), Int(5)),
        ("parse_range", Some(json!("max")), s("5"), Int(5)),
        ("parse_range", Some(json!("max")), Int(5), Int(5)),
        (
            "parse_range",
            Some(json!("max")),
            s("1,001-5,000"),
            Int(5000),
        ),
        ("parse_range", Some(json!("min")), s("51-"), Null),
        ("parse_range", Some(json!("min")), s("a-b"), Null),
        ("parse_range", Some(json!("min")), s("+"), Null),
        ("parse_range", Some(json!("min")), Bool(true), Null),
    ];
    for (name, arg, input, want) in cases {
        assert_eq!(
            run(name, arg.clone(), input.clone()),
            want,
            "{name} {arg:?} on {input:?}"
        );
    }
}

#[test]
fn date_number_range_arguments_are_checked() {
    let bad: Vec<(&str, Option<J>)> = vec![
        ("format_date", None),
        ("format_date", Some(json!("time"))),
        ("format_date", Some(json!(1))),
        ("parse_date", Some(json!("%Y"))),
        ("parse_date", Some(json!({"formats": []}))),
        ("parse_date", Some(json!({"formats": "%Y"}))),
        ("parse_date", Some(json!({"formats": [1]}))),
        ("parse_date", Some(json!({"formats": ["%m/%d"]}))),
        ("parse_date", Some(json!({"formats": ["%Y %d"]}))),
        ("parse_date", Some(json!({"formats": ["%Y %Y"]}))),
        ("parse_date", Some(json!({"formats": ["%Y %m %B"]}))),
        ("parse_date", Some(json!({"formats": ["%Y %H"]}))),
        ("parse_date", Some(json!({"formats": ["%Y%"]}))),
        ("parse_date", Some(json!({"formats": ["%Y"], "x": 1}))),
        ("parse_number", Some(json!(true))),
        ("parse_number", Some(json!({"units": "yes"}))),
        ("parse_number", Some(json!({"units": true, "x": 1}))),
        ("parse_range", None),
        ("parse_range", Some(json!("mid"))),
    ];
    for (name, arg) in bad {
        assert_eq!(
            builtin(name, arg.as_ref()),
            Err(ArgError::InvalidArgument),
            "{name} {arg:?}"
        );
    }
}

#[test]
fn twenty_two_default_transforms() {
    let names = canonic_core::default_transform_names();
    assert_eq!(names.len(), 22);
    assert_eq!(canonic_core::LANGUAGE_1_TRANSFORMS, 9);
    assert_eq!(names[canonic_core::LANGUAGE_1_TRANSFORMS], "regex");
}

fn schema() -> J {
    json!({"type": "object", "properties": {"slug": {"type": "string"}}})
}

fn spec(language: Option<u8>, transform: J) -> J {
    let mut s = json!({"provider": "acme", "endpoint": "orgs", "version": 1, "status": "promoted",
                       "fields": {"slug": {"from": "url", "transform": [transform]}}});
    if let Some(l) = language {
        s["language"] = json!(l);
    }
    s
}

#[test]
fn language_1_does_not_see_the_new_names() {
    for name in [
        "regex",
        "strip_prefix",
        "lookup",
        "upper",
        "to_string",
        "format_date",
        "parse_number",
        "parse_range",
    ] {
        for language in [None, Some(1)] {
            let err = compile(&spec(language, json!({name: "x"})), &schema(), &[]).unwrap_err();
            assert_eq!(
                err.message,
                format!("acme orgs v1: fields[\"slug\"]: unknown transform \"{name}\""),
            );
        }
    }
    // A custom transform with a new name still works in language 1.
    let plan = compile(&spec(None, json!("upper")), &schema(), &["upper".into()]).unwrap();
    assert_eq!(serde_json::to_value(&plan).unwrap()["format"], 1);
}

#[test]
fn language_1_parse_date_takes_no_argument() {
    let t = json!({"parse_date": {"formats": ["%m/%Y"]}});
    for language in [None, Some(1)] {
        let err = compile(&spec(language, t.clone()), &schema(), &[]).unwrap_err();
        assert_eq!(
            err.message,
            "acme orgs v1: fields[\"slug\"]: invalid argument for parse_date"
        );
    }
    let plan = compile(&spec(None, json!("parse_date")), &schema(), &[]).unwrap();
    assert_eq!(serde_json::to_value(&plan).unwrap()["format"], 1);
    let plan = compile(&spec(Some(2), json!("parse_date")), &schema(), &[]).unwrap();
    assert_eq!(serde_json::to_value(&plan).unwrap()["format"], 1);
}

#[test]
fn a_bad_pattern_is_a_compile_error() {
    let err = compile(&spec(Some(2), json!({"regex": "("})), &schema(), &[]).unwrap_err();
    assert_eq!(
        err.message,
        "acme orgs v1: fields[\"slug\"]: invalid argument for regex"
    );
}

#[test]
fn plans_with_new_builtins_round_trip_as_format_2() {
    let all = [
        json!({"regex": "/company/([^/?#]+)"}),
        json!({"strip_prefix": "urn:"}),
        json!({"strip_suffix": "/"}),
        json!({"replace": {"from": "-", "to": " "}}),
        json!({"replace": {"pattern": "[0-9]", "to": ""}}),
        json!({"lookup": {"table": {"A": "a", "B": 2}, "else": "keep"}}),
        json!({"null_if": ["", 0]}),
        json!("upper"),
        json!("title"),
        json!({"split": {"sep": "/", "index": -1}}),
        json!("to_string"),
        json!({"format_date": "datetime_ms"}),
        json!({"format_date": "epoch_s"}),
        json!({"parse_date": {"formats": ["%B %Y", "%m/%d/%Y"]}}),
        json!("parse_number"),
        json!({"parse_number": {"units": true}}),
        json!({"parse_range": "max"}),
    ];
    for t in all {
        let plan = compile(&spec(Some(2), t.clone()), &schema(), &[]).unwrap();
        let text = serde_json::to_string(&plan).unwrap();
        assert_eq!(
            serde_json::from_str::<J>(&text).unwrap()["format"],
            2,
            "{t}"
        );
        let back: Plan = serde_json::from_str(&text).unwrap();
        assert_eq!(back, plan, "{t}");
        assert_eq!(back.source(), plan.source(), "{t}");
    }
    let plan = compile(
        &spec(Some(2), json!({"regex": "/company/([^/?#]+)"})),
        &schema(),
        &[],
    )
    .unwrap();
    assert_eq!(
        plan.source(),
        "slug <- url | regex \"/company/([^/?#]+)\"\n"
    );
    let t = json!({"parse_date": {"formats": ["%B %Y"]}});
    let plan = compile(&spec(Some(2), t), &schema(), &[]).unwrap();
    assert_eq!(
        plan.source(),
        "slug <- url | parse_date {\"formats\":[\"%B %Y\"]}\n"
    );
}

/// Every new transform with a valid argument.
fn new_builtins() -> Vec<Builtin> {
    vec![
        tx("regex", Some(json!("(x+)"))),
        tx("strip_prefix", Some(json!("a"))),
        tx("strip_suffix", Some(json!("a"))),
        tx("replace", Some(json!({"from": "a", "to": "b"}))),
        tx("replace", Some(json!({"pattern": "a+", "to": ""}))),
        tx("lookup", Some(json!({"table": {"a": 1}, "else": "keep"}))),
        tx("lookup", Some(json!({"table": {"a": 1}, "else": "x"}))),
        tx("null_if", Some(json!(["", 0, null, [1], {"a": 1}]))),
        tx("upper", None),
        tx("title", None),
        tx("split", Some(json!({"sep": "a", "index": -1}))),
        tx("to_string", None),
        tx("format_date", Some(json!("datetime_ms"))),
        tx("format_date", Some(json!("epoch_ms"))),
        tx(
            "parse_date",
            Some(json!({"formats": ["%b %d %Y", "%m/%Y"]})),
        ),
        tx("parse_number", None),
        tx("parse_number", Some(json!({"units": true}))),
        tx("parse_range", Some(json!("min"))),
        tx("parse_range", Some(json!("max"))),
    ]
}

/// A JSON value as a transform input: an object or array is one `Container`.
fn leaf(j: &J) -> Value<'static> {
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap()),
        },
        J::String(x) => s(x),
        J::Array(_) | J::Object(_) => Value::Container,
    }
}

fn json_values() -> impl Strategy<Value = J> {
    let scalar = prop_oneof![
        Just(J::Null),
        any::<bool>().prop_map(J::from),
        any::<i64>().prop_map(J::from),
        any::<f64>().prop_map(J::from),
        any::<String>().prop_map(J::from),
    ];
    scalar.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(J::Array),
            prop::collection::btree_map(any::<String>(), inner, 0..4)
                .prop_map(|m| J::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    /// No panic on any input. Containers give null everywhere. The string transforms also give
    /// null on numbers and booleans. lookup, null_if and to_string read scalars, and the date,
    /// number and range transforms read numbers too.
    #[test]
    fn junk_gives_null(j in json_values()) {
        let v = leaf(&j);
        for b in new_builtins() {
            let out = b.apply(v.clone(), None);
            let scalar_ok = matches!(
                b,
                Builtin::Lookup { .. } | Builtin::NullIf(_) | Builtin::ToString
            );
            let number_ok = matches!(
                b,
                Builtin::FormatDate(_)
                    | Builtin::ParseDateFormats(_)
                    | Builtin::ParseNumber { .. }
                    | Builtin::ParseRange(_)
            );
            let junk = matches!(j, J::Null | J::Array(_) | J::Object(_))
                || (!scalar_ok && !j.is_string() && !(number_ok && j.is_number()));
            if junk {
                prop_assert_eq!(out, Value::Null, "{:?} on {}", b, j);
            }
            prop_assert_eq!(b.apply_list(std::slice::from_ref(&v)), Value::Null);
        }
    }
}
