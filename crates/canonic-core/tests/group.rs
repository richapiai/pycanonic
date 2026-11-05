// `groups` and `aggregate` over parsed documents, the errors an aggregate counts, and a property
// test: the groups of a consecutive grouping, joined, give back the input in order.

use canonic_core::doc::{KeyPath, KeyStep};
use canonic_core::group::{Agg, AggOp, aggregate, groups};
use canonic_core::json::Node;
use canonic_core::{Artifact, Filter, Key, Value, compile, parse};
use proptest::prelude::*;
use serde_json::{Value as J, json};

fn key(k: &str) -> KeyPath {
    vec![KeyStep::Key(Key::new(k.to_string(), 0))]
}

/// The elements of a parsed array.
fn elems<'a>(n: &'a Node<'a>) -> Vec<&'a Node<'a>> {
    match n {
        Node::Arr(a) => a.iter().collect(),
        _ => panic!("not an array"),
    }
}

/// Each group as the `id`s of its elements.
fn ids(gs: &[Vec<&Node>]) -> Vec<Vec<i64>> {
    (gs.iter())
        .map(|g| {
            (g.iter())
                .map(
                    |e| match canonic_core::Doc::get(e, &Key::new("id".into(), 0)) {
                        Some(Node::Num(n)) => n.parse().unwrap(),
                        n => panic!("no id: {n:?}"),
                    },
                )
                .collect()
        })
        .collect()
}

#[test]
fn groups_consecutive_and_not() {
    let all = Filter::all();
    let body = br#"[{"id": 1, "k": "A"}, {"id": 2, "k": "A"}, {"id": 3, "k": "B"},
        {"id": 4, "k": "A"}, {"id": 5}, {"id": 6, "k": null}, {"id": 7}]"#;
    let doc = parse(body, &all).unwrap();
    let k = key("k");
    let runs = groups(elems(&doc), std::slice::from_ref(&k), true);
    assert_eq!(
        ids(&runs),
        [vec![1, 2], vec![3], vec![4], vec![5], vec![6], vec![7]]
    );
    let sets = groups(elems(&doc), std::slice::from_ref(&k), false);
    assert_eq!(
        ids(&sets),
        [vec![1, 2, 4], vec![3], vec![5], vec![6], vec![7]]
    );
    assert!(groups(Vec::<&Node>::new(), std::slice::from_ref(&k), true).is_empty());
}

#[test]
fn group_keys_compare_numbers_by_value_and_ignore_key_order() {
    let all = Filter::all();
    let body = br#"[{"id": 1, "k": {"a": 1, "b": [2.0]}}, {"id": 2, "k": {"b": [2], "a": 1.0}},
        {"id": 3, "k": 1}, {"id": 4, "k": 1.0}, {"id": 5, "k": "1"}]"#;
    let doc = parse(body, &all).unwrap();
    assert_eq!(
        ids(&groups(elems(&doc), &[key("k")], true)),
        [vec![1, 2], vec![3, 4], vec![5]]
    );
}

/// `aggregate` over the elements of `body` (a JSON array; `null` elements are `None`).
fn agg(op: AggOp, body: &str, propagate: bool) -> Option<J> {
    let all = Filter::all();
    let doc = parse(body.as_bytes(), &all).unwrap();
    let values: Vec<Option<&Node>> = (elems(&doc).into_iter())
        .map(|e| (!matches!(e, Node::Null)).then_some(e))
        .collect();
    aggregate(op, values, propagate, ", ").map(|a| match a {
        // An input value as it is: a scalar as JSON, a container by its kind.
        Agg::Doc(d) => match d {
            Node::Str(s) => J::String(s.to_string()),
            Node::Num(n) => serde_json::from_str(n).unwrap(),
            Node::Bool(b) => J::Bool(*b),
            Node::Obj(_) => J::String("object".into()),
            n => panic!("{n:?}"),
        },
        Agg::Value(v) => match v {
            Value::Null => J::Null,
            Value::Bool(b) => J::Bool(b),
            Value::Int(i) => json!(i),
            Value::Float(f) => json!(f),
            Value::Str(s) => J::String(s.into_owned()),
            Value::BigInt(s) => J::String(s.into_owned()),
            Value::Container => panic!("container value"),
        },
    })
}

#[test]
fn aggregate_ops() {
    use AggOp::*;
    let some = |v: J| Some(v);
    assert_eq!(agg(First, r#"[null, "a", "b"]"#, false), some(json!("a")));
    assert_eq!(agg(First, r#"[null, "a", "b"]"#, true), some(J::Null));
    assert_eq!(
        agg(Last, r#"["a", {"x": 1}, null]"#, false),
        some(json!("object"))
    );
    assert_eq!(agg(Count, r#"[1, null, "a"]"#, false), some(json!(2)));
    assert_eq!(agg(Count, r#"[1, null, "a"]"#, true), some(J::Null));
    assert_eq!(agg(Count, "[]", false), some(json!(0)));
    assert_eq!(agg(Sum, "[1, 2.5]", false), some(json!(3.5)));
    assert_eq!(agg(Sum, "[1, 2, null]", false), some(json!(3)));
    assert_eq!(agg(Sum, "[]", false), some(json!(0)));
    assert_eq!(
        agg(Sum, "[9223372036854775807, 1]", false),
        some(json!(9223372036854775808.0))
    );
    assert_eq!(agg(Sum, r#"[1, "2"]"#, false), None);
    assert_eq!(agg(Min, "[3, 1.5, 2]", false), some(json!(1.5)));
    assert_eq!(agg(Max, "[3, 1.5, 2]", false), some(json!(3)));
    assert_eq!(
        agg(Max, r#"["2020-01", "2021-12", "2021-02"]"#, false),
        some(json!("2021-12"))
    );
    assert_eq!(agg(Min, r#"["b", "B", "é"]"#, false), some(json!("B")));
    assert_eq!(agg(Max, r#"["a", null]"#, true), some(J::Null));
    assert_eq!(agg(Max, "[]", false), some(J::Null));
    assert_eq!(agg(Min, r#"[1, "a"]"#, false), None);
    assert_eq!(agg(Min, "[true, false]", false), None);
    assert_eq!(agg(Any, "[false, true]", false), some(json!(true)));
    assert_eq!(agg(Any, "[]", false), some(json!(false)));
    assert_eq!(agg(All, "[true, null, false]", false), some(json!(false)));
    assert_eq!(agg(All, "[true, null]", false), some(json!(true)));
    assert_eq!(agg(All, "[]", false), some(json!(true)));
    assert_eq!(agg(Any, "[1]", false), None);
    assert_eq!(
        agg(Join, r#"["a", 2, "", "b"]"#, false),
        some(json!("a, 2, b"))
    );
    assert_eq!(agg(Join, "[]", false), some(J::Null));
}

/// A plan with the A5 rules, the counts it keeps, a format 2 round trip and its source lines.
#[test]
fn aggregates_count_errors_and_round_trip() {
    let schema = json!({"type": "object", "properties": {
        "low": {"type": "number"},
        "jobs": {"type": "array", "items": {"type": "object", "properties": {
            "n": {"type": "number"}, "end": {"type": "string"}}}}}});
    let spec = json!({"language": 2, "provider": "acme", "endpoint": "people", "version": 1,
        "status": "promoted", "fields": {
            "low": {"aggregate": "min", "from": "xs[].v"},
            "jobs[]": {"from": "xs", "group_by": "k", "consecutive": false, "item": {
                "n": {"aggregate": "count", "from": "$group"},
                "end": {"aggregate": "max", "from": "$group[].e", "nulls": "propagate"}}}}});
    let plan = compile(&spec, &schema, &[]).unwrap();
    let text = serde_json::to_string(&plan).unwrap();
    assert_eq!(serde_json::from_str::<J>(&text).unwrap()["format"], 2);
    let a = Artifact::new(serde_json::from_str(&text).unwrap(), Default::default()).unwrap();
    let body = br#"{"xs": [{"k": 1, "v": 1, "e": "b"}, {"k": 2, "v": "x"}, {"k": 1, "e": "a"}]}"#;
    assert_eq!(
        a.map_json(body, None).unwrap(),
        br#"{"low":null,"jobs":[{"n":2,"end":"b"},{"n":1,"end":null}]}"#
    );
    let s = a.stats.snapshot();
    let low = &s.fields[0];
    assert_eq!((low.0.as_str(), low.1.fill, low.1.errors), ("low", 0, 1));
    let end = &s.fields[3];
    assert_eq!(
        (end.0.as_str(), end.1.fill, end.1.total),
        ("jobs[].end", 1, Some(2))
    );
    let src = plan.source();
    assert!(src.contains("low <- min(xs[].v)"), "{src}");
    assert!(src.contains("jobs[] <- xs[] | group_by k all"), "{src}");
    assert!(
        src.contains("jobs[].end <- max($group[].e) | nulls propagate"),
        "{src}"
    );
}

proptest! {
    #[test]
    fn prop_consecutive_groups_keep_input_order(
        ks in prop::collection::vec(prop::option::of(0i64..3), 0..16),
    ) {
        let xs: Vec<J> = (ks.iter().enumerate())
            .map(|(id, k)| match k {
                Some(k) => json!({"id": id, "k": k}),
                None => json!({"id": id}),
            })
            .collect();
        let body = serde_json::to_vec(&xs).unwrap();
        let all = Filter::all();
        let doc = parse(&body, &all).unwrap();
        let gs = groups(elems(&doc), &[key("k")], true);
        let joined: Vec<i64> = ids(&gs).concat();
        prop_assert_eq!(joined, (0..xs.len() as i64).collect::<Vec<_>>());
        // No two neighbouring groups share a key, and each group has one key.
        for g in &gs {
            let k = |e: &&Node| canonic_core::Doc::get(e, &Key::new("k".into(), 0)).map(|n| format!("{n:?}"));
            prop_assert!(g.iter().all(|e| k(e) == k(&g[0])));
        }
    }
}

#[test]
fn fallback_keys_take_the_first_present_key() {
    let all = Filter::all();
    let body = br#"[{"id": 1, "a": "x", "b": "y"}, {"id": 2, "b": "x"}, {"id": 3, "a": null, "b": "y"},
        {"id": 4, "a": "", "b": "y"}, {"id": 5, "a": "", "b": "z"}, {"id": 6}, {"id": 7, "a": null}, {"id": 8, "b": "x"}]"#;
    let doc = parse(body, &all).unwrap();
    let ks = [key("a"), key("b")];
    // Keys: x, x, y, "", "", none, none, x.
    let runs = groups(elems(&doc), &ks, true);
    assert_eq!(
        ids(&runs),
        [vec![1, 2], vec![3], vec![4, 5], vec![6], vec![7], vec![8]]
    );
    // Not consecutive: element 8 (b: "x") rejoins elements 1 and 2.
    let sets = groups(elems(&doc), &ks, false);
    assert_eq!(
        ids(&sets),
        [vec![1, 2, 8], vec![3], vec![4, 5], vec![6], vec![7]]
    );
}

/// An aggregate transform: a null result from a non-null one is one error, a null aggregate
/// counts none. The plan holds both options, writes format 2 and prints them.
#[test]
fn aggregate_transform_and_filter_count_errors_and_round_trip() {
    let schema = json!({"type": "object", "properties": {
        "a": {"type": "string"}, "b": {"type": "string"}, "c": {"type": "string"}}});
    let spec = json!({"language": 2, "provider": "acme", "endpoint": "people", "version": 1,
        "status": "promoted", "fields": {
            "a": {"aggregate": "max", "from": "xs[].s", "transform": [{"format_date": "date"}]},
            "b": {"aggregate": "first", "from": "xs[].v", "transform": "to_string"},
            "c": {"aggregate": "first", "from": "xs[].u",
                "filter": {"path": "t", "eq": "doc"}}}});
    let plan = compile(&spec, &schema, &[]).unwrap();
    let text = serde_json::to_string(&plan).unwrap();
    let a = Artifact::new(serde_json::from_str(&text).unwrap(), Default::default()).unwrap();
    let body = br#"{"xs": [{"s": "junk", "t": "x", "u": "n"}, {"t": "doc", "u": "m"}]}"#;
    assert_eq!(
        a.map_json(body, None).unwrap(),
        br#"{"a":null,"b":null,"c":"m"}"#
    );
    let s = a.stats.snapshot();
    let errors: Vec<_> = s.fields.iter().map(|f| f.1.errors).collect();
    assert_eq!(errors, [1, 0, 0]);
    let src = plan.source();
    assert!(src.contains("a <- max(xs[].s) | format_date"), "{src}");
    assert!(src.contains("| filter"), "{src}");
}
