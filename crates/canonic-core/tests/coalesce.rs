// Conditional and array `coalesce` (R56, R57): plan format, printing and round trip.

use canonic_core::{Plan, compile};
use serde_json::{Value as J, json};

fn spec(fields: J) -> J {
    json!({"language": 2, "provider": "acme", "endpoint": "posts", "version": 1,
           "status": "promoted", "fields": fields})
}

fn schema() -> J {
    json!({"type": "object", "properties": {"ref": {"type": "string"},
           "tags": {"type": "array", "items": {"type": "string"}}}})
}

#[test]
fn a_conditional_entry_is_format_2_and_prints_readably() {
    let f = json!({"ref": {"coalesce": [{"from": "share", "when": {"starts_with": "urn:example:post:"}}, "id"]}});
    let plan = compile(&spec(f), &schema(), &[]).unwrap();
    assert_eq!(plan.format.0, 2);
    assert_eq!(
        plan.source(),
        "ref <- coalesce(share when share starts_with \"urn:example:post:\", id)\ntags <- null\n"
    );
    let back: Plan = serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
    assert_eq!(back, plan);
}

#[test]
fn plain_paths_keep_the_format_1_coalesce() {
    let mut s = spec(json!({"ref": {"coalesce": ["a", "b"]}}));
    s.as_object_mut().unwrap().remove("language");
    let plan = compile(&s, &schema(), &[]).unwrap();
    assert_eq!(plan.format.0, 1);
    assert!(plan.source().starts_with("ref <- coalesce(a, b)\n"));
}

#[test]
fn an_array_coalesce_is_format_2_and_prints_readably() {
    let f = json!({"tags[]": {"coalesce": ["new", {"from": "old", "when": {"path": "v", "gte": 2}}], "wrap": true}});
    let plan = compile(&spec(f), &schema(), &[]).unwrap();
    assert_eq!(plan.format.0, 2);
    assert_eq!(
        plan.source(),
        "ref <- null\ntags[] <- coalesce(new, old when v gte 2.0)[] | wrap\n"
    );
    let back: Plan = serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
    assert_eq!(back, plan);
}
