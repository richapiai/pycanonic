// `holds` over a parsed document: every condition form, `$input` paths and element-relative paths.

use canonic_core::cond::{Cond, holds};
use canonic_core::doc::{KeyPath, KeyStep};
use canonic_core::{Filter, Key, parse};
use serde_json::json;

fn path(keys: &[&str]) -> KeyPath {
    (keys.iter().enumerate())
        .map(|(i, k)| KeyStep::Key(Key::new(k.to_string(), i as u64)))
        .collect()
}

fn present(keys: &[&str], want: bool) -> Cond<KeyPath> {
    Cond::Present(path(keys), want)
}

#[test]
fn present_reads_the_element_then_the_root_and_input_reads_ctx() {
    let all = Filter::all();
    let root = parse(br#"{"a": {"b": 1}, "n": null}"#, &all).unwrap();
    let element = parse(br#"{"c": 2}"#, &all).unwrap();
    let ctx = parse(br#"{"page": 3}"#, &all).unwrap();
    let (root, element, ctx) = (Some(&&root), Some(&&element), Some(&&ctx));
    assert!(holds(&present(&["a", "b"], true), root, None, None));
    assert!(holds(&present(&["n"], false), root, None, None));
    assert!(holds(&present(&["x"], false), root, None, None));
    // Inside an item map, a path reads the element, not the root.
    assert!(holds(&present(&["c"], true), root, element, None));
    assert!(holds(&present(&["a"], false), root, element, None));
    assert!(holds(
        &present(&["$input", "page"], true),
        root,
        element,
        ctx
    ));
    assert!(holds(
        &present(&["$input", "page"], false),
        root,
        None,
        None
    ));
    let all_of = Cond::All(vec![present(&["a"], true), present(&["x"], true)]);
    assert!(!holds(&all_of, root, None, None));
    assert!(holds(&Cond::All(vec![]), root, None, None));
}

#[test]
fn eq_and_in_compare_json_values() {
    let all = Filter::all();
    let root = parse(
        br#"{"s": "x", "i": 1, "f": 1.5, "b": true, "n": null, "o": {"k": [1, 2]}, "e": ""}"#,
        &all,
    )
    .unwrap();
    let root = Some(&&root);
    let eq = |keys: &[&str], v: serde_json::Value| Cond::Eq(path(keys), v);
    assert!(holds(&eq(&["s"], json!("x")), root, None, None));
    assert!(!holds(&eq(&["s"], json!("y")), root, None, None));
    assert!(holds(&eq(&["i"], json!(1)), root, None, None));
    assert!(holds(&eq(&["i"], json!(1.0)), root, None, None));
    assert!(!holds(&eq(&["i"], json!("1")), root, None, None));
    assert!(!holds(&eq(&["b"], json!(1)), root, None, None));
    assert!(holds(&eq(&["f"], json!(1.5)), root, None, None));
    assert!(holds(&eq(&["b"], json!(true)), root, None, None));
    // Missing and null both equal null.
    assert!(holds(&eq(&["n"], json!(null)), root, None, None));
    assert!(holds(&eq(&["x"], json!(null)), root, None, None));
    assert!(!holds(&eq(&["x"], json!("x")), root, None, None));
    assert!(holds(&eq(&["o"], json!({"k": [1, 2]})), root, None, None));
    assert!(!holds(&eq(&["o"], json!({"k": [2, 1]})), root, None, None));
    let any =
        |keys: &[&str], v: serde_json::Value| Cond::In(path(keys), v.as_array().unwrap().clone());
    assert!(holds(&any(&["s"], json!(["y", "x"])), root, None, None));
    assert!(!holds(&any(&["s"], json!([])), root, None, None));
    assert!(holds(&any(&["x"], json!([1, null])), root, None, None));
}

#[test]
fn missing_and_empty() {
    let all = Filter::all();
    let root = parse(
        br#"{"s": "x", "e": "", "n": null, "a": [], "o": {}, "z": 0, "f": false, "l": [null]}"#,
        &all,
    )
    .unwrap();
    let root = Some(&&root);
    let missing = |k: &str| holds(&Cond::Missing(path(&[k])), root, None, None);
    let empty = |k: &str| holds(&Cond::Empty(path(&[k])), root, None, None);
    assert!(missing("x") && missing("n"));
    assert!(!missing("e") && !missing("a") && !missing("z"));
    for k in ["x", "n", "e", "a", "o"] {
        assert!(empty(k), "{k} is empty");
    }
    for k in ["s", "z", "f", "l"] {
        assert!(!empty(k), "{k} is not empty");
    }
}

#[test]
fn eq_on_containers_compares_numbers_by_value_and_ignores_key_order() {
    let all = Filter::all();
    let root = parse(
        br#"{"a": [1], "f": [1.0], "o": {"a": 1, "b": {"c": 2.0, "d": [3]}}, "s": ["1"]}"#,
        &all,
    )
    .unwrap();
    let root = Some(&&root);
    let eq = |k: &str, v: serde_json::Value| holds(&Cond::Eq(path(&[k]), v), root, None, None);
    assert!(eq("a", json!([1.0])));
    assert!(eq("f", json!([1])));
    assert!(eq("o", json!({"b": {"d": [3.0], "c": 2}, "a": 1.0})));
    assert!(!eq("o", json!({"a": 1, "b": {"c": 2.5, "d": [3]}})));
    assert!(!eq("s", json!([1])));
    assert!(!eq("a", json!([true])));
    let any = Cond::In(
        path(&["o"]),
        vec![json!([1]), json!({"b": {"c": 2, "d": [3]}, "a": 1})],
    );
    assert!(holds(&any, root, None, None));
}

#[test]
fn string_tests_read_only_strings_and_contains_reads_arrays() {
    use canonic_core::cond::{Cmp, Operand};
    let all = Filter::all();
    let root = parse(
        br#"{"s": "abc", "n": null, "i": 5, "a": ["x", 5], "w": 3, "t": "5", "f": 4.5}"#,
        &all,
    )
    .unwrap();
    let root = Some(&&root);
    let h = |c: Cond<KeyPath>| holds(&c, root, None, None);
    let re = |s: &str| serde_json::from_value(json!(s)).unwrap();
    assert!(h(Cond::Matches(path(&["s"]), re("^a.c$"))));
    assert!(!h(Cond::Matches(path(&["i"]), re("5"))));
    assert!(!h(Cond::Matches(path(&["n"]), re(""))));
    assert!(!h(Cond::Matches(path(&["x"]), re(""))));
    assert!(h(Cond::StartsWith(path(&["s"]), "ab".into())));
    assert!(h(Cond::EndsWith(path(&["s"]), "bc".into())));
    assert!(!h(Cond::StartsWith(path(&["i"]), "5".into())));
    assert!(h(Cond::Contains(path(&["s"]), json!("b"))));
    assert!(!h(Cond::Contains(path(&["s"]), json!(5))));
    assert!(h(Cond::Contains(path(&["a"]), json!(5))));
    assert!(h(Cond::Contains(path(&["a"]), json!("x"))));
    assert!(!h(Cond::Contains(path(&["a"]), json!("xy"))));
    let cmp = |k: &str, c, o| h(Cond::Compare(path(&[k]), c, o));
    assert!(cmp("i", Cmp::Gte, Operand::Num(5.0)));
    assert!(!cmp("i", Cmp::Gt, Operand::Num(5.0)));
    assert!(cmp("f", Cmp::Lt, Operand::Num(5.0)));
    assert!(!cmp("t", Cmp::Gte, Operand::Num(1.0)));
    assert!(!cmp("x", Cmp::Lt, Operand::Num(1.0)));
    assert!(cmp("i", Cmp::Gt, Operand::Path(path(&["w"]))));
    assert!(!cmp("i", Cmp::Gt, Operand::Path(path(&["t"]))));
    assert!(!cmp("i", Cmp::Lte, Operand::Path(path(&["x"]))));
    let missing = Cond::Missing(path(&["x"]));
    assert!(!h(Cond::Not(Box::new(missing.clone()))));
    assert!(h(Cond::Any(vec![present(&["x"], true), missing])));
    assert!(!h(Cond::Any(vec![])));
}

#[test]
fn parse_reads_the_new_forms_and_nests_combinators() {
    use canonic_core::cond::{parse, parse_reject, parse_relative, parse_v1};
    use canonic_core::spec::parse_path;
    let c = parse(&json!({"all": [{"not": {"path": "a", "starts_with": "x"}},
        {"any": [{"path": "b", "gte": {"path": "c"}}, {"path": "d", "contains": 1}]}]}))
    .unwrap();
    assert!(matches!(&c, Cond::All(v) if v.len() == 2 && matches!(v[0], Cond::Not(_))));
    let own = parse_path("name").unwrap();
    let c = parse_relative(&json!({"matches": "^a"}), &own).unwrap();
    assert!(matches!(&c, Cond::Matches(p, _) if *p == own));
    let c = parse_relative(&json!({"not": {"lt": 3}}), &own).unwrap();
    assert!(matches!(c, Cond::Not(_)));
    // An explicit path wins over `own`.
    let c = parse_relative(&json!({"path": "z", "eq": 1}), &own).unwrap();
    assert_eq!(c, Cond::Eq(parse_path("z").unwrap(), json!(1)));
    // Without own, a path is required.
    assert!(parse(&json!({"matches": "a"})).is_err());
    // Reject keeps missing and empty, also inside a combinator.
    assert!(parse_reject(&json!({"not": {"missing": "a"}})).is_ok());
    assert!(parse(&json!({"not": {"missing": "a"}})).is_err());
    // Language 1 reads neither the new tests nor the combinators.
    assert!(parse_v1(&json!({"path": "a", "starts_with": "x"})).is_err());
    assert!(parse_v1(&json!({"not": {"path": "a", "eq": 1}})).is_err());
    // Issues carry the location inside a combinator.
    let e = parse(&json!({"all": [{"path": "a", "eq": 1}, {"path": "b", "lt": "5"}]})).unwrap_err();
    assert_eq!(e[0].0, ".all[1].lt");
    let e = parse(&json!({"path": "a", "matches": "("})).unwrap_err();
    assert_eq!(
        e,
        vec![(".matches".into(), "invalid regex: unclosed group".into())]
    );
}
