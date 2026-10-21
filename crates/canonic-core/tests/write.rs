// The JSON writer: `map_json` against `map_bytes` and the POC outputs, plus the escaping and number rules.

use canonic_core::{Artifact, Out, compile, write_json};
use serde_json::{Value as J, json};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> J {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

fn bench() -> Artifact {
    let plan = compile(
        &fixture("specs/bench.json"),
        &fixture("specs/canonical/bench.json"),
        &[],
    )
    .unwrap();
    Artifact::new(plan, HashMap::new()).unwrap()
}

/// JavaScript has one number type: a whole Float compares equal to the integer JSON.stringify writes.
fn num(f: f64) -> J {
    if f.fract() == 0.0 && f.abs() < 9e15 {
        json!(f as i64)
    } else {
        serde_json::Number::from_f64(f).map_or(J::Null, J::Number)
    }
}

fn to_json(o: &Out, names: &[String]) -> J {
    match o {
        Out::Null => J::Null,
        Out::Bool(b) => json!(b),
        Out::Int(i) => json!(i),
        Out::BigInt(s) => num(s.parse().unwrap()),
        Out::Float(f) => num(*f),
        Out::Str(s) => json!(s),
        Out::List(xs) => J::Array(xs.iter().map(|x| to_json(x, names)).collect()),
        Out::Obj(m) => J::Object(
            m.iter()
                .map(|(i, x)| (names[*i].clone(), to_json(x, names)))
                .collect(),
        ),
        Out::Map(m) => J::Object(
            m.iter()
                .map(|(k, x)| (k.clone(), to_json(x, names)))
                .collect(),
        ),
    }
}

fn text(a: &Artifact, o: &Out) -> String {
    let mut buf = Vec::new();
    write_json(&a.plan, o, &mut buf);
    String::from_utf8(buf).unwrap()
}

#[test]
fn map_json_matches_map_bytes_and_the_poc_outputs_in_order() {
    let a = bench();
    let bodies = fixture("bench_bodies.json");
    let raws = bodies["raws"].as_array().unwrap();
    let expected = bodies["expected"].as_array().unwrap();
    assert_eq!(raws.len(), 64);
    for (i, (raw, want)) in raws.iter().zip(expected).enumerate() {
        let body = raw.as_str().unwrap().as_bytes();
        let got: J = serde_json::from_slice(&a.map_json(body)).unwrap();
        let tree = to_json(&a.map_bytes(body), &a.plan.names);
        assert_eq!(got, tree, "body {i}");
        assert_eq!(&got, want, "body {i}");
        assert_eq!(got.to_string(), want.to_string(), "body {i} key order");
    }
}

#[test]
fn unparseable_body_writes_the_all_null_object() {
    let a = bench();
    let got: J = serde_json::from_slice(&a.map_json(b"{nope")).unwrap();
    assert_eq!(got, to_json(&a.map_bytes(b"{nope"), &a.plan.names));
}

#[test]
fn strings_escape_per_rfc_8259_and_round_trip() {
    let a = bench();
    let s = "q\"b\\n\nr\rt\tb\u{8}f\u{c}c\u{1}d\u{1f}e\u{7f}é\u{2028}😀";
    let t = text(&a, &Out::Str(s.into()));
    assert_eq!(
        t,
        "\"q\\\"b\\\\n\\nr\\rt\\tb\\bf\\fc\\u0001d\\u001fe\u{7f}é\u{2028}😀\""
    );
    assert_eq!(serde_json::from_str::<J>(&t).unwrap(), json!(s));
}

#[test]
fn map_keys_use_the_same_escaper() {
    let a = bench();
    let o = Out::Map(vec![
        ("a\"\n".into(), Out::Int(1)),
        (
            "b".into(),
            Out::List(vec![Out::Null, Out::Bool(true), Out::Bool(false)]),
        ),
        ("c".into(), Out::Map(vec![])),
        ("d".into(), Out::List(vec![])),
    ]);
    assert_eq!(
        text(&a, &o),
        r#"{"a\"\n":1,"b":[null,true,false],"c":{},"d":[]}"#
    );
}

#[test]
fn obj_keys_come_from_plan_names() {
    let a = bench();
    let o = Out::Obj(vec![(0, Out::Str("x".into())), (1, Out::Obj(vec![]))]);
    let want = format!(
        "{{{}:\"x\",{}:{{}}}}",
        json!(a.plan.names[0]),
        json!(a.plan.names[1])
    );
    assert_eq!(text(&a, &o), want);
}

#[test]
fn numbers_follow_json_stringify() {
    let a = bench();
    let cases: &[(Out, &str)] = &[
        (Out::Float(-0.0), "0"),
        (Out::Float(1.5e-7), "1.5e-7"),
        (Out::Float(1e21), "1e+21"),
        (Out::Float(2.5), "2.5"),
        (Out::Float(f64::NAN), "null"),
        (Out::Float(f64::INFINITY), "null"),
        (Out::Float(f64::NEG_INFINITY), "null"),
        (Out::Int(0), "0"),
        (Out::Int(-42), "-42"),
        (Out::Int(i64::MAX), "9223372036854775807"),
        (Out::Int(i64::MIN), "-9223372036854775808"),
        (
            Out::BigInt("12345678901234567890".into()),
            "12345678901234567890",
        ),
    ];
    for (o, want) in cases {
        assert_eq!(text(&a, o), *want, "{o:?}");
    }
}
