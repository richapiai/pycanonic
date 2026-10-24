// Port of the canonical-schema cases in canonic/packages/canonic/test/validate.test.ts
// ("canonical schema errors", "validateCanonical ...", "names a record cannot safely carry")
// and compile.test.ts. The fixture is vendored from canonic/packages/canonic/test/fixtures.

use canonic_core::{Issue, Kind, Node, Semantic, load_schema, validate_canonical};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn message(schema: Value) -> String {
    let issues = validate_canonical(&schema);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].at, "canonical schema");
    issues[0].message.clone()
}

fn child<'a>(fields: &'a [(String, Node)], name: &str) -> &'a Node {
    &fields.iter().find(|(n, _)| n == name).unwrap().1
}

#[test]
fn a_valid_schema_has_no_issues() {
    let s = json!({ "type": "object", "properties": { "a": { "type": "string" }, "b": { "type": "array" } } });
    assert_eq!(validate_canonical(&s), Vec::<Issue>::new());
}

#[test]
fn a_non_object_root_says_so() {
    assert_eq!(
        validate_canonical(&json!({ "type": "string" })),
        vec![Issue {
            at: "canonical schema".into(),
            message: "root must be an object with properties".into()
        }]
    );
}

#[test]
fn field_names_must_be_identifiers() {
    assert_eq!(
        message(json!({ "properties": { "a b": {} } })),
        r#"field name "a b" is not allowed"#
    );
    assert_eq!(
        message(json!({ "properties": { "Home City": {} } })),
        r#"field name "Home City" is not allowed"#
    );
    assert_eq!(
        message(json!({ "properties": { "9a": {} } })),
        r#"field name "9a" is not allowed"#
    );
    assert_eq!(
        message(json!({ "properties": { "a": { "properties": { "b-c": {} } } } })),
        r#"field name "a.b-c" is not allowed"#
    );
    assert_eq!(
        message(
            json!({ "properties": { "a": { "type": "array", "items": { "properties": { "x\"y": {} } } } } })
        ),
        r#"field name "a[].x\"y" is not allowed"#
    );
}

#[test]
fn names_a_record_cant_safely_carry() {
    let names = [
        "toJSON",
        "constructor",
        "__defineGetter__",
        "__defineSetter__",
        "hasOwnProperty",
        "__lookupGetter__",
        "__lookupSetter__",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "toString",
        "valueOf",
        "__proto__",
        "toLocaleString",
    ];
    for name in names {
        assert_eq!(
            message(json!({ "properties": { name: {} } })),
            format!("field name \"{name}\" is not allowed")
        );
    }
}

#[test]
fn a_schema_that_isnt_an_object_says_so() {
    assert_eq!(message(json!("nope")), "the schema must be a schema object");
    assert_eq!(
        message(json!({ "properties": { "a": 1 } })),
        "a must be a schema object"
    );
    assert_eq!(
        message(json!({ "properties": { "a": { "type": "array", "items": [] } } })),
        "a[] must be a schema object"
    );
}

#[test]
fn arrays_of_arrays_are_rejected() {
    let s = json!({ "properties": { "a": { "type": "array", "items": { "type": "array", "items": {} } } } });
    assert_eq!(
        message(s),
        "a is an array of arrays, which specs can't address"
    );
}

#[test]
fn a_routing_field_inside_an_array_is_rejected() {
    let s = json!({ "properties": { "a": { "type": "array", "items": { "properties": { "e": { "semantic": "email", "routing": true } } } } } });
    assert_eq!(message(s), "routing field a[].e can't be inside an array");
    let s = json!({ "properties": { "a": { "type": ["array", "null"], "items": { "routing": true } } } });
    assert_eq!(message(s), "routing field a[] can't be inside an array");
}

#[test]
fn load_schema_returns_the_same_issue() {
    let err = load_schema(&json!({ "type": "string" }), 1).unwrap_err();
    assert_eq!(
        err.to_string(),
        "canonical schema: root must be an object with properties"
    );
}

#[test]
fn an_array_without_items_holds_any_elements() {
    let s = load_schema(&json!({ "properties": { "a": { "type": "array" } } }), 1).unwrap();
    assert!(matches!(
        child(&s.root, "a").kind,
        Kind::Array { items: None, .. }
    ));
}

#[test]
fn the_fixture_loads_in_property_order() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/canonical.json");
    let v: Value = serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap();
    let s = load_schema(&v, 1).unwrap();
    let names: Vec<&str> = s.root.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "first_name",
            "last_name",
            "full_name",
            "email",
            "phone",
            "linkedin_url",
            "headcount",
            "verified",
            "born",
            "company",
            "experiences",
            "skills"
        ]
    );
    match &child(&s.root, "email").kind {
        Kind::Scalar {
            semantic,
            routing,
            aliases,
        } => {
            assert_eq!(*semantic, Some(Semantic::Email));
            assert!(*routing);
            assert_eq!(aliases, &["work_email"]);
        }
        k => panic!("{k:?}"),
    }
    // An unknown semantic is not a load error (the routing check rejects it later). It carries no semantic.
    assert!(matches!(
        child(&s.root, "full_name").kind,
        Kind::Scalar {
            semantic: None,
            routing: false,
            ..
        }
    ));
    assert!(matches!(
        child(&s.root, "born").kind,
        Kind::Scalar {
            semantic: Some(Semantic::IsoDate),
            ..
        }
    ));
    let Kind::Object {
        children: company, ..
    } = &child(&s.root, "company").kind
    else {
        panic!()
    };
    assert!(matches!(
        child(company, "domain").kind,
        Kind::Scalar {
            semantic: Some(Semantic::Domain),
            routing: true,
            ..
        }
    ));
    let Kind::Array {
        items: Some(exp), ..
    } = &child(&s.root, "experiences").kind
    else {
        panic!()
    };
    let Kind::Object { children: exp, .. } = &exp.kind else {
        panic!()
    };
    assert!(matches!(
        child(exp, "tags").kind,
        Kind::Array { items: Some(_), .. }
    ));
    let Kind::Array {
        items: Some(skill), ..
    } = &child(&s.root, "skills").kind
    else {
        panic!()
    };
    assert!(matches!(
        skill.kind,
        Kind::Scalar {
            semantic: None,
            routing: false,
            ..
        }
    ));
}

#[test]
fn every_semantic_parses_and_aliases_keep_only_strings() {
    let tags = [
        ("email", Semantic::Email),
        ("phone", Semantic::Phone),
        ("domain", Semantic::Domain),
        ("linkedin_url", Semantic::LinkedinUrl),
        ("url", Semantic::Url),
        ("iso_date", Semantic::IsoDate),
        ("money", Semantic::Money),
    ];
    for (tag, want) in tags {
        let s = load_schema(
            &json!({ "properties": { "x": { "semantic": tag, "aliases": ["a", 1, "b"] } } }),
            1,
        )
        .unwrap();
        match &child(&s.root, "x").kind {
            Kind::Scalar {
                semantic, aliases, ..
            } => {
                assert_eq!(*semantic, Some(want));
                assert_eq!(aliases, &["a", "b"]);
            }
            k => panic!("{k:?}"),
        }
    }
}

#[test]
fn x_empty_parses_in_language_2() {
    use canonic_core::Empty;
    for (tag, want) in [
        ("null", Empty::Null),
        ("[]", Empty::List),
        ("{}", Empty::Object),
        ("missing", Empty::Missing),
    ] {
        let x = match tag {
            "{}" => json!({ "properties": { "a": {} }, "x-empty": tag }),
            _ => json!({ "type": "array", "x-empty": tag }),
        };
        let s = load_schema(&json!({ "properties": { "x": x } }), 2).unwrap();
        assert_eq!(child(&s.root, "x").empty, Some(want));
        // Language 1 ignores it.
        let s = load_schema(&json!({ "properties": { "x": { "x-empty": tag } } }), 1).unwrap();
        assert_eq!(child(&s.root, "x").empty, None);
    }
    let err =
        load_schema(&json!({ "properties": { "x": { "x-empty": "maybe" } } }), 2).unwrap_err();
    assert_eq!(
        err.to_string(),
        "canonical schema: x: x-empty must be one of null, [], {}, missing"
    );
}

#[test]
fn language_2_schemas_take_any_name_open_maps_and_nested_arrays() {
    let s = load_schema(
        &json!({ "properties": {
            "first-name": {},
            "toString": {},
            "attrs": { "type": "object", "x-open": true },
            "grid": { "type": "array", "items": { "type": "array" } }
        } }),
        2,
    )
    .unwrap();
    assert!(matches!(
        child(&s.root, "first-name").kind,
        Kind::Scalar { .. }
    ));
    assert!(matches!(
        child(&s.root, "toString").kind,
        Kind::Scalar { .. }
    ));
    assert!(matches!(
        child(&s.root, "attrs").kind,
        Kind::Object { open: true, .. }
    ));
    assert!(matches!(
        child(&s.root, "grid").kind,
        Kind::Array { nested: true, .. }
    ));
}

#[test]
fn x_empty_and_x_open_need_the_right_node_kind() {
    let err = |node: serde_json::Value| {
        load_schema(&json!({ "properties": { "x": node } }), 2)
            .unwrap_err()
            .to_string()
    };
    let leaf = json!({ "type": "string" });
    let array = json!({ "type": "array", "items": { "type": "string" } });
    let object = json!({ "type": "object", "properties": { "a": { "type": "string" } } });
    let with = |n: &serde_json::Value, k: &str, v: serde_json::Value| {
        let mut n = n.clone();
        n[k] = v;
        n
    };
    for (node, tag, want) in [
        (&leaf, "[]", "an array node"),
        (&leaf, "{}", "an object node"),
        (&array, "{}", "an object node"),
        (&object, "[]", "an array node"),
    ] {
        assert_eq!(
            err(with(node, "x-empty", json!(tag))),
            format!("canonical schema: x: x-empty \"{tag}\" needs {want}")
        );
    }
    for node in [&leaf, &array] {
        assert_eq!(
            err(with(node, "x-open", json!(true))),
            "canonical schema: x: x-open needs an object node"
        );
    }
    for (node, tags) in [
        (&leaf, ["null", "missing"].as_slice()),
        (&array, &["null", "[]", "missing"]),
        (&object, &["null", "{}", "missing"]),
    ] {
        for tag in tags {
            load_schema(
                &json!({ "properties": { "x": with(node, "x-empty", json!(tag)) } }),
                2,
            )
            .unwrap();
        }
    }
}
