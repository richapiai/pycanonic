// Computed sources in a `from` list (R52): the plan, its printing, the routing refusal and stats.

use canonic_core::{Artifact, compile};
use serde_json::{Value as J, json};

fn spec(fields: J) -> J {
    json!({"language": 2, "provider": "acme", "endpoint": "items", "version": 1,
           "status": "promoted", "fields": fields})
}

fn schema() -> J {
    json!({"type": "object", "properties": {"last": {"type": "boolean"}, "n": {"type": "integer"}}})
}

fn last() -> J {
    json!({"from": [{"from": [{"from": "$input.page", "transform": {"add": 1}}, "$input.size"],
                     "transform": "mul"}, "total"], "transform": "gte"})
}

#[test]
fn plan_is_format_2_and_prints_readably() {
    let plan = compile(&spec(json!({"last": last()})), &schema(), &[]).unwrap();
    assert_eq!(plan.format.0, 2);
    assert_eq!(
        plan.source(),
        "last <- from((from(($input.page | add 1), $input.size) | mul), total) | gte\nn <- null\n"
    );
}

#[test]
fn plan_round_trips_through_json() {
    let plan = compile(&spec(json!({"last": last()})), &schema(), &[]).unwrap();
    let back: canonic_core::Plan =
        serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
    assert_eq!(back, plan);
}

#[test]
fn a_list_of_paths_stays_format_1() {
    let mut s = spec(json!({"n": {"from": ["a", "b"], "transform": "join"}}));
    s.as_object_mut().unwrap().remove("language");
    let plan = compile(&s, &schema(), &[]).unwrap();
    assert_eq!(plan.format.0, 1);
}

#[test]
fn routing_fields_refuse_a_computed_source() {
    let schema = json!({"type": "object", "properties": {
        "email": {"type": "string", "semantic": "email", "routing": true}}});
    let s = spec(
        json!({"email": {"from": [{"from": "a", "transform": "trim"}, "b"], "transform": "join"}}),
    );
    let e = compile(&s, &schema, &[]).unwrap_err().to_string();
    assert!(
        e.ends_with(r#"fields["email"]: routing fields can't read a computed source"#),
        "{e}"
    );
}

#[test]
fn a_missing_inner_source_is_one_error_on_the_outer_field() {
    let plan = compile(&spec(json!({"last": last()})), &schema(), &[]).unwrap();
    let a = Artifact::new(plan, Default::default()).unwrap();
    let ctx = json!({"page": 4, "size": 10});
    let out = a.map_json(br#"{"total": 45}"#, Some(&ctx)).unwrap();
    assert_eq!(out, br#"{"last":true,"n":null}"#);
    a.map_json(br#"{"total": 45}"#, None).unwrap();
    let snap = a.stats.snapshot();
    let f = snap.fields.iter().find(|f| f.0 == "last").unwrap();
    assert_eq!((f.1.fill, f.1.errors), (1, 1), "{snap:?}");
}
