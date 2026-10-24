// Port of the map cases of canonic/packages/canonic/test/compile.test.ts: every construct fixture,
// the stats cases, injected transforms, the large-spec cases, and the bench pin from the POC.

use canonic_core::{
    Artifact, CompileError, FieldStats, Out, Plan, RustSink, StatsSnapshot, Transform, Value,
    compile, filter_for,
};
use serde_json::{Value as J, json};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

fn fixture(name: &str) -> J {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

fn canonical() -> J {
    fixture("canonical.json")
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

/// All-null canonical object read from the JSON Schema, with `partial` laid over it (helpers.ts `full`).
fn full(partial: &J, schema: &J) -> J {
    let mut out = serde_json::Map::new();
    for (k, child) in schema["properties"].as_object().unwrap() {
        let v = partial.get(k);
        let nested = child.get("properties").is_some();
        out.insert(
            k.clone(),
            match (v, nested) {
                (Some(v), true) => full(v, child),
                (None, true) => full(&json!({}), child),
                (Some(v), false) => v.clone(),
                (None, false) => J::Null,
            },
        );
    }
    J::Object(out)
}

type Custom = HashMap<String, Arc<dyn Transform>>;

fn artifact_with(spec: &J, schema: &J, custom: Custom) -> Artifact {
    let names: Vec<String> = custom.keys().cloned().collect();
    Artifact::new(compile(spec, schema, &names).unwrap(), custom).unwrap()
}

fn artifact(spec: &J, schema: &J) -> Artifact {
    artifact_with(spec, schema, HashMap::new())
}

fn map(a: &Artifact, payload: &J) -> J {
    to_json(
        &a.map_bytes(payload.to_string().as_bytes(), None).unwrap(),
        &a.plan.names,
    )
}

fn spec(fields: J) -> J {
    json!({ "provider": "p", "endpoint": "e", "version": 1, "status": "promoted", "fields": fields })
}

fn fs_(fill: u64, errors: u64) -> FieldStats {
    FieldStats {
        fill,
        errors,
        total: None,
    }
}

fn snap(responses: u64, fields: &[(&str, FieldStats)]) -> StatsSnapshot {
    StatsSnapshot {
        responses,
        rejected: 0,
        fields: fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    }
}

struct F(for<'a> fn(Value<'a>) -> Value<'a>);

impl Transform for F {
    fn apply<'a>(&self, v: Value<'a>, _: Option<&J>) -> Value<'a> {
        (self.0)(v)
    }
}

fn custom(name: &str, f: for<'a> fn(Value<'a>) -> Value<'a>) -> Custom {
    HashMap::from([(name.to_string(), Arc::new(F(f)) as Arc<dyn Transform>)])
}

#[test]
fn every_construct_case_maps_to_the_full_canonical_shape() {
    let schema = canonical();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/constructs");
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let mut n = 0;
    for f in files {
        let c: J = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        let a = artifact(&c["spec"], &schema);
        for (i, k) in c["cases"].as_array().unwrap().iter().enumerate() {
            let got = map(&a, &k["payload"]);
            assert_eq!(
                got,
                full(&k["expected"], &schema),
                "{} case {i}",
                f.display()
            );
            // Schema order, not just equal maps.
            assert_eq!(
                got.to_string(),
                full(&k["expected"], &schema).to_string(),
                "{} case {i} key order",
                f.display()
            );
            n += 1;
        }
    }
    assert!(n > 50, "only {n} cases");
}

fn stats_spec() -> J {
    spec(json!({
        "email": { "from": "e", "transform": ["trim", "lower"] },
        "headcount": { "from": "n", "transform": { "cast": "integer" }, "default": 0 },
        "skills[]": { "from": "tags", "pluck": "l", "transform": "lower" },
        "experiences[]": { "from": "jobs", "item": { "title": { "from": "t", "transform": "trim" } } },
    }))
}

#[test]
fn stats_fill_and_errors_match_the_nulls_map_produced() {
    let a = artifact(&stats_spec(), &canonical());
    let payloads = [
        json!({ "e": " A@B.CO ", "n": "12", "tags": [{ "l": "X" }], "jobs": [{ "t": " cto " }] }),
        json!({ "e": "N/A", "n": "twelve", "tags": [{ "l": 5 }, { "l": "Y" }], "jobs": [{ "t": 7 }, { "t": 8 }] }),
        json!({ "tags": "nope", "jobs": {} }),
        json!({}),
    ];
    let outs: Vec<J> = payloads.iter().map(|p| map(&a, p)).collect();
    assert_eq!(outs[1]["email"], J::Null);
    assert_eq!(outs[1]["headcount"], json!(0));
    assert_eq!(outs[1]["skills"], json!([null, "y"]));
    assert_eq!(
        outs[1]["experiences"],
        json!([{ "title": null, "company": null, "tags": null }, { "title": null, "company": null, "tags": null }])
    );
    assert_eq!(
        a.stats.snapshot(),
        snap(
            4,
            &[
                ("email", fs_(1, 1)),
                ("headcount", fs_(1, 1)), // fill counts before the default applies
                ("skills[]", fs_(2, 2)),
                ("experiences[]", fs_(2, 1)),
                (
                    "experiences[].title",
                    FieldStats {
                        fill: 1,
                        errors: 2,
                        total: Some(3)
                    }
                ),
            ]
        )
    );
    a.stats.reset();
    let s = a.stats.snapshot();
    assert_eq!(s.responses, 0);
    assert_eq!(s.fields[0], ("email".to_string(), fs_(0, 0)));
}

#[test]
fn fields_inside_nested_items_count_against_their_own_arrays_elements() {
    let nested = json!({ "type": "object", "properties": { "orders": { "type": "array", "items": { "type": "object", "properties": { "id": {}, "lines": { "type": "array", "items": { "type": "object", "properties": { "sku": {} } } } } } } } });
    let fields = json!({ "orders[]": { "from": "o", "item": { "id": "id", "lines[]": { "from": "l", "item": { "sku": "s" } } } } });
    let a = artifact(&spec(fields), &nested);
    map(
        &a,
        &json!({ "o": [{ "id": 1, "l": [{ "s": "a" }, { "s": "b" }, {}] }, { "l": [] }, { "id": 3 }] }),
    );
    map(&a, &json!({}));
    let t = |fill, total| FieldStats {
        fill,
        errors: 0,
        total: Some(total),
    };
    assert_eq!(
        a.stats.snapshot(),
        snap(
            2,
            &[
                ("orders[]", fs_(1, 0)),
                ("orders[].id", t(2, 3)),
                ("orders[].lines[]", t(2, 3)),
                ("orders[].lines[].sku", t(2, 3)),
            ]
        )
    );
}

#[test]
fn a_custom_transform_that_panics_counts_as_an_error_and_yields_null() {
    let a = artifact_with(
        &spec(json!({ "first_name": { "from": "n", "transform": "boom" } })),
        &canonical(),
        custom("boom", |_| panic!("boom")),
    );
    // The default hook prints the panic to stderr. That noise is expected.
    let out = map(&a, &json!({ "n": "Ada" }));
    assert_eq!(out["first_name"], J::Null);
    assert_eq!(
        a.stats.snapshot().fields,
        vec![("first_name".into(), fs_(0, 1))]
    );
}

#[test]
fn map_counts_stats_once_per_call() {
    let a = artifact(&spec(json!({ "email": "e" })), &canonical());
    assert_eq!(a.stats.snapshot().responses, 0);
    map(&a, &json!({ "e": "a@b.co" }));
    assert_eq!(a.stats.snapshot(), snap(1, &[("email", fs_(1, 0))]));
}

#[test]
fn custom_transforms_are_called_by_name_and_can_replace_a_default() {
    fn strip(v: Value<'_>) -> Value<'_> {
        match v {
            Value::Str(s) => {
                let t = ["Dr. ", "Dr ", "Mr. ", "Mr ", "Mrs. ", "Mrs ", "Ms. ", "Ms "]
                    .iter()
                    .find_map(|p| s.strip_prefix(p))
                    .unwrap_or(&s)
                    .to_string();
                Value::Str(t.into())
            }
            _ => Value::Null,
        }
    }
    let a = artifact_with(
        &spec(json!({ "full_name": { "from": "name", "transform": ["trim", "strip_honorific"] } })),
        &canonical(),
        custom("strip_honorific", strip),
    );
    assert_eq!(
        map(&a, &json!({ "name": " Dr. Ada Lovelace " }))["full_name"],
        "Ada Lovelace"
    );
    let b = artifact_with(
        &spec(json!({ "first_name": { "from": "n", "transform": "trim" } })),
        &canonical(),
        custom("trim", |_| Value::Str("replaced".into())),
    );
    assert_eq!(map(&b, &json!({ "n": " Ada " }))["first_name"], "replaced");
}

#[test]
fn large_specs_map_every_field() {
    let mut properties = serde_json::Map::new();
    let mut fields = serde_json::Map::new();
    properties.insert(
        "skills".into(),
        json!({ "type": "array", "items": { "type": "string" } }),
    );
    fields.insert(
        "skills[]".into(),
        json!({ "from": "tags", "pluck": "label", "transform": { "cast": "string" } }),
    );
    for i in 0..60 {
        properties.insert(format!("n{i}"), json!({ "type": "number" }));
        fields.insert(
            format!("n{i}"),
            json!({ "from": format!("s{i}"), "transform": { "cast": "number" } }),
        );
    }
    properties.insert(
        "jobs".into(),
        json!({ "type": "array", "items": { "type": "object", "properties": { "t": {} } } }),
    );
    fields.insert(
        "jobs[]".into(),
        json!({ "from": "w", "item": { "t": { "from": "t", "transform": { "cast": "number" } } } }),
    );
    let a = artifact(
        &spec(J::Object(fields)),
        &json!({ "type": "object", "properties": properties }),
    );
    let out = map(
        &a,
        &json!({ "s59": "7", "tags": [{ "label": 1 }], "w": [{ "t": "2" }] }),
    );
    assert_eq!(
        [&out["n59"], &out["skills"], &out["jobs"]],
        [&json!(7), &json!(["1"]), &json!([{ "t": 2 }])]
    );
}

#[test]
fn twelve_groups_of_twenty_five_map_with_stats() {
    let mut properties = serde_json::Map::new();
    let mut fields = serde_json::Map::new();
    let mut src = serde_json::Map::new();
    for g in 0..12 {
        let mut inner = serde_json::Map::new();
        inner.insert(
            "tags".into(),
            json!({ "type": "array", "items": { "type": "string" } }),
        );
        inner.insert("unmapped".into(), json!({}));
        let mut s = serde_json::Map::new();
        for k in 0..25 {
            inner.insert(format!("f{k}"), json!({ "type": "number" }));
            fields.insert(
                format!("g{g}.f{k}"),
                json!({ "from": format!("src{g}.v{k}"), "transform": { "cast": "number" } }),
            );
            s.insert(format!("v{k}"), json!((g * 100 + k).to_string()));
        }
        properties.insert(
            format!("g{g}"),
            json!({ "type": "object", "properties": inner }),
        );
        fields.insert(
            format!("g{g}.tags[]"),
            json!({ "from": format!("src{g}.t"), "pluck": "l" }),
        );
        s.insert("t".into(), json!([{ "l": "x" }]));
        src.insert(format!("src{g}"), J::Object(s));
    }
    src["src3"]["v4"] = json!("N/A");
    let a = artifact(
        &spec(J::Object(fields)),
        &json!({ "type": "object", "properties": properties }),
    );
    let out = map(&a, &J::Object(src));
    let mut g11 = serde_json::Map::new();
    for k in 0..25 {
        g11.insert(format!("f{k}"), json!(1100 + k));
    }
    g11.insert("tags".into(), json!(["x"]));
    g11.insert("unmapped".into(), J::Null);
    assert_eq!(out["g11"], J::Object(g11));
    assert_eq!(out["g3"]["f4"], J::Null);
    let s = a.stats.snapshot();
    let g3f4 = s.fields.iter().find(|(k, _)| k == "g3.f4").unwrap();
    assert_eq!(g3f4.1, fs_(0, 1));
}

#[test]
fn bench_bodies_map_to_the_poc_outputs() {
    let a = artifact(
        &fixture("specs/bench.json"),
        &fixture("specs/canonical/bench.json"),
    );
    let bodies = fixture("bench_bodies.json");
    let raws = bodies["raws"].as_array().unwrap();
    let expected = bodies["expected"].as_array().unwrap();
    assert_eq!(raws.len(), 64);
    for (i, (raw, want)) in raws.iter().zip(expected).enumerate() {
        let out = a.map_bytes(raw.as_str().unwrap().as_bytes(), None).unwrap();
        let got = to_json(&out, &a.plan.names);
        assert_eq!(&got, want, "body {i}");
        assert_eq!(got.to_string(), want.to_string(), "body {i} key order");
    }
    assert_eq!(a.stats.snapshot().responses, 64);
}

#[test]
fn map_field_maps_one_field_by_index_path_and_counts_nothing() {
    let a = artifact(
        &fixture("specs/bench.json"),
        &fixture("specs/canonical/bench.json"),
    );
    let bodies = fixture("bench_bodies.json");
    let names = &a.plan.names;
    for raw in bodies["raws"].as_array().unwrap() {
        let node = a.parse(raw.as_str().unwrap().as_bytes());
        let doc = node.as_ref();
        let Ok(Out::Obj(whole)) = a.map_with(doc.as_ref(), None, &mut RustSink) else {
            panic!("map gives an object")
        };
        a.stats.reset();
        let root = a.map_field(&[], doc.as_ref(), &mut RustSink).unwrap();
        assert_eq!(
            to_json(&root, names),
            to_json(&Out::Obj(whole.clone()), names)
        );
        for (i, (_, want)) in whole.iter().enumerate() {
            let got = a.map_field(&[i], doc.as_ref(), &mut RustSink).unwrap();
            assert_eq!(&got, want, "field {i}");
            if let Out::Obj(members) = want {
                for (j, (_, w)) in members.iter().enumerate() {
                    assert_eq!(
                        a.map_field(&[i, j], doc.as_ref(), &mut RustSink).as_ref(),
                        Some(w)
                    );
                }
                assert_eq!(
                    a.map_field(&[i, members.len()], doc.as_ref(), &mut RustSink),
                    None
                );
            } else {
                // Only an object field has members.
                assert_eq!(a.map_field(&[i, 0], doc.as_ref(), &mut RustSink), None);
            }
        }
        assert_eq!(
            a.map_field(&[whole.len()], doc.as_ref(), &mut RustSink),
            None
        );
        let s = a.stats.snapshot();
        assert_eq!(s.responses, 0);
        assert!(
            s.fields
                .iter()
                .all(|(_, f)| f.fill == 0 && f.errors == 0 && f.total.is_none_or(|t| t == 0))
        );
    }
}

// The rules of codegen.ts, one at a time.

#[test]
fn missing_is_null_without_error_and_a_rejection_is_null_with_one() {
    let a = artifact(
        &spec(json!({ "headcount": { "from": "n", "transform": { "cast": "integer" } } })),
        &canonical(),
    );
    assert_eq!(map(&a, &json!({}))["headcount"], J::Null);
    assert_eq!(map(&a, &json!({ "n": null }))["headcount"], J::Null);
    assert_eq!(a.stats.snapshot().fields[0].1, fs_(0, 0));
    assert_eq!(map(&a, &json!({ "n": "x" }))["headcount"], J::Null);
    assert_eq!(map(&a, &json!({ "n": {} }))["headcount"], J::Null);
    assert_eq!(map(&a, &json!({ "n": "4" }))["headcount"], json!(4));
    assert_eq!(a.stats.snapshot().fields[0].1, fs_(1, 2));
}

#[test]
fn a_default_applies_on_missing_and_on_rejection_and_counts_a_miss() {
    let a = artifact(
        &spec(
            json!({ "headcount": { "from": "n", "transform": { "cast": "integer" }, "default": 7 },
                      "first_name": { "from": "f", "default": "anon" } }),
        ),
        &canonical(),
    );
    let out = map(&a, &json!({ "n": "x" }));
    assert_eq!(
        [&out["headcount"], &out["first_name"]],
        [&json!(7), &json!("anon")]
    );
    let s = a.stats.snapshot();
    assert_eq!(s.fields[0].1, fs_(0, 1));
    assert_eq!(s.fields[1].1, fs_(0, 0));
}

#[test]
fn coalesce_takes_the_first_present_value_and_does_not_retry_after_a_rejection() {
    let a = artifact(
        &spec(
            json!({ "headcount": { "coalesce": ["a", "b"], "transform": { "cast": "integer" } } }),
        ),
        &canonical(),
    );
    assert_eq!(
        map(&a, &json!({ "a": null, "b": "3" }))["headcount"],
        json!(3)
    );
    assert_eq!(
        map(&a, &json!({ "a": "x", "b": "3" }))["headcount"],
        J::Null
    );
    assert_eq!(a.stats.snapshot().fields[0].1, fs_(1, 1));
}

#[test]
fn from_many_without_a_transform_is_the_list_and_with_one_null_when_all_missing() {
    let a = artifact(
        &spec(
            json!({ "full_name": { "from": ["a", "b"] }, "first_name": { "from": ["a", "b"], "transform": "join" } }),
        ),
        &canonical(),
    );
    let out = map(&a, &json!({ "a": "Ada", "b": { "x": [1] } }));
    assert_eq!(out["full_name"], json!(["Ada", { "x": [1] }]));
    assert_eq!(out["first_name"], json!("Ada"));
    let out = map(&a, &json!({}));
    assert_eq!(
        [&out["full_name"], &out["first_name"]],
        [&J::Null, &J::Null]
    );
    let s = a.stats.snapshot();
    assert_eq!(s.fields[0].1, fs_(1, 0));
    assert_eq!(s.fields[1].1, fs_(1, 0));
}

#[test]
fn a_leaf_without_transforms_passes_containers_and_numbers_through() {
    let a = artifact(&spec(json!({ "full_name": "$" })), &canonical());
    let body = br#"{"o":{"k":[1,"two",null]},"big":12345678901234567890,"f":1.5,"e":1e2,"z":-0}"#;
    let out = a.map_bytes(body, None).unwrap();
    let Out::Obj(fields) = out else { panic!() };
    let name = a.plan.names.iter().position(|n| n == "full_name").unwrap();
    let got = &fields.iter().find(|(i, _)| *i == name).unwrap().1;
    assert_eq!(
        got,
        &Out::Map(vec![
            (
                "o".into(),
                Out::Map(vec![(
                    "k".into(),
                    Out::List(vec![Out::Int(1), Out::Str("two".into()), Out::Null])
                )])
            ),
            ("big".into(), Out::BigInt("12345678901234567890".into())),
            ("f".into(), Out::Float(1.5)),
            ("e".into(), Out::Float(100.0)),
            ("z".into(), Out::Int(0)),
        ])
    );
}

#[test]
fn an_unparseable_body_maps_to_all_null_and_counts_misses() {
    let a = artifact(
        &spec(json!({ "email": "e", "skills[]": { "from": "t" } })),
        &canonical(),
    );
    let out = to_json(&a.map_bytes(b"{nope", None).unwrap(), &a.plan.names);
    assert_eq!(out, full(&json!({}), &canonical()));
    assert_eq!(
        a.stats.snapshot(),
        snap(1, &[("email", fs_(0, 0)), ("skills[]", fs_(0, 0))])
    );
}

#[test]
fn an_index_path_and_a_list_over_the_same_array_both_read() {
    let s = spec(json!({
        "email": { "from": "emails[0].address" },
        "skills[]": { "from": "emails", "pluck": "type" },
    }));
    let a = artifact(&s, &canonical());
    let out = map(
        &a,
        &json!({ "emails": [{ "address": "a@b.co", "type": "work" }, { "type": "home" }] }),
    );
    assert_eq!(out["email"], "a@b.co");
    assert_eq!(out["skills"], json!(["work", "home"]));
    let f = filter_for(&a.plan);
    let emails = &f.keys()[0].1;
    assert_eq!(emails.idxs[0].1.keys().len(), 2, "{f:?}");
}

fn plan() -> Plan {
    compile(
        &spec(json!({ "first_name": { "from": "n", "transform": "boom" } })),
        &canonical(),
        &["boom".to_string()],
    )
    .unwrap()
}

fn new_err(p: Plan, c: Custom) -> String {
    match Artifact::new(p, c) {
        Ok(_) => panic!("accepted"),
        Err(CompileError { message }) => message,
    }
}

#[test]
fn artifact_new_rejects_out_of_range_indexes_and_unregistered_transforms() {
    let boom = || custom("boom", |v| v);
    assert!(Artifact::new(plan(), boom()).is_ok());
    assert_eq!(
        new_err(plan(), HashMap::new()),
        r#"p e v1: transform "boom" is not registered"#
    );
    let mut p = plan();
    p.fields[0].name_idx = 999;
    assert!(new_err(p, boom()).contains("name index 999"));
    let mut p = plan();
    p.stat_names.clear();
    assert!(new_err(p, boom()).contains("stat slot 0"));
    let mut p = plan();
    p.custom_names.clear();
    assert!(new_err(p, HashMap::new()).contains("custom transform 0"));
}

#[test]
fn a_plain_list_element_a_transform_rejects_is_null_in_place_with_one_error() {
    let a = artifact(
        &spec(json!({ "skills[]": { "from": "t", "transform": "lower" } })),
        &canonical(),
    );
    let out = map(&a, &json!({ "t": ["A", 7, null, "B"] }));
    assert_eq!(out["skills"], json!(["a", null, null, "b"]));
    assert_eq!(a.stats.snapshot().fields[0].1, fs_(1, 1));
}

#[test]
fn a_passed_through_object_keeps_the_last_duplicate_at_the_first_position() {
    let a = artifact(&spec(json!({ "full_name": "$" })), &canonical());
    let out = map_raw(&a, br#"{"a":1,"b":2,"a":3}"#);
    assert_eq!(out["full_name"].to_string(), r#"{"a":3,"b":2}"#);
}

#[test]
fn passing_through_an_object_with_many_keys_is_linear() {
    let a = artifact(&spec(json!({ "full_name": "$" })), &canonical());
    let n = 160_000;
    let mut body = b"{".to_vec();
    for i in 0..n {
        body.extend(format!("\"k{i}\":{i},").as_bytes());
    }
    body.extend(b"\"k0\":-1}");
    let started = std::time::Instant::now();
    let out = map_raw(&a, &body);
    let elapsed = started.elapsed();
    let obj = out["full_name"].as_object().unwrap();
    assert_eq!(obj.len(), n);
    assert_eq!(obj["k0"], json!(-1));
    // A quadratic run takes minutes. Wall clock is only checked when asked (CI's timing job).
    if std::env::var_os("PYCANONIC_TIMING_TESTS").is_some_and(|v| v == "1") {
        assert!(elapsed.as_secs() < 10, "took {elapsed:?}");
    }
}

fn map_raw(a: &Artifact, body: &[u8]) -> J {
    to_json(&a.map_bytes(body, None).unwrap(), &a.plan.names)
}

/// Renders what it is given as JSON text, so a test can see a container input.
struct Show;

impl Transform for Show {
    fn apply<'a>(&self, v: Value<'a>, _: Option<&J>) -> Value<'a> {
        Value::Str(format!("{v:?}").into())
    }

    fn apply_container(&self, v: &J, arg: Option<&J>) -> Value<'static> {
        Value::Str(format!("{v} {}", arg.unwrap_or(&J::Null)).into())
    }
}

#[test]
fn a_custom_transform_sees_a_container_input_as_json() {
    let custom: Custom =
        HashMap::from([("show".to_string(), Arc::new(Show) as Arc<dyn Transform>)]);
    let a = artifact_with(
        &spec(json!({
            "full_name": { "from": ["a", "b.c", "x"], "transform": "show" },
            "first_name": { "from": "o", "transform": { "show": 1 } },
            "last_name": { "from": "s", "transform": "show" },
            "skills[]": { "from": "l", "transform": "show" },
        })),
        &canonical(),
        custom,
    );
    let out = map_raw(
        &a,
        br#"{"a":"x","b":{"c":[2,{"d":null}]},"o":{"k":1,"n":{"m":true}},"s":"t","l":[{"q":1},3]}"#,
    );
    assert_eq!(out["full_name"], r#"["x",[2,{"d":null}],null] null"#);
    assert_eq!(out["first_name"], r#"{"k":1,"n":{"m":true}} 1"#);
    assert_eq!(out["last_name"], r#"Str("t")"#);
    assert_eq!(out["skills"], json!([r#"{"q":1} null"#, "Int(3)"]));
}

#[test]
fn a_custom_transform_without_apply_container_gets_a_container_value_as_before() {
    let a = artifact_with(
        &spec(json!({ "full_name": { "from": "o", "transform": "same" } })),
        &canonical(),
        custom("same", |v| v),
    );
    assert_eq!(
        map_raw(&a, br#"{"o":{"k":1}}"#)["full_name"],
        json!({ "k": 1 })
    );
}
