// Port of the semantic error cases: canonic/docs/spec-format.md section Errors, the `semantic` list of
// test/fixtures/invalid.json and the compileSpec cases of validate.test.ts. Plus the plan shape:
// stat names, routing, serde round trip and a source() snapshot of specs/acme.json.

use canonic_core::{Builtin, Field, ListKind, Plan, Semantic, TxRef, compile};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

fn person() -> Value {
    fixture("specs/canonical/person.enrich.json")
}

fn spec(fields: Value) -> Value {
    json!({ "provider": "acme", "endpoint": "person.enrich", "version": 2, "status": "promoted", "fields": fields })
}

fn err(spec: &Value, schema: &Value, custom: &[String]) -> String {
    match compile(spec, schema, custom) {
        Ok(_) => panic!("compiled: {spec}"),
        Err(e) => e.message,
    }
}

fn person_err(fields: Value) -> String {
    err(&spec(fields), &person(), &[])
}

#[test]
fn the_messages_in_spec_format_md() {
    let p = "acme person.enrich v2: ";
    let cases = [
        (
            json!({ "nickname": "n" }),
            r#"fields["nickname"]: canonical schema has no field "nickname""#,
        ),
        (
            json!({ "email": { "from": "e", "transform": "fix_email" } }),
            r#"fields["email"]: unknown transform "fix_email""#,
        ),
        (
            json!({ "first_name": { "from": "n", "transform": { "split_name": "middle" } } }),
            r#"fields["first_name"]: invalid argument for split_name"#,
        ),
        (
            json!({ "skills": "tags" }),
            r#"fields["skills"]: canonical field is an array; add []"#,
        ),
        (
            json!({ "experiences[]": { "from": "jobs" } }),
            r#"fields["experiences[]"]: array of objects; use item"#,
        ),
        (
            json!({ "experiences[]": { "from": "jobs", "item": { "title": { "from": "r", "transform": "nope" } } } }),
            r#"fields["experiences[]"].item["title"]: unknown transform "nope""#,
        ),
    ];
    for (fields, message) in cases {
        assert_eq!(person_err(fields), format!("{p}{message}"));
    }
    let mut s = spec(json!({ "title": "t" }));
    s["anchors"] = json!({ "title": "\"t\":" });
    assert_eq!(
        err(&s, &person(), &[]),
        format!(r#"{p}anchors["title"]: only routing fields take anchors"#)
    );
}

#[test]
fn the_other_kind_errors() {
    assert!(
        person_err(json!({ "company": "c" }))
            .ends_with(r#"fields["company"]: canonical field is an object"#)
    );
    assert!(
        person_err(json!({ "email[]": { "from": "e" } }))
            .ends_with(r#"fields["email[]"]: canonical field is not an array"#)
    );
    assert!(
        person_err(json!({ "skills[]": { "from": "t", "item": { "a": "b" } } }))
            .ends_with(r#"fields["skills[]"]: array of scalars; use pluck, or from alone"#)
    );
    assert!(
        person_err(json!({ "experiences[]": { "from": "t", "pluck": "x" } }))
            .ends_with(r#"fields["experiences[]"]: array of objects; use item"#)
    );
    assert!(
        person_err(json!({ "experiences[]": { "from": "t", "item": { "nope": "x" } } })).ends_with(
            r#"fields["experiences[]"].item["nope"]: canonical schema has no field "nope""#
        )
    );
}

#[test]
fn an_array_with_no_items_takes_no_item_rule() {
    let schema = json!({ "type": "object", "properties": { "a": { "type": "array" } } });
    let s = json!({ "provider": "p", "endpoint": "e", "version": 1, "status": "draft", "fields": { "a[]": { "from": "x", "item": { "b": "c" } } } });
    assert_eq!(
        err(&s, &schema, &[]),
        r#"p e v1: fields["a[]"]: array of scalars; use pluck, or from alone"#
    );
    let s = json!({ "provider": "p", "endpoint": "e", "version": 1, "status": "draft", "fields": { "a[]": { "from": "x" } } });
    let plan = compile(&s, &schema, &[]).unwrap();
    assert!(
        matches!(&plan.fields[0].value, Field::List { kind: ListKind::Plain(tx), .. } if tx.is_empty())
    );
}

#[test]
fn routing_field_rules() {
    assert!(
        person_err(json!({ "email": { "from": "e", "default": "none@acme.com" } }))
            .ends_with(r#"fields["email"]: routing fields can't have a default"#)
    );
    assert!(
        person_err(json!({ "email": "$" }))
            .ends_with(r#"fields["email"]: routing fields can't read the whole response ($)"#)
    );
    assert!(
        person_err(json!({ "email": { "coalesce": ["a", "$"] } }))
            .ends_with(r#"fields["email"]: routing fields can't read the whole response ($)"#)
    );
    // Transform errors come before the routing checks, as in resolve.ts.
    assert!(
        person_err(json!({ "email": { "from": "$", "transform": "nope" } }))
            .ends_with(r#"fields["email"]: unknown transform "nope""#)
    );
    let schema = json!({ "type": "object", "properties": { "title": { "type": "string", "semantic": "job_title", "routing": true } } });
    let s = json!({ "provider": "p", "endpoint": "e", "version": 1, "status": "draft", "fields": { "title": "t" } });
    assert_eq!(
        err(&s, &schema, &[]),
        r#"p e v1: fields["title"]: routing field needs a semantic with a validator (email, phone, domain, linkedin_url, url, iso_date, money)"#
    );
}

#[test]
fn anchor_rules() {
    let with = |fields: Value, anchors: Value| {
        let mut s = spec(fields);
        s["anchors"] = anchors;
        err(&s, &person(), &[])
    };
    let p = "acme person.enrich v2: ";
    assert_eq!(
        with(json!({ "email": "e" }), json!({ "phone": "\"p\":" })),
        format!(r#"{p}anchors["phone"]: not a field in this spec"#)
    );
    let need = r#"anchored fields need a single from path ending in a key"#;
    assert_eq!(
        with(
            json!({ "email": { "coalesce": ["a", "b"] } }),
            json!({ "email": "\"a\":" })
        ),
        format!(r#"{p}anchors["email"]: {need}"#)
    );
    assert_eq!(
        with(json!({ "email": "a[0]" }), json!({ "email": "\"a\":" })),
        format!(r#"{p}anchors["email"]: {need}"#)
    );
    assert_eq!(
        with(
            json!({ "email": { "from": ["a", "b"] } }),
            json!({ "email": "\"b\":" })
        ),
        format!(r#"{p}anchors["email"]: {need}"#)
    );
    assert_eq!(
        with(
            json!({ "email": "work.workEmail" }),
            json!({ "email": "\"mail\":" })
        ),
        format!(r#"{p}anchors["email"]: token must be "\"workEmail\":""#)
    );
    // Anchors are checked in spec order and resolution stops at the first error.
    assert_eq!(
        with(
            json!({ "email": "e", "title": "t" }),
            json!({ "title": "x", "email": "y" })
        ),
        format!(r#"{p}anchors["title"]: only routing fields take anchors"#)
    );
    let mut s = spec(json!({ "email": "work.workEmail" }));
    s["anchors"] = json!({ "email": "\"workEmail\":" });
    let plan = compile(&s, &person(), &[]).unwrap();
    assert_eq!(plan.routing[0].anchor.as_deref(), Some("\"workEmail\":"));
}

#[test]
fn resolution_stops_at_the_first_error_in_spec_order() {
    assert!(
        person_err(json!({ "zzz": "a", "nickname": "b" }))
            .ends_with(r#"fields["zzz"]: canonical schema has no field "zzz""#)
    );
}

#[test]
fn grammar_and_schema_errors_come_first() {
    let bad = json!({ "provider": "p", "endpoint": "e", "version": 1, "status": "nope", "fields": { "a": "a" } });
    let m = err(&bad, &person(), &[]);
    assert!(
        m.starts_with(
            "invalid spec: spec.status: must be one of draft, shadow, promoted, deprecated"
        ),
        "{m}"
    );
    let s = json!({ "provider": "p", "endpoint": "e", "version": 1, "status": "draft", "fields": { "a": "a" } });
    assert_eq!(
        err(&s, &json!({ "type": "string" }), &[]),
        "canonical schema: root must be an object with properties"
    );
}

#[test]
fn the_invalid_json_semantic_cases() {
    let canonical = fixture("canonical.json");
    let cases = fixture("invalid.json")["semantic"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(cases.len(), 21);
    for c in cases {
        let m = err(&c["spec"], &canonical, &[]);
        let want = c["error"].as_str().unwrap();
        assert!(m.contains(want), "{}: {m} lacks {want}", c["name"]);
    }
}

#[test]
fn prototype_names_are_unknown_transforms() {
    for t in ["toString", "constructor", "hasOwnProperty"] {
        let m = person_err(json!({ "first_name": { "from": "n", "transform": t } }));
        assert!(m.ends_with(&format!(r#"unknown transform "{t}""#)), "{m}");
    }
}

#[test]
fn custom_transforms_become_slots_and_shadow_defaults() {
    let custom = vec!["fix_email".to_string(), "split_name".to_string()];
    let s = spec(json!({
        "email": { "from": "e", "transform": ["trim", "fix_email"] },
        "first_name": { "from": "n", "transform": { "split_name": "middle" } },
        "last_name": { "from": "n", "transform": { "cast": "integer" } }
    }));
    let plan = compile(&s, &person(), &custom).unwrap();
    assert_eq!(plan.custom_names, custom);
    let scalar = |name: &str| match &plan.fields.iter().find(|f| f.name == name).unwrap().value {
        Field::Scalar(s) => s.clone(),
        other => panic!("{other:?}"),
    };
    let email = scalar("email");
    assert_eq!(
        email.tx,
        vec![
            TxRef::Builtin(Builtin::Trim),
            TxRef::Custom {
                index: 0,
                name: "fix_email".into(),
                arg: None
            }
        ]
    );
    assert_eq!(email.validator, Some(Semantic::Email));
    assert_eq!(
        scalar("first_name").tx,
        vec![TxRef::Custom {
            index: 1,
            name: "split_name".into(),
            arg: Some(json!("middle"))
        }]
    );
    assert_eq!(
        scalar("last_name").tx,
        vec![TxRef::Builtin(Builtin::Cast(canonic_core::CastTo::Integer))]
    );
}

fn acme() -> Plan {
    compile(&fixture("specs/acme.json"), &person(), &[]).unwrap()
}

#[test]
fn stat_names_follow_codegen() {
    let plan = acme();
    let names: Vec<&str> = plan.stat_names.iter().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "first_name",
            "last_name",
            "full_name",
            "email",
            "phone",
            "title",
            "seniority",
            "linkedin_url",
            "location.city",
            "location.country",
            "company.name",
            "company.domain",
            "company.headcount",
            "company.industry",
            "company.founded",
            "experiences[]",
            "experiences[].title",
            "experiences[].company",
            "experiences[].start",
            "experiences[].end",
            "skills[]",
            "updated_at",
        ]
    );
    let routing: Vec<(&str, Option<Semantic>)> = plan
        .routing
        .iter()
        .map(|r| (r.name.as_str(), r.scalar.validator))
        .collect();
    assert_eq!(
        routing,
        [
            ("email", Some(Semantic::Email)),
            ("phone", Some(Semantic::Phone)),
            ("company.domain", Some(Semantic::Domain))
        ]
    );
    for (i, n) in plan.names.iter().enumerate() {
        assert_eq!(
            plan.names.iter().position(|m| m == n),
            Some(i),
            "names are interned"
        );
    }
    fn check(plan: &Plan, fields: &[canonic_core::Named<Field>]) {
        for f in fields {
            assert_eq!(plan.names[f.name_idx], f.name);
            match &f.value {
                Field::Object { members: c, .. }
                | Field::List {
                    kind: ListKind::Item(c),
                    ..
                } => check(plan, c),
                _ => {}
            }
        }
    }
    check(&plan, &plan.fields);
}

#[test]
fn unmapped_fields_are_null_in_schema_order() {
    let plan = compile(
        &spec(json!({ "company.domain": "d", "email": "e" })),
        &person(),
        &[],
    )
    .unwrap();
    let top: Vec<&str> = plan.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(top[..4], ["first_name", "last_name", "full_name", "email"]);
    assert_eq!(top.len(), 13);
    assert!(matches!(plan.fields[0].value, Field::Null));
    assert_eq!(plan.stat_names, ["company.domain", "email"]);
    assert_eq!(
        plan.routing
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["email", "company.domain"]
    );
}

#[test]
fn plans_round_trip_through_serde() {
    let custom = vec!["x".to_string()];
    let odd = compile(
        &spec(json!({ "first_name": { "from": "n", "default": 0, "transform": { "x": null } }, "last_name": { "from": "n", "transform": "x" } })),
        &person(),
        &custom,
    )
    .unwrap();
    let globex = compile(&fixture("specs/globex.json"), &person(), &[]).unwrap();
    for plan in [acme(), globex, odd] {
        let text = serde_json::to_string(&plan).unwrap();
        let back: Plan = serde_json::from_str(&text).unwrap();
        assert_eq!(back, plan);
    }
}

#[test]
fn source_snapshot_of_acme() {
    let expected = r#"first_name <- profile.fullName | split_name "first"
last_name <- profile.fullName | split_name "last"
full_name <- profile.fullName
email <- coalesce(contact.workEmail, contact.emails[0].address) | trim | lower | validate email
phone <- contact.mobile | normalize_phone | validate phone
title <- profile.headline | trim
seniority <- profile.level
linkedin_url <- social.li | normalize_linkedin
location.city <- geo.city
location.country <- geo.countryCode
company.name <- org.name
company.domain <- org.website | to_domain | validate domain
company.headcount <- org.size | cast "integer"
company.industry <- org.industry
company.founded <- org.foundedOn | parse_date
experiences[] <- workHistory[]
experiences[].title <- role
experiences[].company <- orgName
experiences[].start <- from | parse_date
experiences[].end <- to | parse_date
skills[] <- tags[].label | lower
updated_at <- _meta.updated | parse_date
"#;
    assert_eq!(acme().source(), expected);
}

#[test]
fn a_language_1_pair_on_one_field_keeps_the_kind_error() {
    let want = r#"acme person.enrich v2: fields["skills"]: canonical field is an array; add []"#;
    assert_eq!(
        person_err(json!({ "skills[]": { "from": "a" }, "skills": "b" })),
        want
    );
    assert_eq!(
        person_err(json!({ "skills": "b", "skills[]": { "from": "a" } })),
        want
    );
}
