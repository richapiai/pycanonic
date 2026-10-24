// The `language` key and plan format 2. A spec without `language` is language 1, and a plan with no
// language 2 rule still writes format 1.

use canonic_core::{PLAN_FORMAT, Plan, compile, parse_spec, validate_spec};
use proptest::prelude::*;
use serde_json::{Value as J, json};

fn schema() -> J {
    json!({"type": "object", "properties": {"name": {"type": "string"}}})
}

fn spec_without_language() -> J {
    json!({"provider": "acme", "endpoint": "people", "version": 1, "status": "promoted",
           "fields": {"name": "full_name"}})
}

fn spec_with_language(v: J) -> J {
    let mut s = spec_without_language();
    s["language"] = v;
    s
}

fn lang1_spec() -> J {
    spec_with_language(json!(1))
}

fn parse(spec: J) -> canonic_core::Spec {
    parse_spec(&spec).unwrap()
}

fn assert_issue(spec: J, want: &str) {
    let issues: Vec<String> = validate_spec(&spec)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(issues, [want]);
}

fn plan_json(spec: J) -> J {
    serde_json::to_value(compile(&spec, &schema(), &[]).unwrap()).unwrap()
}

#[test]
fn language_missing_is_1() {
    assert_eq!(parse(spec_without_language()).language, 1);
}

#[test]
fn language_1_and_2_parse() {
    assert_eq!(parse(lang1_spec()).language, 1);
    assert_eq!(parse(spec_with_language(json!(2))).language, 2);
}

#[test]
fn language_3_is_rejected() {
    assert_issue(
        spec_with_language(json!(3)),
        "spec.language: must be 1 or 2",
    );
    assert_issue(
        spec_with_language(json!("2")),
        "spec.language: must be 1 or 2",
    );
    assert_issue(
        spec_with_language(json!(2.5)),
        "spec.language: must be 1 or 2",
    );
}

#[test]
fn plain_spec_writes_format_1() {
    assert_eq!(plan_json(lang1_spec())["format"], 1);
    // Language 2 alone does not need format 2. Only a language 2 rule does.
    assert_eq!(plan_json(spec_with_language(json!(2)))["format"], 1);
}

#[test]
fn format_1_and_2_load_format_3_fails() {
    assert_eq!(PLAN_FORMAT, 2);
    let mut plan = plan_json(lang1_spec());
    for f in [1, 2] {
        plan["format"] = json!(f);
        let loaded: Plan = serde_json::from_value(plan.clone()).unwrap();
        assert_eq!(serde_json::to_value(&loaded).unwrap()["format"], f);
    }
    plan["format"] = json!(3);
    let err = serde_json::from_value::<Plan>(plan)
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        "plan format 3 is not supported. This build reads format 1 or 2. Rebuild the plan."
    );
}

#[test]
fn const_writes_format_2_and_loads_back() {
    let mut spec = spec_with_language(json!(2));
    spec["fields"]["name"] = json!({"const": "Ada"});
    let plan = plan_json(spec);
    assert_eq!(plan["format"], 2);
    let loaded: Plan = serde_json::from_value(plan.clone()).unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap(), plan);
    let a = canonic_core::Artifact::new(loaded, Default::default()).unwrap();
    assert_eq!(a.map_json(b"{}", None).unwrap(), br#"{"name":"Ada"}"#);
}

fn compile_err(fields: J, schema: J) -> String {
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = fields;
    compile(&spec, &schema, &[]).unwrap_err().message
}

#[test]
fn const_compile_errors() {
    let obj = json!({"type": "object", "properties": {
        "meta": {"type": "object", "properties": {"v": {"type": "integer"}}},
        "tags": {"type": "array", "items": {"type": "string"}},
        "name": {"type": "string"}}});
    assert_eq!(
        compile_err(
            json!({"tags": {"const": []}, "tags[]": {"from": "x"}}),
            obj.clone()
        ),
        r#"acme people v1: fields["tags[]"]: canonical field is mapped twice"#
    );
    assert_eq!(
        compile_err(json!({"name": "a", "[\"name\"]": "b"}), obj.clone()),
        r#"acme people v1: fields["[\"name\"]"]: canonical field is mapped twice"#
    );
    assert_eq!(
        compile_err(json!({"meta": {"const": {}}, "meta.v": "x"}), obj.clone()),
        r#"acme people v1: fields["meta.v"]: inside a const field"#
    );
    assert_eq!(
        compile_err(json!({"name[]": {"const": []}}), obj.clone()),
        r#"acme people v1: fields["name[]"]: canonical field is not an array"#
    );
    assert_eq!(
        compile_err(json!({"name": {"const": 1, "from": "a"}}), obj),
        r#"invalid spec: spec.fields["name"].from: unknown property"#
    );
}

#[test]
fn const_inside_an_item_fills_per_element() {
    let schema = json!({"type": "object", "properties": {"jobs": {"type": "array", "items": {
        "type": "object", "properties": {"kind": {"type": "string"}}}}}});
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"jobs[]": {"from": "roles", "item": {"kind": {"const": "JOB"}}}});
    let plan = compile(&spec, &schema, &[]).unwrap();
    assert_eq!(
        plan.source(),
        "jobs[] <- roles[]\njobs[].kind <- const \"JOB\"\n"
    );
    let a = canonic_core::Artifact::new(plan, Default::default()).unwrap();
    assert_eq!(
        a.map_json(br#"{"roles": [1, 2]}"#, None).unwrap(),
        br#"{"jobs":[{"kind":"JOB"},{"kind":"JOB"}]}"#
    );
    let kind = &a.stats.snapshot().fields[1];
    assert_eq!(kind.0, "jobs[].kind");
    assert_eq!((kind.1.fill, kind.1.total), (2, Some(2)));
}

#[test]
fn arrays_of_arrays_are_copied_whole_only() {
    let schema = json!({"type": "object", "properties": {"grid": {"type": "array", "items": {
        "type": "array", "items": {"type": "object", "properties": {"x": {"type": "integer"}}}}}}});
    assert_eq!(
        compile_err(
            json!({"grid[]": {"from": "rows", "item": {"x": "x"}}}),
            schema
        ),
        r#"acme people v1: fields["grid[]"]: arrays of arrays can only be copied whole"#
    );
}

#[test]
fn language_2_field_keys_use_the_path_grammar() {
    let bad = |k: &str| {
        let mut spec = spec_with_language(json!(2));
        spec["fields"] = json!({k: {"from": "x"}});
        validate_spec(&spec)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    for k in ["a[0]", "$", "$.a", "a..b", "[\"a\"]x"] {
        assert_eq!(
            bad(k),
            [format!(
                "spec.fields[{}]: canonical field must be a path of keys, optionally ending in []",
                J::from(k)
            )],
            "{k}"
        );
    }
    for k in [
        "[\"first-name\"]",
        "meta[\"@type\"]",
        "[\"@graph\"][]",
        "a.b",
        // The array root.
        "[]",
    ] {
        assert!(bad(k).is_empty(), "{k}");
    }
}

fn tags_schema(empty: Option<&str>) -> J {
    let mut tags = json!({"type": "array", "items": {"type": "string"}});
    if let Some(e) = empty {
        tags["x-empty"] = json!(e);
    }
    json!({"type": "object", "properties": {"tags": tags}})
}

#[test]
fn empty_options_keep_one_error_on_a_non_array() {
    // The last plan is language 1 behavior (the spec undoes the schema), so it stays format 1.
    for (rule, empty, format) in [
        (json!({"from": "labels", "missing": "[]"}), None, 2),
        (json!({"from": "labels", "empty": "null"}), None, 2),
        (json!({"from": "labels"}), Some("[]"), 2),
        (json!({"from": "labels", "missing": "null"}), Some("[]"), 1),
    ] {
        let mut spec = spec_with_language(json!(2));
        spec["fields"] = json!({ "tags[]": rule });
        let plan = compile(&spec, &tags_schema(empty), &[]).unwrap();
        let text = serde_json::to_value(&plan).unwrap();
        assert_eq!(text["format"], format, "{rule}");
        assert_eq!(serde_json::from_value::<Plan>(text).unwrap(), plan);
        let a = canonic_core::Artifact::new(plan, Default::default()).unwrap();
        assert_eq!(
            a.map_json(br#"{"labels": "x"}"#, None).unwrap(),
            br#"{"tags":null}"#
        );
        let tags = &a.stats.snapshot().fields[0];
        assert_eq!((tags.1.fill, tags.1.errors), (0, 1), "{rule}");
    }
}

#[test]
fn empty_options_need_language_2_and_a_known_value() {
    let mut spec = lang1_spec();
    spec["fields"] = json!({"tags[]": {"from": "labels", "missing": "[]"}});
    assert_issue(
        spec,
        r#"spec.fields["tags[]"]: "missing" needs "language": 2"#,
    );
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"tags[]": {"from": "labels", "empty": "{}"}});
    assert_issue(
        spec,
        r#"spec.fields["tags[]"].empty: must be "null" or "[]""#,
    );
}

#[test]
fn x_empty_missing_writes_format_2() {
    let schema = json!({"type": "object", "properties": {
        "name": {"type": "string", "x-empty": "missing"}}});
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"name": "n"});
    let plan = compile(&spec, &schema, &[]).unwrap();
    let text = serde_json::to_value(&plan).unwrap();
    assert_eq!(text["format"], 2);
    let loaded: Plan = serde_json::from_value(text).unwrap();
    let a = canonic_core::Artifact::new(loaded, Default::default()).unwrap();
    assert_eq!(a.map_json(b"{}", None).unwrap(), b"{}");
    assert_eq!(
        a.map_json(br#"{"n": "a"}"#, None).unwrap(),
        br#"{"name":"a"}"#
    );
}

#[test]
fn a_routing_leaf_with_x_empty_missing_still_extracts() {
    let schema = json!({"type": "object", "properties": {"email": {
        "type": "string", "semantic": "email", "routing": true, "x-empty": "missing"}}});
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"email": "e"});
    let plan = compile(&spec, &schema, &[]).unwrap();
    assert_eq!(plan.routing.len(), 1);
    assert_eq!(plan.routing[0].name, "email");
    let a = canonic_core::Artifact::new(plan, Default::default()).unwrap();
    assert_eq!(
        a.extract(br#"{"e": "a@b.co"}"#),
        [Some(canonic_core::Value::Str("a@b.co".into()))]
    );
    assert_eq!(
        a.map_json(br#"{"e": "a@b.co"}"#, None).unwrap(),
        br#"{"email":"a@b.co"}"#
    );
    assert_eq!(a.map_json(br#"{"e": null}"#, None).unwrap(), b"{}");
}

fn hq_schema(x_empty: Option<&str>) -> J {
    let mut hq = json!({"type": "object", "properties": {
        "city": {"type": "string"},
        "email": {"type": "string", "semantic": "email", "routing": true}}});
    if let Some(e) = x_empty {
        hq["x-empty"] = json!(e);
    }
    json!({"type": "object", "properties": {"hq": hq}})
}

fn hq_artifact(rule: Option<J>, x_empty: Option<&str>) -> canonic_core::Artifact {
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"hq.city": "d.city", "hq.email": "d.email"});
    if let Some(r) = rule {
        spec["fields"]["hq"] = r;
    }
    let plan = compile(&spec, &hq_schema(x_empty), &[]).unwrap();
    canonic_core::Artifact::new(plan, Default::default()).unwrap()
}

#[test]
fn a_routing_leaf_in_an_x_empty_missing_object_still_extracts() {
    let a = hq_artifact(None, Some("missing"));
    assert_eq!(a.plan.routing.len(), 1);
    assert_eq!(a.plan.routing[0].name, "hq.email");
    let body = br#"{"d": {"email": "a@b.co"}}"#;
    assert_eq!(
        a.extract(body),
        [Some(canonic_core::Value::Str("a@b.co".into()))]
    );
    assert_eq!(
        a.map_json(body, None).unwrap(),
        br#"{"hq":{"city":null,"email":"a@b.co"}}"#
    );
    assert_eq!(a.map_json(b"{}", None).unwrap(), b"{}");
}

#[test]
fn optional_objects_write_format_2_and_load_back() {
    for rule in [json!({"present": "d"}), json!({"if_empty": "null"})] {
        let a = hq_artifact(Some(rule), None);
        let text = serde_json::to_value(&*a.plan).unwrap();
        assert_eq!(text["format"], 2);
        let loaded: Plan = serde_json::from_value(text.clone()).unwrap();
        assert_eq!(serde_json::to_value(&loaded).unwrap(), text);
        let b = canonic_core::Artifact::new(loaded, Default::default()).unwrap();
        assert_eq!(b.map_json(b"{}", None).unwrap(), br#"{"hq":null}"#);
        assert_eq!(
            b.map_json(br#"{"d": {"city": "x"}}"#, None).unwrap(),
            br#"{"hq":{"city":"x","email":null}}"#
        );
    }
    // A plain object keeps the format 1 layout: its members alone.
    let text = serde_json::to_value(&*hq_artifact(None, None).plan).unwrap();
    assert_eq!(text["format"], 1);
    assert!(text["fields"][0]["value"]["Object"].is_array());
}

#[test]
fn a_failed_present_counts_a_miss_for_each_leaf_and_still_routes() {
    let a = hq_artifact(Some(json!({"present": "d.ok"})), None);
    let body = br#"{"d": {"city": "x", "email": "a@b.co"}}"#;
    assert_eq!(a.map_json(body, None).unwrap(), br#"{"hq":null}"#);
    let fills: Vec<(String, u64)> = (a.stats.snapshot().fields.into_iter())
        .map(|(name, f)| (name, f.fill))
        .collect();
    // Nothing inside the object is mapped. The object rule has no stat slot of its own.
    assert_eq!(fills, [("hq.city".into(), 0), ("hq.email".into(), 0)]);
    // `extract` reads routing fields and ignores `present`.
    assert_eq!(
        a.extract(body),
        [Some(canonic_core::Value::Str("a@b.co".into()))]
    );
}

fn ids_artifact(rule: J) -> canonic_core::Artifact {
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"ids[]": rule});
    let schema = json!({"type": "object", "properties": {
        "ids": {"type": "array", "items": {"type": "integer"}}}});
    canonic_core::Artifact::new(compile(&spec, &schema, &[]).unwrap(), Default::default()).unwrap()
}

#[test]
fn list_ops_write_format_2_and_load_back() {
    let a = ids_artifact(json!({"from": "xs[].ys[]", "wrap": true, "pluck": "id",
        "filter": [{"path": "k", "in": [1, "a"]}, {"path": "id", "present": true}],
        "unique": "id", "sort_by": "id", "order": "desc", "limit": 2}));
    let text = serde_json::to_value(&*a.plan).unwrap();
    assert_eq!(text["format"], 2);
    let loaded: Plan = serde_json::from_value(text.clone()).unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap(), text);
    let b = canonic_core::Artifact::new(loaded, Default::default()).unwrap();
    let body = br#"{"xs": {"ys": [{"id": 1, "k": 1}, {"id": 3, "k": "a"}, {"id": 2, "k": 1},
        {"id": 3, "k": 1}, {"id": 9}]}}"#;
    assert_eq!(b.map_json(body, None).unwrap(), br#"{"ids":[3,2]}"#);
    assert!(a.plan.source().contains("ids[] <- xs[].ys[].id | wrap"));
}

fn page_artifact(spec: J) -> canonic_core::Artifact {
    let schema = json!({"type": "object", "properties": {"page": {"type": "integer"},
        "name": {"type": "string"}}});
    canonic_core::Artifact::new(compile(&spec, &schema, &[]).unwrap(), Default::default()).unwrap()
}

#[test]
fn a_language_1_input_path_reads_the_body() {
    let mut spec = lang1_spec();
    spec["fields"] = json!({"page": "$input.page"});
    let a = page_artifact(spec);
    assert_eq!(serde_json::to_value(&*a.plan).unwrap()["format"], 1);
    let ctx = json!({"page": 3});
    let body = br#"{"$input": {"page": 2}}"#;
    assert_eq!(
        a.map_json(body, Some(&ctx)).unwrap(),
        br#"{"page":2,"name":null}"#
    );
}

#[test]
fn context_and_when_write_format_2_and_load_back() {
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"page": "$input.page"});
    let text = serde_json::to_value(&*page_artifact(spec.clone()).plan).unwrap();
    assert_eq!(
        (text["format"].clone(), text["context"].clone()),
        (json!(2), json!(true))
    );
    spec["fields"]["name"] = json!({"from": "n", "when": {"path": "$input.on", "eq": true},
        "omit": true});
    let a = page_artifact(spec);
    let text = serde_json::to_value(&*a.plan).unwrap();
    let loaded: Plan = serde_json::from_value(text.clone()).unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap(), text);
    assert!(
        a.plan
            .source()
            .contains("name: left out unless $input.on eq true")
    );
    let b = canonic_core::Artifact::new(loaded, Default::default()).unwrap();
    let body = br#"{"n": "Ada"}"#;
    let on = json!({"page": 3, "on": true});
    assert_eq!(
        b.map_json(body, Some(&on)).unwrap(),
        br#"{"page":3,"name":"Ada"}"#
    );
    assert_eq!(b.map_json(body, None).unwrap(), br#"{"page":null}"#);
    // Every way in reads the context the same.
    let node = b.parse(body).unwrap();
    let want = b.map_bytes(body, Some(&on)).unwrap();
    assert_eq!(b.map_doc(&&node, Some(&on)).unwrap(), want);
    let parsed: J = serde_json::from_slice(body).unwrap();
    assert_eq!(
        b.map_with(Some(&&parsed), Some(&&on), &mut canonic_core::RustSink),
        Ok(want)
    );
}

#[test]
fn a_failed_when_counts_a_miss_for_each_field_inside() {
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"name": {"from": "n", "when": {"path": "$input.on", "eq": true}}});
    let a = page_artifact(spec);
    let off = json!({"on": false});
    assert_eq!(
        a.map_json(br#"{"n": "Ada"}"#, Some(&off)).unwrap(),
        br#"{"page":null,"name":null}"#
    );
    assert_eq!(a.stats.snapshot().fields[0].1.fill, 0);
    let on = json!({"on": true});
    a.map_json(br#"{"n": "Ada"}"#, Some(&on)).unwrap();
    assert_eq!(a.stats.snapshot().fields[0].1.fill, 1);
}

/// A small value for `k`: an int, a string, a bool or null.
fn small() -> impl Strategy<Value = J> {
    prop_oneof![
        (0i64..3).prop_map(J::from),
        prop::sample::select(vec!["a", "b"]).prop_map(J::from),
        Just(J::Bool(true)),
        Just(J::Null),
    ]
}

proptest! {
    #[test]
    fn prop_filter_keeps_order(
        ks in prop::collection::vec(prop::option::of(small()), 0..12),
        want in small(),
    ) {
        let xs: Vec<J> = (ks.iter().enumerate())
            .map(|(id, k)| match k {
                Some(k) => json!({"id": id, "k": k}),
                None => json!({"id": id}),
            })
            .collect();
        // [x["id"] for x in xs if x.get("k") == want]
        let expect: Vec<J> = (xs.iter())
            .filter(|x| *x.get("k").unwrap_or(&J::Null) == want)
            .map(|x| x["id"].clone())
            .collect();
        let a = ids_artifact(json!({"from": "xs", "pluck": "id",
            "filter": {"path": "k", "eq": want}}));
        let body = serde_json::to_vec(&json!({"xs": xs})).unwrap();
        let got: J = serde_json::from_slice(&a.map_json(&body, None).unwrap()).unwrap();
        prop_assert_eq!(&got, &json!({"ids": expect}));
    }
}

#[test]
fn reject_writes_format_2_and_loads_back() {
    let mut spec = spec_with_language(json!(2));
    spec["reject"] = json!([{"missing": "full_name"}]);
    let text = plan_json(spec);
    assert_eq!(text["format"], json!(2));
    let loaded: Plan = serde_json::from_value(text.clone()).unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap(), text);
    let a = canonic_core::Artifact::new(loaded, Default::default()).unwrap();
    let r = a.map_bytes(b"{}", None).unwrap_err();
    assert_eq!(r.reason, r#"{"missing":"full_name"}"#);
    assert!(a.map_json(br#"{"full_name": "Ada"}"#, None).is_ok());
    let s = a.stats.snapshot();
    assert_eq!((s.responses, s.rejected), (1, 1));
    // A plan without `reject` writes no key for it.
    assert!(
        plan_json(spec_with_language(json!(2)))
            .get("reject")
            .is_none()
    );
}

fn keys_artifact(keys: Option<&str>) -> canonic_core::Artifact {
    let mut spec = spec_with_language(json!(2));
    if let Some(k) = keys {
        spec["keys"] = json!(k);
    }
    spec["fields"] = json!({"email": "contact.email", "city": "contact.city"});
    let schema = json!({"type": "object", "properties": {
        "email": {"type": "string", "semantic": "email", "routing": true},
        "city": {"type": "string"}}});
    canonic_core::Artifact::new(compile(&spec, &schema, &[]).unwrap(), Default::default()).unwrap()
}

/// A body whose `contact` object has more than `json::INDEXED` keys, so the exact plan's filter
/// keeps a pruned, indexed object there.
fn wide_body(email_key: &str) -> Vec<u8> {
    let filler: String = (0..12).map(|i| format!(r#""k{i}": {i}, "#)).collect();
    format!(r#"{{"Contact": {{{filler}"{email_key}": "a@b.co", "CITY": "Oslo"}}}}"#).into_bytes()
}

#[test]
fn case_insensitive_keys_survive_the_parse_filter() {
    let body = wide_body("eMail");
    let ci = keys_artifact(Some("case_insensitive"));
    assert_eq!(
        canonic_core::filter_for(&ci.plan),
        canonic_core::Filter::all()
    );
    assert_eq!(
        ci.map_json(&body, None).unwrap(),
        br#"{"email":"a@b.co","city":"Oslo"}"#
    );
    let exact = keys_artifact(None);
    assert_ne!(
        canonic_core::filter_for(&exact.plan),
        canonic_core::Filter::all()
    );
    assert_eq!(
        exact.map_json(&body, None).unwrap(),
        br#"{"email":null,"city":null}"#
    );
}

#[test]
fn case_insensitive_extract_and_stream_read_by_case() {
    let a = keys_artifact(Some("case_insensitive"));
    let want = [Some(canonic_core::Value::Str("a@b.co".into()))];
    let body = wide_body("EMAIL");
    assert_eq!(a.extract(&body), want);
    let mut s = a.stream();
    for c in body.chunks(3) {
        assert!(!s.push(c));
    }
    assert_eq!(s.result(), want);
    // Exact wins, as in `map`, and a body that does not parse reads as absent.
    let both = br#"{"contact": {"EMAIL": "x@y.co", "email": "a@b.co"}}"#;
    assert_eq!(a.extract(both), want);
    assert_eq!(a.extract(b"{\"contact\": "), [None]);
    // Without the option, the scanner still matches exact keys only.
    assert_eq!(keys_artifact(None).extract(&body), [None]);
}

#[test]
fn case_insensitive_keys_write_format_2_and_load_back() {
    let a = keys_artifact(Some("case_insensitive"));
    let text = serde_json::to_value(&*a.plan).unwrap();
    assert_eq!(text["format"], 2);
    assert_eq!(text["case_insensitive"], true);
    let loaded: Plan = serde_json::from_value(text.clone()).unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap(), text);
    let b = canonic_core::Artifact::new(loaded, Default::default()).unwrap();
    assert_eq!(
        b.map_json(br#"{"Contact": {"Email": "a@b.co"}}"#, None)
            .unwrap(),
        br#"{"email":"a@b.co","city":null}"#
    );
    // "exact" is the default: format 1, no key in the plan.
    let exact = serde_json::to_value(&*keys_artifact(Some("exact")).plan).unwrap();
    assert_eq!(exact["format"], 1);
    assert!(exact.get("case_insensitive").is_none());
}

#[test]
fn array_root_round_trips_counts_and_extracts_nothing() {
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"[]": {"from": "$", "item": {"name": "n"}}});
    let schema = json!({"type": "array", "items": {"type": "object",
        "properties": {"name": {"type": "string"}}}});
    let plan = compile(&spec, &schema, &[]).unwrap();
    let text = serde_json::to_value(&plan).unwrap();
    assert_eq!(text["format"], 2);
    let loaded: Plan = serde_json::from_value(text.clone()).unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap(), text);
    let a = canonic_core::Artifact::new(loaded, Default::default()).unwrap();
    let body = br#"[{"n": "a"}, {"x": 1}]"#;
    assert_eq!(
        a.map_json(body, None).unwrap(),
        br#"[{"name":"a"},{"name":null}]"#
    );
    assert_eq!(
        a.map_named(body, None).unwrap(),
        canonic_core::Out::List(vec![
            canonic_core::Out::Map(vec![("name".into(), canonic_core::Out::Str("a".into()))]),
            canonic_core::Out::Map(vec![("name".into(), canonic_core::Out::Null)]),
        ])
    );
    let names: Vec<_> = a
        .stats
        .snapshot()
        .fields
        .into_iter()
        .map(|(n, s)| (n, s.fill))
        .collect();
    assert_eq!(names, [("[]".to_string(), 2), ("[].name".to_string(), 2)]);
    assert!(a.plan.routing.is_empty());
    assert!(a.extract(body).is_empty());
}

#[test]
fn array_root_source_and_other_errors() {
    let schema = json!({"type": "array", "items": {"type": "string"}});
    let mut spec = spec_with_language(json!(2));
    spec["fields"] = json!({"[]": {"from": "data", "transform": "trim"}});
    let plan = compile(&spec, &schema, &[]).unwrap();
    assert_eq!(plan.source(), "[] <- data[] | trim\n");
    let when = json!({"[]": {"from": "$", "when": {"path": "a", "present": true}}});
    assert_eq!(
        compile_err(when, schema.clone()),
        r#"acme people v1: fields["[]"]: an array root needs an array rule without when"#
    );
    // An object root has no field "[]".
    assert_eq!(
        compile_err(json!({"[]": {"from": "$"}}), schema_obj()),
        r#"acme people v1: fields["[]"]: only an array root has the field "[]""#
    );
}

fn schema_obj() -> J {
    json!({"type": "object", "properties": {"tags": {"type": "array"}}})
}
