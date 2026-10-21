// Port of canonic/packages/canonic/test/shadow.test.ts, plus every merge_gate reason string. The
// extra cases (index-key order, int vs float, routing regressions, toFixed ties) were checked
// against packages/canonic/dist/index.js and give the same output there.

use canonic_core::json::Node;
use canonic_core::{
    AlertKind, Artifact, FieldAlert, FieldDiff, Filter, GateOptions, GateResult, Out, ShadowDiff,
    ShadowReport, compile, merge_gate, parse, to_out_bounded,
};
use serde_json::{Value as J, json};
use std::collections::HashMap;

fn all() -> Filter {
    Filter {
        all: true,
        ..Filter::default()
    }
}

/// One response: both sides given as JSON, read as parsed documents.
fn add(d: &mut ShadowDiff, promoted: &J, candidate: &J) {
    add_text(d, &promoted.to_string(), &candidate.to_string());
}

fn add_text(d: &mut ShadowDiff, promoted: &str, candidate: &str) {
    let p = parse(promoted.as_bytes(), &all()).unwrap();
    let c = parse(candidate.as_bytes(), &all()).unwrap();
    d.add(&&p, &&c);
}

/// A canonical value as an `Out`, keys by name.
fn out(v: &J) -> Out {
    let text = v.to_string();
    let node: Node<'_> = parse(text.as_bytes(), &all()).unwrap();
    to_out_bounded(&&node).unwrap()
}

fn diff(
    compared: u64,
    same: u64,
    improvements: u64,
    regressions: u64,
    conflicts: u64,
) -> FieldDiff {
    FieldDiff {
        compared,
        same,
        improvements,
        regressions,
        conflicts,
    }
}

fn field<'a>(r: &'a ShadowReport, path: &str) -> &'a FieldDiff {
    &r.fields.iter().find(|(p, _)| p == path).unwrap().1
}

fn result(merge: bool, reasons: &[&str]) -> GateResult {
    GateResult {
        merge,
        reasons: reasons.iter().map(|s| s.to_string()).collect(),
    }
}

fn gate(r: &ShadowReport) -> GateResult {
    merge_gate(r, &GateOptions::default())
}

fn canonical() -> J {
    json!({
        "type": "object",
        "properties": {
            "email": { "type": "string", "semantic": "email", "routing": true },
            "title": { "type": "string" },
            "patent_count": { "type": "integer" },
            "company": { "type": "object", "properties": { "name": { "type": "string" } } },
            "skills": { "type": "array", "items": { "type": "string" } },
        },
    })
}

fn base() -> serde_json::Map<String, J> {
    json!({
        "email": "workEmail",
        "title": "role",
        "patent_count": { "from": "patents", "transform": { "cast": "integer" } },
        "company.name": "org.name",
        "skills[]": { "from": "tags", "pluck": "label" },
    })
    .as_object()
    .unwrap()
    .clone()
}

fn compile_spec(fields: serde_json::Map<String, J>, version: u32) -> Artifact {
    let status = if version == 1 { "promoted" } else { "shadow" };
    let spec = json!({ "provider": "acme", "endpoint": "person.enrich", "version": version, "status": status, "fields": fields });
    Artifact::new(compile(&spec, &canonical(), &[]).unwrap(), HashMap::new()).unwrap()
}

fn promoted() -> Artifact {
    compile_spec(base(), 1)
}

fn with(mut fields: serde_json::Map<String, J>, key: &str, v: J) -> serde_json::Map<String, J> {
    fields.insert(key.into(), v);
    fields
}

fn routing(a: &Artifact) -> Vec<String> {
    a.plan.routing.iter().map(|r| r.name.clone()).collect()
}

/// 1,000 responses; one in 200 has a patent count, so that field is rarely filled.
fn bodies(shape: impl Fn(usize) -> J) -> Vec<J> {
    (0..1000)
        .map(|i| {
            let mut b = json!({ "role": format!("Role {i}"), "dept": format!("Dept {}", i % 7), "org": { "name": "Acme" }, "tags": [{ "label": "sql" }], "patents": if i % 200 != 0 { J::Null } else { json!(3) } });
            for (k, v) in shape(i).as_object().unwrap() {
                b[k] = v.clone();
            }
            b
        })
        .collect()
}

fn map(a: &Artifact, body: &J) -> String {
    String::from_utf8(a.map_json(body.to_string().as_bytes())).unwrap()
}

fn soak(promoted: &Artifact, candidate: &Artifact, responses: &[J]) -> ShadowReport {
    let mut d = ShadowDiff::new(routing(promoted));
    for r in responses {
        add_text(&mut d, &map(promoted, r), &map(candidate, r));
    }
    d.snapshot()
}

fn known(candidate: &J, expected: &J) -> (Out, Out) {
    (out(candidate), out(expected))
}

#[test]
fn classifies_each_field_and_skips_responses_where_both_sides_are_null() {
    let mut d = ShadowDiff::new(vec!["email".into()]);
    let row = |email: J, title: J, skills: J| json!({ "email": email, "title": title, "company": { "name": "Acme" }, "skills": skills });
    add(
        &mut d,
        &row(J::Null, json!("CTO"), json!(["a"])),
        &row(json!("a@b.co"), json!("CTO"), json!(["a"])),
    );
    add(
        &mut d,
        &row(json!("a@b.co"), json!("CTO"), json!(["a"])),
        &row(J::Null, json!("VP"), json!(["a", "b"])),
    );
    add(
        &mut d,
        &row(J::Null, J::Null, J::Null),
        &row(J::Null, J::Null, J::Null),
    );
    assert_eq!(
        d.snapshot(),
        ShadowReport {
            responses: 3,
            routing: vec!["email".into()],
            fields: vec![
                ("email".into(), diff(2, 0, 1, 1, 0)),
                ("title".into(), diff(2, 1, 0, 0, 1)),
                ("company.name".into(), diff(3, 3, 0, 0, 0)),
                ("skills".into(), diff(2, 1, 0, 0, 1)),
            ],
        }
    );
}

#[test]
fn inside_arrays_key_order_doesnt_count_and_a_null_key_equals_a_missing_one() {
    let mut d = ShadowDiff::new(vec![]);
    // The left side is what hand-written code returned; the right is map() output for the same body.
    add(
        &mut d,
        &json!({ "jobs": [{ "title": "CTO", "org": { "name": "Acme" } }] }),
        &json!({ "jobs": [{ "org": { "name": "Acme", "domain": null }, "title": "CTO", "start": null }] }),
    );
    add(
        &mut d,
        &json!({ "jobs": [{ "title": "CTO" }] }),
        &json!({ "jobs": [{ "title": "CTO", "org": { "name": null } }] }),
    );
    add(
        &mut d,
        &json!({ "jobs": [{ "title": "CTO" }] }),
        &json!({ "jobs": [{ "title": "CEO" }] }),
    );
    add(
        &mut d,
        &json!({ "jobs": [{ "title": "CTO" }] }),
        &json!({ "jobs": [{ "title": "CTO" }, { "title": "CTO" }] }),
    );
    assert_eq!(field(&d.snapshot(), "jobs"), &diff(4, 2, 0, 0, 2));
}

#[test]
fn a_list_whose_elements_only_gained_or_only_lost_values_is_an_improvement_or_a_regression() {
    let mut d = ShadowDiff::new(vec![]);
    let before =
        json!({ "jobs": [{ "title": null, "org": "Acme" }, { "title": null, "org": "Globex" }] });
    let after =
        json!({ "jobs": [{ "title": "CTO", "org": "Acme" }, { "title": "VP", "org": "Globex" }] });
    add(&mut d, &before, &after); // a renamed item key, fixed by the candidate
    add(&mut d, &after, &before); // the candidate lost it
    // gained a value, changed another
    add(
        &mut d,
        &before,
        &json!({ "jobs": [{ "title": "CTO", "org": "Initech" }, { "title": "VP", "org": "Globex" }] }),
    );
    // a different length is never an improvement
    add(
        &mut d,
        &before,
        &json!({ "jobs": [{ "title": "CTO", "org": "Acme" }] }),
    );
    assert_eq!(field(&d.snapshot(), "jobs"), &diff(4, 0, 1, 1, 2));
}

#[test]
fn artifact_routing_lists_canonical_paths() {
    assert_eq!(routing(&promoted()), ["email"]);
}

#[test]
fn numbers_compare_by_value_and_snapshot_keeps_first_seen_order_with_index_keys_first() {
    let mut d = ShadowDiff::new(vec![]);
    // An integer and the same float are one number, as in JavaScript.
    add_text(
        &mut d,
        r#"{"b": 1, "10": "x", "a": [2], "2": null}"#,
        r#"{"b": 1.0, "10": "x", "a": [2.0], "2": true}"#,
    );
    let r = d.snapshot();
    // JavaScript objects list array-index keys first, in numeric order, then the rest in insertion order.
    let order: Vec<&str> = r.fields.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(order, ["2", "10", "b", "a"]);
    assert_eq!(field(&r, "b"), &diff(1, 1, 0, 0, 0));
    assert_eq!(field(&r, "a"), &diff(1, 1, 0, 0, 0));
    assert_eq!(field(&r, "2"), &diff(1, 0, 1, 0, 0));
}

#[test]
fn a_path_both_sides_leave_null_is_listed_with_nothing_counted() {
    let mut d = ShadowDiff::new(vec![]);
    add(&mut d, &json!({ "a": null }), &json!({}));
    assert_eq!(
        d.snapshot().fields,
        [("a".to_string(), diff(0, 0, 0, 0, 0))]
    );
}

#[test]
fn gate_options_default_as_in_shadow_ts() {
    let o = GateOptions::default();
    assert_eq!(o.max_change, 0.01);
    assert!(o.known.is_empty() && o.fill_alerts.is_empty());
}

// mergeGate on the three synthetic drifts

#[test]
fn a_routing_field_rename_auto_merges_once_known_entities_confirm_it() {
    // The provider renamed workEmail to email. Promoted now maps null; the candidate reads the new key.
    let responses = bodies(|i| json!({ "email": format!("u{i}@acme.com") }));
    let promoted = promoted();
    let candidate = compile_spec(with(base(), "email", json!("email")), 2);
    let report = soak(&promoted, &candidate, &responses);
    let e = field(&report, "email");
    assert_eq!((e.improvements, e.regressions, e.conflicts), (1000, 0, 0));

    assert_eq!(
        gate(&report),
        result(
            false,
            &["email: 1000 routing improvements with no known-entity check"]
        )
    );
    let ok: Vec<(Out, Out)> = (0..3)
        .map(|i| {
            let c: J = serde_json::from_str(&map(&candidate, &responses[i])).unwrap();
            known(&c, &json!({ "email": format!("u{i}@acme.com") }))
        })
        .collect();
    let opts = GateOptions {
        known: ok,
        ..GateOptions::default()
    };
    assert_eq!(merge_gate(&report, &opts), result(true, &[]));

    // A candidate that picks a personal address passes the validator but not the known entities.
    let wrong = GateOptions {
        known: vec![known(
            &json!({ "email": "ada@gmail.com" }),
            &json!({ "email": "u0@acme.com" }),
        )],
        ..GateOptions::default()
    };
    assert_eq!(
        merge_gate(&report, &wrong),
        result(
            false,
            &[r#"known entity 0: email is "ada@gmail.com", expected "u0@acme.com""#]
        )
    );
}

#[test]
fn a_non_routing_field_that_changes_meaning_escalates_to_a_person() {
    // Re-inference maps title to the department key: every value still fills, and every value differs.
    let report = soak(
        &promoted(),
        &compile_spec(with(base(), "title", json!("dept")), 2),
        &bodies(|_| json!({})),
    );
    assert_eq!(
        gate(&report),
        result(
            false,
            &["title: 0 regressions and 1000 conflicts in 1000 compared"]
        )
    );
}

#[test]
fn a_candidate_that_drops_a_rarely_filled_field_doesnt_auto_merge() {
    let mut rest = base();
    rest.remove("patent_count");
    let report = soak(&promoted(), &compile_spec(rest, 2), &bodies(|_| json!({})));
    // Five responses had a value; the candidate lost all five. Among 1,000 responses that's 0.5%.
    let p = field(&report, "patent_count");
    assert_eq!((p.compared, p.regressions), (5, 5));
    assert_eq!(
        gate(&report).reasons,
        ["patent_count: 5 regressions and 0 conflicts in 5 compared"]
    );
}

#[test]
fn an_empty_report_never_merges_and_a_known_entity_has_to_name_the_routing_field_it_vouches_for() {
    let promoted = promoted();
    assert_eq!(
        gate(&ShadowDiff::new(routing(&promoted)).snapshot()),
        result(false, &["no responses compared"])
    );
    let candidate = compile_spec(with(base(), "email", json!("email")), 2);
    let report = soak(
        &promoted,
        &candidate,
        &bodies(|i| json!({ "email": format!("u{i}@acme.com") })),
    );
    let silent = GateOptions {
        known: vec![
            known(&json!({}), &json!({})),
            known(&json!({ "title": "x" }), &json!({ "title": "x" })),
        ],
        ..GateOptions::default()
    };
    assert_eq!(
        merge_gate(&report, &silent).reasons,
        ["email: 1000 routing improvements with no known-entity check"]
    );
}

#[test]
fn a_clean_candidate_merges_and_a_fill_alert_on_it_blocks_the_merge() {
    let report = soak(
        &promoted(),
        &compile_spec(base(), 2),
        &bodies(|_| json!({ "workEmail": "a@acme.com" })),
    );
    assert_eq!(gate(&report), result(true, &[]));
    let alert = FieldAlert {
        field: "title".into(),
        kind: AlertKind::Fill,
        rate: 0.2,
        baseline: 0.9,
        responses: 1000,
    };
    let opts = GateOptions {
        fill_alerts: vec![alert],
        ..GateOptions::default()
    };
    assert_eq!(
        merge_gate(&report, &opts).reasons,
        ["title: fill alert on the candidate (0.200 vs 0.900)"]
    );
}

#[test]
fn routing_regressions_and_conflicts_block_the_merge_and_max_change_is_inclusive() {
    let report = ShadowReport {
        responses: 100,
        routing: vec!["email".into()],
        fields: vec![
            ("email".into(), diff(100, 90, 0, 7, 3)),
            ("title".into(), diff(100, 98, 0, 1, 1)),
            ("seniority".into(), diff(100, 97, 0, 2, 1)),
        ],
    };
    assert_eq!(
        merge_gate(
            &report,
            &GateOptions {
                max_change: 0.02,
                ..GateOptions::default()
            }
        ),
        result(
            false,
            &[
                "email: 7 routing regressions",
                "email: 3 routing conflicts",
                "seniority: 2 regressions and 1 conflicts in 100 compared",
            ]
        )
    );
}

#[test]
fn a_known_entity_reason_writes_both_values_as_json_and_alerts_round_like_to_fixed() {
    let report = ShadowReport {
        responses: 1,
        routing: vec!["email".into(), "company.domain".into()],
        fields: vec![],
    };
    let opts = GateOptions {
        known: vec![known(
            &json!({ "email": null, "company": { "domain": ["a\"b"] } }),
            &json!({ "email": "x@y.co", "company": { "domain": ["a\"b"] } }),
        )],
        fill_alerts: vec![FieldAlert {
            field: "jobs[].title".into(),
            kind: AlertKind::Errors,
            rate: 0.0625,
            baseline: 1.0625,
            responses: 400,
        }],
        ..GateOptions::default()
    };
    // toFixed rounds an exact tie up, where Rust's `{:.3}` rounds it to even.
    assert_eq!(
        merge_gate(&report, &opts),
        result(
            false,
            &[
                r#"known entity 0: email is null, expected "x@y.co""#,
                "jobs[].title: errors alert on the candidate (0.063 vs 1.063)",
            ]
        )
    );
}
