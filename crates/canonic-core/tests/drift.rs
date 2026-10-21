// Port of canonic/packages/canonic/test/drift.test.ts, plus fingerprints pinned against the
// TypeScript dist so the two implementations agree.

use canonic_core::json::Node;
use canonic_core::{
    AlertKind, Artifact, FieldAlert, FieldStats, FillMonitor, FillMonitorOptions, Filter,
    StatsSnapshot, compile, fingerprint, hash_shape, parse, shape, shape_bounded,
};
use proptest::prelude::*;
use serde_json::{Value as J, json};
use std::collections::HashMap;

fn all() -> Filter {
    Filter::all()
}

fn with<T>(v: &J, f: impl FnOnce(&&Node<'_>) -> T) -> T {
    let text = v.to_string();
    let node = parse(text.as_bytes(), &all()).unwrap();
    f(&&node)
}

fn shape_of(v: &J) -> Vec<String> {
    with(v, |d| shape(d))
}

fn fp(v: &J) -> String {
    with(v, |d| fingerprint(d))
}

fn body() -> J {
    json!({ "id": 1, "contact": { "workEmail": "a@b.co", "mobile": null }, "jobs": [{ "title": "x" }], "tags": [] })
}

#[test]
fn fingerprint_matches_the_typescript_function() {
    // cd canonic && node -e 'import("./packages/canonic/dist/index.js").then(m=>{console.log(m.fingerprint({"a":1,"b":[{"c":2}]}), JSON.stringify(m.shape({"a":1,"b":[{"c":2}]})))})'
    // 6a2ab0d3 ["\"a\"","\"b\"[]","\"b\"[].\"c\""]
    let v = json!({ "a": 1, "b": [{ "c": 2 }] });
    assert_eq!(shape_of(&v), [r#""a""#, r#""b"[]"#, r#""b"[]."c""#]);
    assert_eq!(fp(&v), "6a2ab0d3");
}

#[test]
fn fingerprint_hashes_utf16_code_units_and_sorts_by_them() {
    // cd canonic && node -e 'import("./packages/canonic/dist/index.js").then(m=>{const d={"\u{1F600}":1,"\uFF01":{"x":[{"y":null,"z":2}]},"q\"\n":true};console.log(m.fingerprint(d), JSON.stringify(m.shape(d)))})'
    // 2297bcc6 ["\"q\\\"\\n\"","\"😀\"","\"！\"","\"！\".\"x\"[]","\"！\".\"x\"[].\"z\""]
    // The emoji is two code units (0xd83d first), so it sorts before U+FF01, unlike in UTF-8 order.
    let v = json!({ "\u{1F600}": 1, "\u{FF01}": { "x": [{ "y": null, "z": 2 }] }, "q\"\n": true });
    assert_eq!(
        shape_of(&v),
        [
            r#""q\"\n""#,
            "\"\u{1F600}\"",
            "\"\u{FF01}\"",
            "\"\u{FF01}\".\"x\"[]",
            "\"\u{FF01}\".\"x\"[].\"z\""
        ]
    );
    assert_eq!(fp(&v), "2297bcc6");
}

#[test]
fn key_order_null_values_and_array_contents_dont_change_the_fingerprint() {
    let same = json!({ "tags": [1, 2, 3], "jobs": [{ "title": "y" }, { "title": null }], "contact": { "workEmail": "z@y.co" }, "id": 99, "extra": null });
    assert_eq!(
        shape_of(&body()),
        [
            r#""contact""#,
            r#""contact"."workEmail""#,
            r#""id""#,
            r#""jobs"[]"#,
            r#""jobs"[]."title""#,
            r#""tags"[]"#
        ]
    );
    assert_eq!(fp(&same), fp(&body()));
}

#[test]
fn a_key_renamed_inside_array_elements_changes_it_count_and_order_dont() {
    let jobs = |key: &str, n: usize| {
        let items: Vec<J> = (0..n)
            .map(|i| {
                let mut m = serde_json::Map::new();
                m.insert(key.into(), json!(format!("r{i}")));
                m.insert("org".into(), json!("Acme"));
                J::Object(m)
            })
            .collect();
        json!({ "jobs": items })
    };
    assert_eq!(
        shape_of(&jobs("role", 2)),
        [r#""jobs"[]"#, r#""jobs"[]."org""#, r#""jobs"[]."role""#]
    );
    assert_eq!(fp(&jobs("role", 1)), fp(&jobs("role", 5)));
    assert_ne!(fp(&jobs("title", 2)), fp(&jobs("role", 2)));
}

#[test]
fn a_renamed_or_moved_key_changes_it() {
    let mut renamed = body();
    renamed["contact"] = json!({ "email": "a@b.co" });
    assert_ne!(fp(&renamed), fp(&body()));
    let mut moved = body();
    moved["workEmail"] = json!("a@b.co");
    moved["contact"] = json!({});
    assert_ne!(fp(&moved), fp(&body()));
}

#[test]
fn keys_that_contain_dots_or_quotes_cant_collide_with_nesting() {
    assert_ne!(fp(&json!({ "a.b": 1 })), fp(&json!({ "a": { "b": 1 } })));
    assert_eq!(
        shape_of(&json!({ "a\"": { "b": 1 } })),
        [r#""a\"""#, r#""a\""."b""#]
    );
}

#[test]
fn scalars_have_an_empty_shape() {
    assert!(shape_of(&json!("text")).is_empty());
}

fn json_value() -> impl Strategy<Value = J> {
    let leaf = prop_oneof![
        Just(J::Null),
        any::<bool>().prop_map(J::from),
        any::<i64>().prop_map(J::from),
        ".*".prop_map(J::from),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(J::Array),
            prop::collection::vec((".*", inner), 0..6)
                .prop_map(|kv| J::Object(kv.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn fingerprint_is_eight_hex_digits_for_any_json(v in json_value()) {
        let f = fp(&v);
        prop_assert!(f.len() == 8 && f.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }
}

#[test]
fn a_response_that_is_itself_an_array_shapes_like_any_other_array() {
    assert_eq!(
        shape_of(&json!([{ "a": 1 }, { "b": null, "c": [] }])),
        ["[]", r#"[]."a""#, r#"[]."c"[]"#]
    );
    assert_eq!(
        fp(&json!([{ "a": 1 }])),
        fp(&json!([{ "a": 2 }, { "a": 3 }]))
    );
    assert_ne!(fp(&json!([{ "renamed": 1 }])), fp(&json!([{ "a": 1 }])));
    assert_ne!(fp(&json!([])), fp(&json!({})));
}

fn snap(responses: u64, fields: &[(&str, u64, u64)]) -> StatsSnapshot {
    StatsSnapshot {
        responses,
        fields: fields
            .iter()
            .map(|&(f, fill, errors)| {
                (
                    f.to_string(),
                    FieldStats {
                        fill,
                        errors,
                        total: None,
                    },
                )
            })
            .collect(),
    }
}

fn monitor() -> FillMonitor {
    FillMonitor::new(FillMonitorOptions::default())
}

fn alert(field: &str, kind: AlertKind, rate: f64, baseline: f64, responses: u64) -> FieldAlert {
    FieldAlert {
        field: field.into(),
        kind,
        rate,
        baseline,
        responses,
    }
}

fn artifact(spec: J, canonical: J) -> Artifact {
    Artifact::new(compile(&spec, &canonical, &[]).unwrap(), HashMap::new()).unwrap()
}

/// Maps each body, then checks the window and resets, as a caller does once per period.
fn period(a: &Artifact, m: &mut FillMonitor, bodies: impl Iterator<Item = J>) -> Vec<FieldAlert> {
    for b in bodies {
        a.map_bytes(b.to_string().as_bytes());
    }
    let s = a.stats.snapshot();
    a.stats.reset();
    m.check(&s)
}

#[test]
fn options_default_as_in_drift_ts() {
    let o = FillMonitorOptions::default();
    assert_eq!(
        (o.min_responses, o.history, o.sigmas, o.min_drop, o.min_rise),
        (200, 24, 4.0, 0.25, 0.01)
    );
}

#[test]
fn a_simulated_field_rename_trips_the_fill_alert_once_the_window_reaches_the_sample_floor() {
    let spec = json!({ "provider": "acme", "endpoint": "person.enrich", "version": 1, "status": "promoted", "fields": { "email": "workEmail", "first_name": "name" } });
    let canonical = json!({ "type": "object", "properties": { "email": { "type": "string", "semantic": "email", "routing": true }, "first_name": { "type": "string" } } });
    let a = artifact(spec, canonical);
    let mut m = FillMonitor::new(FillMonitorOptions {
        min_responses: 300,
        ..FillMonitorOptions::default()
    });
    let before = |i: usize| json!({ "workEmail": if !i.is_multiple_of(10) { json!(format!("u{i}@acme.com")) } else { J::Null }, "name": "Ada" });
    for _ in 0..5 {
        assert_eq!(period(&a, &mut m, (0..300).map(before)), []);
    }
    // The provider renames workEmail. 100 responses are below the floor, so nothing is judged yet.
    let after = |i: usize| json!({ "email": format!("u{i}@acme.com"), "name": "Ada" });
    assert_eq!(period(&a, &mut m, (0..100).map(after)), []);
    assert_eq!(
        period(&a, &mut m, (0..200).map(after)),
        [alert("email", AlertKind::Fill, 0.0, 0.9, 300)]
    );
    // The alert keeps firing: the bad window never joins the baseline.
    let again = period(&a, &mut m, (0..300).map(after));
    assert_eq!(
        again.iter().map(|x| x.field.as_str()).collect::<Vec<_>>(),
        ["email"]
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]
    #[test]
    fn steady_fields_with_ordinary_noise_never_alert(fills in prop::collection::vec(850u64..=950, 30)) {
        let mut m = monitor();
        for fill in fills {
            prop_assert!(m.check(&snap(1000, &[("f", fill, 0)])).is_empty());
        }
    }
}

#[test]
fn a_rarely_filled_field_that_goes_to_zero_alerts() {
    let mut m = monitor();
    for _ in 0..10 {
        m.check(&snap(2000, &[("rare", 100, 0)]));
    }
    let got = m.check(&snap(2000, &[("rare", 0, 0)]));
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].field.as_str(), got[0].kind, got[0].baseline),
        ("rare", AlertKind::Fill, 0.05)
    );
}

#[test]
fn a_small_dip_below_the_minimum_drop_doesnt_alert_even_when_its_statistically_clear() {
    let mut m = monitor();
    for _ in 0..10 {
        m.check(&snap(100_000, &[("f", 90_000, 0)]));
    }
    assert_eq!(m.check(&snap(100_000, &[("f", 85_000, 0)])), []);
}

#[test]
fn a_transform_that_starts_failing_raises_an_error_alert() {
    let mut m = monitor();
    for _ in 0..10 {
        m.check(&snap(1000, &[("date", 990, 1)]));
    }
    let got = m.check(&snap(1000, &[("date", 940, 50)]));
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].field.as_str(), got[0].kind, got[0].rate),
        ("date", AlertKind::Errors, 0.05)
    );
}

#[test]
fn a_key_renamed_inside_list_elements_trips_a_fill_alert_on_the_item_field() {
    let canonical = json!({ "type": "object", "properties": { "jobs": { "type": "array", "items": { "type": "object", "properties": { "title": { "type": "string" }, "org": { "type": "string" } } } } } });
    let spec = json!({ "provider": "acme", "endpoint": "jobs.search", "version": 1, "status": "promoted", "fields": { "jobs[]": { "from": "results", "item": { "title": "role", "org": "org" } } } });
    let a = artifact(spec, canonical);
    let mut m = monitor();
    let mut period_of = |key: &str| {
        let el = json!({ key: "CTO", "org": "Acme" });
        let b = json!({ "results": vec![el; 10] });
        period(&a, &mut m, std::iter::repeat_n(b, 50))
    };
    // 50 responses a window is under the floor of 200, so windows merge until 200 responses (2,000 elements).
    for _ in 0..8 {
        assert_eq!(period_of("role"), []);
    }
    for _ in 0..3 {
        assert_eq!(period_of("position"), []);
    }
    // The array itself is as full as ever; only its elements lost the key.
    assert_eq!(
        period_of("position"),
        [alert("jobs[].title", AlertKind::Fill, 0.0, 1.0, 2000)]
    );
}

#[test]
fn an_item_field_is_judged_only_in_windows_with_enough_elements() {
    let mut m = monitor();
    let w = |fill: u64, total: u64| StatsSnapshot {
        responses: 1000,
        fields: vec![(
            "jobs[].title".into(),
            FieldStats {
                fill,
                errors: 0,
                total: Some(total),
            },
        )],
    };
    for _ in 0..5 {
        m.check(&w(900, 1000));
    }
    assert_eq!(m.check(&w(0, 3)), []);
    assert_eq!(
        m.check(&w(0, 300)),
        [alert("jobs[].title", AlertKind::Fill, 0.0, 0.9, 300)]
    );
}

#[test]
fn plucked_array_errors_are_counted_per_element_so_a_rate_above_1_still_alerts() {
    let mut m = monitor();
    let w = |errors: u64| snap(300, &[("tags[]", 300, errors)]);
    for _ in 0..5 {
        assert_eq!(m.check(&w(360)), []); // 1.2 rejected elements per response
    }
    let got = m.check(&w(1500));
    assert_eq!(got.len(), 1);
    assert_eq!(
        (
            got[0].field.as_str(),
            got[0].kind,
            got[0].rate,
            got[0].baseline
        ),
        ("tags[]", AlertKind::Errors, 5.0, 1.2)
    );
}

#[test]
fn the_first_window_only_sets_the_baseline() {
    assert_eq!(monitor().check(&snap(1000, &[("f", 0, 1000)])), []);
}

#[test]
fn a_large_parsed_document_is_never_cut_and_the_bounded_walk_agrees() {
    let mut items: Vec<J> = (0..600_000).map(|i| json!({ "id": i, "v": "x" })).collect();
    items.push(json!({ "late": 1 }));
    let v = json!({ "a": items });
    let expected = [
        r#""a"[]"#,
        r#""a"[]."id""#,
        r#""a"[]."late""#,
        r#""a"[]."v""#,
    ];
    assert_eq!(shape_of(&v), expected);
    with(&v, |d| {
        let bounded = shape_bounded(d).unwrap();
        assert_eq!(bounded, expected);
        assert_eq!(hash_shape(&bounded), fingerprint(d));
    });
}
